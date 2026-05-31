#![allow(dead_code)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use pi_core::{Artifacts, ContextProjectionBudget, TrimmedMessage};

const MAX_ARTIFACTS: usize = 1000;

/// Search result for a single artifact match.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactSearchResult {
    pub id: String,
    pub snippet: String,
    pub match_count: usize,
}

/// Serializable snapshot of host state for persistence.
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

/// Owns host-side state: system prompt, artifacts, and compaction prompt.
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
}

/// Internal directive set used by the TUI host to coordinate between
/// the core `AgentAction` stream and the host-side behaviour.
#[derive(Debug, Clone, PartialEq)]
pub enum HostDirective {
    StreamLlm {
        context: pi_core::LlmContext,
    },
    Summarize {
        context: pi_core::LlmContext,
    },
    ExecuteTools {
        calls: Vec<pi_core::ToolCall>,
    },
    CancelTools {
        tool_call_ids: Vec<pi_core::ToolCallId>,
        reason: pi_core::CancelReason,
    },
    Persist,
    Finished,
    WaitForInput {
        mode: pi_core::WaitMode,
    },
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
    fn tui_host_state_new() {
        let state = HostState::new(String::new(), String::new());
        assert!(state.artifacts.is_empty());
        assert_eq!(state.system_prompt, "");
        assert_eq!(state.compaction_prompt, "");
    }

    #[test]
    fn tui_host_state_artifact_store() {
        let mut state = HostState::new(String::new(), String::new());
        state.store_artifact("art-1".to_string(), "hello world".to_string());
        assert_eq!(state.read_artifact("art-1"), Some("hello world"));
        assert_eq!(state.read_artifact("missing"), None);
    }

    #[test]
    fn tui_host_state_artifact_eviction() {
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
    fn tui_host_state_artifact_search() {
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
    fn tui_session_persistence() {
        let mut state = HostState::new("You are helpful.".to_string(), "Summarize.".to_string());
        state.store_artifact("art-1".to_string(), "data".to_string());

        let transcript: Vec<TrimmedMessage> = vec![];
        let artifacts = Artifacts::new();

        let data = state.get_persist_data(&transcript, &artifacts, 0, &default_budget());
        assert!(data.transcript.is_empty());
        assert!(data.artifacts.is_empty());
        assert_eq!(data.turn_number, 0);
        assert_eq!(data.host_artifacts.len(), 1);
        assert_eq!(
            data.host_artifacts[0],
            ("art-1".to_string(), "data".to_string())
        );
        assert_eq!(data.system_prompt, "You are helpful.");
        assert_eq!(data.compaction_prompt, "Summarize.");

        let restored = HostState::restore(data.clone());
        assert_eq!(restored.system_prompt, "You are helpful.");
        assert_eq!(restored.compaction_prompt, "Summarize.");
        assert_eq!(restored.read_artifact("art-1"), Some("data"));
    }

    #[test]
    fn tui_directive_handling() {
        let directive = HostDirective::Finished;
        assert!(matches!(directive, HostDirective::Finished));
        let directive = HostDirective::Persist;
        assert!(matches!(directive, HostDirective::Persist));
        let directive = HostDirective::StreamLlm {
            context: pi_core::LlmContext {
                system_prompt: "test".to_string(),
                messages: vec![],
                tools: vec![],
            },
        };
        assert!(matches!(directive, HostDirective::StreamLlm { .. }));
        let directive = HostDirective::Summarize {
            context: pi_core::LlmContext {
                system_prompt: "test".to_string(),
                messages: vec![],
                tools: vec![],
            },
        };
        assert!(matches!(directive, HostDirective::Summarize { .. }));
    }
}
