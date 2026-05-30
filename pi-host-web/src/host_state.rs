use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tsify::Tsify;

use pi_core::{
    AgentMessage, CompactionPlan, ContextProjectionBudget, ContextProjectionReport,
    ContextProjectionState, SessionEntry,
};

use crate::dto::OldSessionState;

const MAX_ARTIFACTS: usize = 1000;

/// Search result for a single artifact match.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct ArtifactSearchResult {
    pub id: String,
    pub snippet: String,
    pub match_count: usize,
}

/// Serializable snapshot of host state for persistence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct PersistData {
    pub entries: Vec<SessionEntry>,
    pub leaf_id: String,
    pub name: String,
    pub projection_state: ContextProjectionState,
    pub artifacts: Vec<(String, String)>,
    pub budget: ContextProjectionBudget,
    pub system_prompt: String,
}

/// Owns all host-side state: entries, projection state, artifacts, and budget.
#[derive(Debug, Clone, PartialEq)]
pub struct HostState {
    pub system_prompt: String,
    pub entries: Vec<SessionEntry>,
    pub leaf_id: String,
    pub name: String,
    pub projection_state: ContextProjectionState,
    pub artifacts: BTreeMap<String, String>,
    pub budget: ContextProjectionBudget,
    /// Compaction plans emitted with `Compact` directives, keyed by compact_up_to entry id.
    pub pending_compaction_plans: Vec<(String, pi_core::CompactionPlan)>,
}

impl HostState {
    /// Initialize with empty state and the given budget.
    pub fn new(system_prompt: String, budget: ContextProjectionBudget) -> Self {
        Self {
            system_prompt,
            entries: Vec::new(),
            leaf_id: String::new(),
            name: String::new(),
            projection_state: ContextProjectionState::default(),
            artifacts: BTreeMap::new(),
            budget,
            pending_compaction_plans: Vec::new(),
        }
    }

    /// Append an entry and update the leaf to point at it.
    pub fn append_entry(&mut self, entry: SessionEntry) {
        self.leaf_id = entry.id.clone();
        self.entries.push(entry);
    }

    /// Run context projection using the current state and budget.
    ///
    /// Updates `self.projection_state` and ensures every reported replacement
    /// has a slot in the artifact store (populated later by `store_artifact`).
    pub fn project(
        &mut self,
        system_prompt: &str,
        messages: &[AgentMessage],
    ) -> (Vec<AgentMessage>, ContextProjectionReport) {
        let input = pi_core::ProjectionInput {
            system_prompt: system_prompt.to_string(),
            messages: messages.to_vec(),
            budget: self.budget.clone(),
            state: self.projection_state.clone(),
        };
        let output = pi_core::project(input);
        self.projection_state = output.updated_state;

        // Ensure every replacement has an artifact slot (host will populate text).
        for replacement in &output.report.replacements {
            self.artifacts
                .entry(replacement.artifact_id.clone())
                .or_default();
        }

        (output.projected_messages, output.report)
    }

    /// Store the full text of an artifact. Evicts the oldest entry when over
    /// the MAX_ARTIFACTS limit.
    pub fn store_artifact(&mut self, id: String, text: String) {
        if self.artifacts.len() >= MAX_ARTIFACTS && !self.artifacts.contains_key(&id) {
            // FIFO eviction: remove the lexicographically smallest key.
            // In practice artifact IDs are sequential, so this behaves as FIFO.
            let first_key = self.artifacts.keys().next().cloned().unwrap();
            self.artifacts.remove(&first_key);
        }
        self.artifacts.insert(id, text);
    }

    /// Read the stored text for an artifact.
    pub fn read_artifact(&self, id: &str) -> Option<&str> {
        self.artifacts.get(id).map(|s| s.as_str())
    }

