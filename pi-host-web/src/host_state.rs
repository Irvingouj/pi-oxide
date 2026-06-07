use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tsify::Tsify;

use pi_core::{Artifacts, ContextProjectionBudget, OriginalToolResult, TrimmedMessage};

const MAX_ARTIFACTS: usize = 1000;

/// Search result for a single artifact match.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct ArtifactSearchResult {
    pub id: String,
    pub snippet: String,
    pub match_count: usize,
}

/// Serializable snapshot of host state for persistence (internal only).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistData {
    #[serde(rename = "T")]
    pub transcript: Vec<TrimmedMessage>,
    #[serde(rename = "A")]
    pub artifacts: Artifacts,
    pub turn_number: u32,
    pub host_artifacts: Vec<(String, String)>,
    pub budget: ContextProjectionBudget,
    pub system_prompt: String,
    pub compaction_prompt: String,
}

/// Owns all host-side state: projection state, artifacts, and budget.
#[derive(Debug, Clone, PartialEq)]
pub struct HostState {
    pub system_prompt: String,
    pub compaction_prompt: String,
    pub artifacts: BTreeMap<String, String>,
}

impl HostState {
    /// Initialize with empty state.
    pub fn new(system_prompt: String, compaction_prompt: String) -> Self {
        Self {
            system_prompt,
            compaction_prompt,
            artifacts: BTreeMap::new(),
        }
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
    pub fn get_persist_data(
        &self,
        transcript: &[TrimmedMessage],
        artifacts: &Artifacts,
        turn_number: u32,
        budget: &ContextProjectionBudget,
    ) -> PersistData {
        PersistData {
            transcript: transcript.to_vec(),
            artifacts: artifacts.clone(),
            turn_number,
            host_artifacts: self
                .artifacts
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            budget: budget.clone(),
            system_prompt: self.system_prompt.clone(),
            compaction_prompt: self.compaction_prompt.clone(),
        }
    }

    /// Restore host state from a persisted snapshot.
    pub fn restore(data: PersistData) -> Self {
        Self {
            system_prompt: data.system_prompt,
            compaction_prompt: data.compaction_prompt,
            artifacts: data.host_artifacts.into_iter().collect(),
        }
    }

    /// Sync specific artifacts from core's A into host_state.artifacts.
    /// First sync wins — does not overwrite existing entries.
    pub fn sync_artifacts_from_core(&mut self, artifacts: &Artifacts, entry_ids: &[String]) {
        for id in entry_ids {
            if !self.artifacts.contains_key(id) {
                if let Some(original) = artifacts.get(id) {
                    let text = extract_text_from_tool_result(original);
                    self.store_artifact(id.clone(), text);
                }
            }
        }
    }

    /// Sync all missing artifacts from core's A into host_state.artifacts.
    /// Does not overwrite existing entries.
    pub fn sync_missing_artifacts_from_core(&mut self, artifacts: &Artifacts) {
        for (id, original) in artifacts {
            if !self.artifacts.contains_key(id) {
                let text = extract_text_from_tool_result(original);
                self.store_artifact(id.clone(), text);
            }
        }
    }
}

/// Extract plain text from an OriginalToolResult, using placeholders for non-text content.
pub fn extract_text_from_tool_result(original: &OriginalToolResult) -> String {
    let mut parts: Vec<String> = Vec::new();
    for c in &original.content {
        match c {
            pi_core::Content::Text(t) => parts.push(t.text.clone()),
            pi_core::Content::Image(img) => parts.push(format!("[image: {}]", img.media_type)),
            pi_core::Content::ToolCall(tc) => {
                parts.push(format!("[tool_call: {}]", tc.name.as_str()))
            }
        }
    }
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_budget() -> ContextProjectionBudget {
        ContextProjectionBudget {
            max_tool_result_chars: 50000,
            max_context_tokens: 100000,
            microcompact_after_turns: 5,
            compaction_threshold: 0.75,
        }
    }

    #[test]
    fn host_state_new_default() {
        let state = HostState::new(String::new(), String::new());
        assert!(state.artifacts.is_empty());
        assert_eq!(state.system_prompt, "");
        assert_eq!(state.compaction_prompt, "");
    }

    #[test]
    fn host_state_artifact_store() {
        let mut state = HostState::new(String::new(), String::new());
        state.store_artifact("art-1".to_string(), "hello world".to_string());
        assert_eq!(state.read_artifact("art-1"), Some("hello world"));
        assert_eq!(state.read_artifact("missing"), None);
    }

    #[test]
    fn host_state_artifact_eviction() {
        let mut state = HostState::new(String::new(), String::new());
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
        let mut state = HostState::new(String::new(), String::new());
        state.store_artifact("a1".to_string(), "text content".to_string());
        assert_eq!(state.read_artifact("a1"), Some("text content"));
        assert_eq!(state.read_artifact("a2"), None);
    }

    #[test]
    fn host_state_artifact_search() {
        let mut state = HostState::new(String::new(), String::new());
        state.store_artifact("a1".to_string(), "the quick brown fox".to_string());
        state.store_artifact("a2".to_string(), "lazy dog sleeping".to_string());
        let results = state.search_artifacts("quick");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "a1");
        assert!(results[0].snippet.contains("quick"));
        assert_eq!(results[0].match_count, 1);
    }

