use serde::{Deserialize, Serialize};

use pi_core::{Artifacts, ContextProjectionBudget, TrimmedMessage};

/// Serializable snapshot of host state for persistence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistData {
    #[serde(rename = "T")]
    pub transcript: Vec<TrimmedMessage>,
    #[serde(rename = "A")]
    pub artifacts: Artifacts,
    pub turn_number: u32,
    pub budget: ContextProjectionBudget,
    pub system_prompt: String,
    pub compaction_prompt: String,
}

/// Session context restored from a persisted snapshot.
#[derive(Debug, Clone)]
pub struct SessionContext {
    pub transcript: Vec<TrimmedMessage>,
    pub artifacts: Artifacts,
    pub turn_number: u32,
    pub budget: ContextProjectionBudget,
}

/// Host-side state: system prompt and compaction prompt.
#[derive(Debug, Clone, PartialEq)]
pub struct HostState {
    pub system_prompt: String,
    pub compaction_prompt: String,
}

impl HostState {
    /// Initialize with the given prompts.
    pub fn new(system_prompt: String, compaction_prompt: String) -> Self {
        Self {
            system_prompt,
            compaction_prompt,
        }
    }

    /// Serialize host state for persistence.
    pub fn get_persist_data(&self, session_ctx: &SessionContext) -> PersistData {
        PersistData {
            transcript: session_ctx.transcript.clone(),
            artifacts: session_ctx.artifacts.clone(),
            turn_number: session_ctx.turn_number,
            budget: session_ctx.budget.clone(),
            system_prompt: self.system_prompt.clone(),
            compaction_prompt: self.compaction_prompt.clone(),
        }
    }

    /// Restore host state and session context from a persisted snapshot.
    pub fn restore(data: PersistData) -> (Self, SessionContext) {
        (
            Self {
                system_prompt: data.system_prompt,
                compaction_prompt: data.compaction_prompt,
            },
            SessionContext {
                transcript: data.transcript,
                artifacts: data.artifacts,
                turn_number: data.turn_number,
                budget: data.budget,
            },
        )
    }
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
    fn host_state_new() {
        let state = HostState::new("sys".into(), "compact".into());
        assert_eq!(state.system_prompt, "sys");
        assert_eq!(state.compaction_prompt, "compact");
    }

    #[test]
    fn host_state_persistence_roundtrip() {
        let state = HostState::new("You are helpful.".into(), "Summarize.".into());

        let session_ctx = SessionContext {
            transcript: vec![],
            artifacts: Artifacts::new(),
            turn_number: 0,
            budget: default_budget(),
        };

        let data = state.get_persist_data(&session_ctx);
        assert!(data.transcript.is_empty());
        assert!(data.artifacts.is_empty());
        assert_eq!(data.turn_number, 0);
        assert_eq!(data.system_prompt, "You are helpful.");
        assert_eq!(data.compaction_prompt, "Summarize.");
        assert_eq!(data.budget.max_context_tokens, 100000);

        let (restored_state, restored_ctx) = HostState::restore(data);
        assert_eq!(restored_state.system_prompt, "You are helpful.");
        assert_eq!(restored_state.compaction_prompt, "Summarize.");
        assert!(restored_ctx.transcript.is_empty());
        assert_eq!(restored_ctx.turn_number, 0);
        assert_eq!(restored_ctx.budget.max_context_tokens, 100000);
    }
}