    /// Simple substring search across all artifact texts.
    pub fn search_artifacts(&self, query: &str) -> Vec<ArtifactSearchResult> {
        let mut results = Vec::new();
        for (id, text) in &self.artifacts {
            let mut match_count = 0;
            let mut snippet = String::new();
            for (idx, _) in text.match_indices(query) {
                match_count += 1;
                if snippet.is_empty() {
                    let start = idx.saturating_sub(40);
                    let end = (idx + query.len() + 40).min(text.len());
                    snippet = text[start..end].to_string();
                }
            }
            if match_count > 0 {
                results.push(ArtifactSearchResult {
                    id: id.clone(),
                    snippet,
                    match_count,
                });
            }
        }
        results
    }

    /// Serialize host state for persistence.
    pub fn get_persist_data(&self) -> PersistData {
        PersistData {
            entries: self.entries.clone(),
            leaf_id: self.leaf_id.clone(),
            name: self.name.clone(),
            projection_state: self.projection_state.clone(),
            artifacts: self.artifacts.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            budget: self.budget.clone(),
            system_prompt: self.system_prompt.clone(),
        }
    }

    /// Restore host state from a persisted snapshot.
    pub fn restore(data: PersistData) -> Self {
        Self {
            system_prompt: data.system_prompt,
            entries: data.entries,
            leaf_id: data.leaf_id,
            name: data.name,
            projection_state: data.projection_state,
            artifacts: data.artifacts.into_iter().collect(),
            budget: data.budget,
            pending_compaction_plans: Vec::new(),
        }
    }

    /// Migrate from old session JSON that had `projection_state` and `artifacts`
    /// nested inside the `SessionState` object.
    pub fn migrate_from_old_session(old: OldSessionState) -> Self {
        Self {
            system_prompt: String::new(),
            entries: old.entries.into_iter().map(|e| e.try_into().unwrap()).collect(),
            leaf_id: old.leaf_id,
            name: old.name,
            projection_state: old.projection_state.unwrap_or_default().try_into().unwrap(),
            artifacts: old.artifacts.into_iter().map(|a| (a.id, a.text)).collect(),
            budget: ContextProjectionBudget::default(),
            pending_compaction_plans: Vec::new(),
        }
    }

    /// Try to deserialize and migrate from old session JSON format.
    pub fn try_migrate_old_session(json: &str) -> Option<Self> {
        let old: OldSessionState = serde_json::from_str(json).ok()?;
        Some(Self::migrate_from_old_session(old))
    }

    /// Plan which entries to compact based on the current budget.
    pub fn plan_compaction(&self) -> Option<CompactionPlan> {
        pi_core::plan_compaction(&self.entries, &self.budget)
    }

    /// Apply a compaction plan and update entries / leaf_id.
    pub fn accept_compaction(&mut self, plan: CompactionPlan, summary: String) {
        let new_entries = pi_core::apply_compaction(self.entries.clone(), plan, summary);
        if let Some(last) = new_entries.last() {
            self.leaf_id = last.id.clone();
        }
        self.entries = new_entries;
    }