    #[test]
    fn host_state_serialize_for_persist() {
        let mut state = HostState::new("You are helpful.".to_string(), "Summarize.".to_string());
        state.store_artifact("art-1".to_string(), "data".to_string());
        let transcript: Vec<TrimmedMessage> = vec![];
        let artifacts = Artifacts::new();
        let data = state.get_persist_data(&transcript, &artifacts, 1, &default_budget());
        assert_eq!(data.turn_number, 1);
        assert_eq!(data.host_artifacts.len(), 1);
        assert_eq!(
            data.host_artifacts[0],
            ("art-1".to_string(), "data".to_string())
        );
        assert_eq!(data.system_prompt, "You are helpful.");
        assert_eq!(data.compaction_prompt, "Summarize.");
    }

    #[test]
    fn host_state_restore_from_persist() {
        let data = PersistData {
            transcript: vec![],
            artifacts: Artifacts::new(),
            turn_number: 1,
            host_artifacts: vec![("a1".to_string(), "hello".to_string())],
            budget: default_budget(),
            system_prompt: "restored-prompt".to_string(),
            compaction_prompt: "restored-compaction".to_string(),
        };
        let state = HostState::restore(data);
        assert_eq!(state.read_artifact("a1"), Some("hello"));
        assert_eq!(state.system_prompt, "restored-prompt");
        assert_eq!(state.compaction_prompt, "restored-compaction");
    }

    #[test]
    fn sync_artifacts_from_core_guard() {
        let mut state = HostState::new("sp".to_string(), "cp".to_string());
        state.store_artifact("old-id".to_string(), "existing text".to_string());

        let mut core_artifacts = pi_core::Artifacts::new();
        core_artifacts.insert(
            "old-id".to_string(),
            pi_core::OriginalToolResult {
                entry_id: "old-id".to_string(),
                tool_call_id: pi_core::ToolCallId::new("tc1"),
                tool_name: pi_core::ToolName::new("bash"),
                content: vec![pi_core::Content::Text(pi_core::TextContent {
                    text: "new text".to_string(),
                })],
                is_error: false,
                turn: 1,
            },
        );
        core_artifacts.insert(
            "new-id".to_string(),
            pi_core::OriginalToolResult {
                entry_id: "new-id".to_string(),
                tool_call_id: pi_core::ToolCallId::new("tc2"),
                tool_name: pi_core::ToolName::new("bash"),
                content: vec![pi_core::Content::Text(pi_core::TextContent {
                    text: "new artifact".to_string(),
                })],
                is_error: false,
                turn: 1,
            },
        );

        state.sync_artifacts_from_core(
            &core_artifacts,
            &["old-id".to_string(), "new-id".to_string()],
        );

        assert_eq!(
            state.read_artifact("old-id"),
            Some("existing text"),
            "old-id should NOT be overwritten"
        );
        assert_eq!(
            state.read_artifact("new-id"),
            Some("new artifact"),
            "new-id should be inserted"
        );
    }

    #[test]
    fn sync_missing_artifacts_from_core() {
        let mut state = HostState::new("sp".to_string(), "cp".to_string());
        state.store_artifact("existing".to_string(), "existing text".to_string());

        let mut core_artifacts = pi_core::Artifacts::new();
        core_artifacts.insert(
            "existing".to_string(),
            pi_core::OriginalToolResult {
                entry_id: "existing".to_string(),
                tool_call_id: pi_core::ToolCallId::new("tc1"),
                tool_name: pi_core::ToolName::new("bash"),
                content: vec![pi_core::Content::Text(pi_core::TextContent {
                    text: "new text".to_string(),
                })],
                is_error: false,
                turn: 1,
            },
        );
        core_artifacts.insert(
            "missing".to_string(),
            pi_core::OriginalToolResult {
                entry_id: "missing".to_string(),
                tool_call_id: pi_core::ToolCallId::new("tc2"),
                tool_name: pi_core::ToolName::new("bash"),
                content: vec![pi_core::Content::Text(pi_core::TextContent {
                    text: "missing text".to_string(),
                })],
                is_error: false,
                turn: 1,
            },
        );

        state.sync_missing_artifacts_from_core(&core_artifacts);

        assert_eq!(
            state.read_artifact("existing"),
            Some("existing text"),
            "existing should be unchanged"
        );
        assert_eq!(
            state.read_artifact("missing"),
            Some("missing text"),
            "missing should be inserted"
        );
    }

    #[test]
    fn extract_text_from_tool_result_all_variants() {
        let original = pi_core::OriginalToolResult {
            entry_id: "entry-1".to_string(),
            tool_call_id: pi_core::ToolCallId::new("tc1"),
            tool_name: pi_core::ToolName::new("bash"),
            content: vec![
                pi_core::Content::Text(pi_core::TextContent {
                    text: "actual text".to_string(),
                }),
                pi_core::Content::Image(pi_core::ImageContent {
                    media_type: "image/png".to_string(),
                    data: "base64data".to_string(),
                }),
                pi_core::Content::ToolCall(pi_core::ToolCall {
                    id: pi_core::ToolCallId::new("tc2"),
                    name: pi_core::ToolName::new("read"),
                    arguments: pi_core::ToolArguments(serde_json::json!({})),
                }),
            ],
            is_error: false,
            turn: 1,
        };

        let text = super::extract_text_from_tool_result(&original);
        assert!(text.contains("actual text"), "should contain actual text");
        assert!(
            text.contains("[image: image/png]"),
            "should contain image placeholder"
        );
        assert!(
            text.contains("[tool_call: read]"),
            "should contain tool_call placeholder"
        );
    }
}