    /// Check whether a projection report signals that compaction is needed.
    pub fn detect_needs_compaction(&self, report: &ContextProjectionReport) -> bool {
        report.needs_compaction
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pi_core::{
        AgentMessage, ContextProjectionBudget, ContextProjectionState, EntryKind, SessionEntry,
    };

    fn default_budget() -> ContextProjectionBudget {
        ContextProjectionBudget {
            max_tool_result_chars: 50000,
            max_context_tokens: 100000,
            microcompact_after_turns: 5,
            compaction_threshold: 0.75,
        }
    }

    fn empty_entry(id: &str, parent_id: Option<&str>) -> SessionEntry {
        SessionEntry {
            id: id.to_string(),
            parent_id: parent_id.map(|s| s.to_string()),
            kind: EntryKind::Message {
                message: AgentMessage::user(""),
            },
            timestamp: 0,
        }
    }

    #[test]
    fn host_state_new_default() {
        let state = HostState::new(String::new(), default_budget());
        assert!(state.entries.is_empty());
        assert_eq!(state.leaf_id, "");
        assert_eq!(state.name, "");
        assert_eq!(state.projection_state, ContextProjectionState::default());
        assert!(state.artifacts.is_empty());
    }

    #[test]
    fn host_state_entries_append() {
        let mut state = HostState::new(String::new(), default_budget());
        state.append_entry(empty_entry("e1", None));
        assert_eq!(state.entries.len(), 1);
        assert_eq!(state.leaf_id, "e1");
        state.append_entry(empty_entry("e2", Some("e1")));
        assert_eq!(state.entries.len(), 2);
        assert_eq!(state.leaf_id, "e2");
    }

    #[test]
    fn host_state_projection_lifecycle() {
        let mut state = HostState::new(String::new(), default_budget());
        let messages = vec![AgentMessage::user("hello")];
        let (projected, report) = state.project("You are helpful.", &messages);
        assert!(!projected.is_empty());
        assert!(report.estimated_tokens > 0);
        // projection_state should be updated (current_turn derived from messages)
        assert_eq!(state.projection_state.current_turn, 0);
    }

    #[test]
    fn host_state_artifact_store() {
        let mut state = HostState::new(String::new(), default_budget());
        state.store_artifact("art-1".to_string(), "hello world".to_string());
        assert_eq!(state.read_artifact("art-1"), Some("hello world"));
        assert_eq!(state.read_artifact("missing"), None);
    }

    #[test]
    fn host_state_artifact_eviction() {
        let mut state = HostState::new(String::new(), default_budget());
        for i in 0..1002 {
            state.store_artifact(format!("art-{i:04}"), "x".to_string());
        }
        assert_eq!(state.artifacts.len(), MAX_ARTIFACTS);
        assert!(!state.artifacts.contains_key("art-0000"));
        assert!(!state.artifacts.contains_key("art-0001"));
        assert!(state.artifacts.contains_key("art-0002"));
        assert!(state.artifacts.contains_key("art-1001"));
    }

    #[test]
    fn host_state_artifact_read() {
        let mut state = HostState::new(String::new(), default_budget());
        state.store_artifact("a1".to_string(), "text content".to_string());
        assert_eq!(state.read_artifact("a1"), Some("text content"));
        assert_eq!(state.read_artifact("a2"), None);
    }

    #[test]
    fn host_state_artifact_search() {
        let mut state = HostState::new(String::new(), default_budget());
        state.store_artifact("a1".to_string(), "the quick brown fox".to_string());
        state.store_artifact("a2".to_string(), "lazy dog sleeping".to_string());
        let results = state.search_artifacts("quick");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "a1");
        assert!(results[0].snippet.contains("quick"));
        assert_eq!(results[0].match_count, 1);
    }

    #[test]
    fn host_state_budget_from_config() {
        let budget = ContextProjectionBudget {
            max_tool_result_chars: 42,
            max_context_tokens: 1000,
            microcompact_after_turns: 3,
            compaction_threshold: 0.5,
        };
        let state = HostState::new(String::new(), budget.clone());
        assert_eq!(state.budget.max_tool_result_chars, 42);
        assert_eq!(state.budget.max_context_tokens, 1000);
        assert_eq!(state.budget.microcompact_after_turns, 3);
        assert_eq!(state.budget.compaction_threshold, 0.5);
    }

    #[test]
    fn host_state_budget_override() {
        let mut state = HostState::new(String::new(), default_budget());
        state.budget.max_tool_result_chars = 999;
        assert_eq!(state.budget.max_tool_result_chars, 999);
    }

    #[test]
    fn host_state_serialize_for_persist() {
        let mut state = HostState::new("You are helpful.".to_string(), default_budget());
        state.name = "test-session".to_string();
        state.append_entry(empty_entry("e1", None));
        state.store_artifact("art-1".to_string(), "data".to_string());
        let data = state.get_persist_data();
        assert_eq!(data.name, "test-session");
        assert_eq!(data.entries.len(), 1);
        assert_eq!(data.artifacts.len(), 1);
        assert_eq!(data.artifacts[0], ("art-1".to_string(), "data".to_string()));
        assert_eq!(data.system_prompt, "You are helpful.");
    }

    #[test]
    fn host_state_restore_from_persist() {
        let data = PersistData {
            entries: vec![empty_entry("e1", None)],
            leaf_id: "e1".to_string(),
            name: "restored".to_string(),
            projection_state: ContextProjectionState::default(),
            artifacts: vec![("a1".to_string(), "hello".to_string())],
            budget: default_budget(),
            system_prompt: "restored-prompt".to_string(),
        };
        let state = HostState::restore(data);
        assert_eq!(state.entries.len(), 1);
        assert_eq!(state.leaf_id, "e1");
        assert_eq!(state.name, "restored");
        assert_eq!(state.read_artifact("a1"), Some("hello"));
        assert_eq!(state.system_prompt, "restored-prompt");
    }

    #[test]
    fn host_state_restore_old_format() {
        let old_json = r#"{
            "entries": [{"id":"e1","parent_id":null,"kind":{"type":"message","role":"user","content":[{"type":"text","text":"hi"}],"timestamp":1},"timestamp":1}],
            "leaf_id": "e1",
            "name": "legacy",
            "projection_state": {"tools":{},"current_turn":3,"turns_since_compaction":1},
            "artifacts": [{"id": "a1", "text": "old-text"}],
            "budget": {"max_tool_result_chars":50000,"max_context_tokens":100000,"microcompact_after_turns":5,"compaction_threshold":0.75}
        }"#;
        let state = HostState::try_migrate_old_session(old_json).unwrap();
        assert_eq!(state.name, "legacy");
        assert_eq!(state.leaf_id, "e1");
        assert_eq!(state.projection_state.current_turn, 3);
        assert_eq!(state.projection_state.turns_since_compaction, 1);
        assert_eq!(state.read_artifact("a1"), Some("old-text"));
        assert_eq!(state.system_prompt, "");
    }

    #[test]
    fn host_state_compaction_detection() {
        let state = HostState::new(String::new(), default_budget());
        let report = ContextProjectionReport {
            estimated_tokens: 100,
            replacements: vec![],
            dropped_messages: 0,
            needs_compaction: true,
            cache_breakpoints: vec![],
        };
        assert!(state.detect_needs_compaction(&report));
        let report2 = ContextProjectionReport {
            estimated_tokens: 100,
            replacements: vec![],
            dropped_messages: 0,
            needs_compaction: false,
            cache_breakpoints: vec![],
        };
        assert!(!state.detect_needs_compaction(&report2));
    }

    #[test]
    fn host_state_compaction_apply() {
        let mut state = HostState::new(String::new(), default_budget());
        state.append_entry(empty_entry("e1", None));
        state.append_entry(empty_entry("e2", Some("e1")));
        state.append_entry(empty_entry("e3", Some("e2")));

        let plan = CompactionPlan {
            cut_index: 2,
            entries_to_summarize: state.entries[..2].to_vec(),
            tokens_to_free: 10,
        };
        state.accept_compaction(plan, "summary".to_string());
        assert_eq!(state.entries.len(), 2);
        assert!(matches!(state.entries[0].kind, EntryKind::Compaction { .. }));
        assert_eq!(state.entries[1].id, "e3");
        assert_eq!(state.leaf_id, "e3");
    }

    #[test]
    fn dto_conversion_without_projection() {
        let core = pi_core::SessionState {
            entries: vec![SessionEntry {
                id: "e1".to_string(),
                parent_id: None,
                kind: EntryKind::Message {
                    message: AgentMessage::user("hi"),
                },
                timestamp: 1,
            }],
            leaf_id: "e1".to_string(),
            name: "test".to_string(),
        };
        let json = serde_json::to_string(&core).unwrap();
        let back: pi_core::SessionState = serde_json::from_str(&json).unwrap();
        assert_eq!(core, back);
        // Old format with extra fields should be ignored
        let old_json = r#"{"entries":[],"leaf_id":"","name":"old","projection_state":{},"artifacts":[]}"#;
        let old: pi_core::SessionState = serde_json::from_str(old_json).unwrap();
        assert_eq!(old.name, "old");
    }
}
