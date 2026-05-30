use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{debug, trace};

use crate::context_projection::{estimate_tokens, ContextProjectionBudget};
use crate::events::ThinkingLevel;
use crate::message::{AgentMessage, Content};

/// An entry in the session tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionEntry {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub kind: EntryKind,
    pub timestamp: u64,
}

/// The kind of a session entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EntryKind {
    Message {
        #[serde(flatten)]
        message: AgentMessage,
    },
    Compaction {
        summary: String,
        first_kept_entry_id: String,
        tokens_before: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        details: Option<Value>,
    },
    BranchSummary {
        summary: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        details: Option<Value>,
    },
    ModelChange {
        provider: String,
        model_id: String,
    },
    ThinkingLevelChange(ThinkingLevel),
    Custom {
        custom_type: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<Value>,
    },
}

/// In-memory session state managed by the core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SessionState {
    pub entries: Vec<SessionEntry>,
    pub leaf_id: String,
    #[serde(default)]
    pub name: String,
}

impl SessionState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a session tree from existing messages (backfill for agents created before session support).
    pub fn from_messages(messages: &[AgentMessage]) -> Self {
        let mut state = Self::new();
        for msg in messages {
            let id = format!("entry-{}", state.entries.len());
            let parent_id = state.entries.last().map(|e| e.id.clone());
            state.entries.push(SessionEntry {
                id,
                parent_id,
                kind: EntryKind::Message {
                    message: msg.clone(),
                },
                timestamp: current_timestamp(),
            });
        }
        if let Some(last) = state.entries.last() {
            state.leaf_id = last.id.clone();
        }
        trace!(
            entries = state.entries.len(),
            "session state built from messages"
        );
        state
    }

    /// Get the full branch from root to leaf.
    pub fn get_branch(&self) -> Vec<&SessionEntry> {
        let index: std::collections::HashMap<&str, &SessionEntry> =
            self.entries.iter().map(|e| (e.id.as_str(), e)).collect();

        let mut branch = Vec::new();
        let mut current = index.get(self.leaf_id.as_str()).copied();
        trace!(
            leaf_id = self.leaf_id,
            total_entries = self.entries.len(),
            "get_branch called"
        );

        while let Some(entry) = current {
            branch.push(entry);
            current = entry
                .parent_id
                .as_ref()
                .and_then(|pid| index.get(pid.as_str()))
                .copied();
        }

        branch.reverse();
        branch
    }

    /// Build the LLM-visible context from the current branch.
    pub fn build_context(&self) -> Vec<AgentMessage> {
        let ctx = self
            .get_branch()
            .iter()
            .filter_map(|e| match &e.kind {
                EntryKind::Message { message } => Some(message.clone()),
                EntryKind::Compaction { summary, .. } => {
                    Some(AgentMessage::user(format!("Previous conversation summary: {}", summary)))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        trace!(message_count = ctx.len(), "build_context");
        ctx
    }

    /// Move to a target node, optionally creating a branch summary.
    pub fn move_to(&mut self, target_id: &str, summary: Option<BranchSummary>) -> Option<String> {
        let target = self.entries.iter().find(|e| e.id == target_id)?;
        self.leaf_id = target_id.to_string();
        trace!(target_id, "session moved to target");

        if let Some(summary) = summary {
            let summary_id = format!("summary-{}", self.entries.len());
            debug!(summary_id, target_id, "branch summary created");
            let entry = SessionEntry {
                id: summary_id.clone(),
                parent_id: target.parent_id.clone(),
                kind: EntryKind::BranchSummary {
                    summary: summary.summary,
                    details: summary.details,
                },
                timestamp: current_timestamp(),
            };
            self.entries.push(entry);
            return Some(summary_id);
        }

        None
    }
}

/// Summary created when navigating between branches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BranchSummary {
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

/// Host-provided session storage trait.
/// Core holds no I/O; the host implements persistence.
pub trait SessionStorage: Send + Sync {
    fn append_entry(&mut self, entry: SessionEntry) -> Result<String, SessionError>;
    fn get_entry(&self, id: &str) -> Result<Option<SessionEntry>, SessionError>;
    fn get_branch(&self, leaf_id: &str) -> Result<Vec<SessionEntry>, SessionError>;
    fn move_to(
        &mut self,
        target_id: &str,
        summary: Option<BranchSummary>,
    ) -> Result<Option<String>, SessionError>;
    fn set_leaf_id(&mut self, id: &str) -> Result<(), SessionError>;
    fn get_leaf_id(&self) -> Result<String, SessionError>;
    fn append_compaction(
        &mut self,
        summary: String,
        first_kept: String,
        tokens: u32,
        details: Value,
    ) -> Result<String, SessionError>;
}

/// Session storage errors.
#[derive(Debug, Clone, PartialEq, thiserror::Error, Serialize, Deserialize)]
pub enum SessionError {
    #[error("entry not found: {0}")]
    NotFound(String),
    #[error("storage error: {message}")]
    Storage { message: String },
    #[error("invalid state: {message}")]
    InvalidState { message: String },
}

/// Plan for compacting old session entries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompactionPlan {
    /// Index into entries: entries before this should be summarized
    pub cut_index: usize,
    /// Entries that will be summarized (indices [0..cut_index])
    pub entries_to_summarize: Vec<SessionEntry>,
    /// Estimated tokens to free
    pub tokens_to_free: usize,
}

/// Build a list of messages from entries for token estimation.
fn entry_messages(entries: &[SessionEntry]) -> Vec<AgentMessage> {
    entries
        .iter()
        .filter_map(|e| match &e.kind {
            EntryKind::Message { message } => Some(message.clone()),
            EntryKind::Compaction { summary, .. } => {
                Some(AgentMessage::user(format!("Previous conversation summary: {}", summary)))
            }
            _ => None,
        })
        .collect()
}

/// Plan which entries to compact based on the budget.
pub fn plan_compaction(
    entries: &[SessionEntry],
    budget: &ContextProjectionBudget,
) -> Option<CompactionPlan> {
    let total_tokens = estimate_tokens(&entry_messages(entries));
    let threshold = (budget.compaction_threshold * budget.max_context_tokens as f32) as usize;

    if total_tokens <= threshold {
        return None;
    }

    // Find turn boundaries at User message entries
    let mut boundaries = vec![0];
    for (i, entry) in entries.iter().enumerate() {
        if i > 0
            && matches!(
                entry.kind,
                EntryKind::Message {
                    message: AgentMessage::User(..)
                }
            )
        {
            boundaries.push(i);
        }
    }
    boundaries.push(entries.len());

    let num_turns = boundaries.len().saturating_sub(1);
    if num_turns == 0 {
        return None;
    }

    let target_keep_tokens =
        (budget.compaction_threshold * budget.max_context_tokens as f32 / 2.0) as usize;

    // Start with keeping the last turn
    let mut cut_index = boundaries[num_turns.saturating_sub(1)];
    let mut kept_tokens = estimate_tokens(&entry_messages(&entries[cut_index..]));

    // Try to keep more turns going backwards
    for i in (0..num_turns.saturating_sub(1)).rev() {
        let turn_start = boundaries[i];
        let turn_end = boundaries[i + 1];
        let turn_tokens = estimate_tokens(&entry_messages(&entries[turn_start..turn_end]));

        if kept_tokens + turn_tokens > target_keep_tokens {
            break;
        }
        kept_tokens += turn_tokens;
        cut_index = turn_start;
    }

    // Adjust for tool split safety: never split a ToolCall/ToolResult pair
    let mut earliest_split = cut_index;
    let mut tool_call_indices: std::collections::HashMap<&str, usize> =
        std::collections::HashMap::new();
    let mut tool_result_indices: std::collections::HashMap<&str, usize> =
        std::collections::HashMap::new();

    for (i, entry) in entries.iter().enumerate() {
        if let EntryKind::Message { message } = &entry.kind {
            match message {
                AgentMessage::Assistant(a) => {
                    for block in &a.content {
                        if let Content::ToolCall(tc) = block {
                            tool_call_indices.insert(tc.id.as_str(), i);
                        }
                    }
                }
                AgentMessage::ToolResult(tr) => {
                    tool_result_indices.insert(tr.tool_call_id.as_str(), i);
                }
                _ => {}
            }
        }
    }

    for (id, call_idx) in &tool_call_indices {
        if let Some(result_idx) = tool_result_indices.get(id) {
            let call_before = *call_idx < cut_index;
            let result_before = *result_idx < cut_index;
            if call_before != result_before {
                // Split detected. Move cut back to include both in kept portion.
                earliest_split = earliest_split.min(*call_idx.min(result_idx));
            }
        }
    }

    if earliest_split < cut_index {
        // Snap back to the nearest turn boundary that is <= earliest_split
        cut_index = 0;
        for &boundary in &boundaries {
            if boundary <= earliest_split {
                cut_index = boundary;
            } else {
                break;
            }
        }
    }

    if cut_index == 0 {
        return None;
    }

    let entries_to_summarize = entries[..cut_index].to_vec();
    let tokens_to_free = estimate_tokens(&entry_messages(&entries_to_summarize));

    Some(CompactionPlan {
        cut_index,
        entries_to_summarize,
        tokens_to_free,
    })
}

/// Apply a compaction plan to a list of entries.
pub fn apply_compaction(
    entries: Vec<SessionEntry>,
    plan: CompactionPlan,
    summary: String,
) -> Vec<SessionEntry> {
    let cut_index = plan.cut_index;

    let first_kept_entry_id = if cut_index < entries.len() {
        entries[cut_index].id.clone()
    } else {
        String::new()
    };

    let tokens_before = estimate_tokens(&entry_messages(&plan.entries_to_summarize)) as u32;

    let compaction_id = format!("compaction-{}", entries.len());

    let ids_before_cut: std::collections::HashSet<String> =
        entries[..cut_index].iter().map(|e| e.id.clone()).collect();

    let parent_id = if cut_index < entries.len() {
        if let Some(ref pid) = entries[cut_index].parent_id {
            if ids_before_cut.contains(pid) {
                None
            } else {
                Some(pid.clone())
            }
        } else {
            None
        }
    } else {
        None
    };

    let mut kept_entries: Vec<SessionEntry> = entries.into_iter().skip(cut_index).collect();

    for entry in &mut kept_entries {
        if let Some(ref pid) = entry.parent_id {
            if ids_before_cut.contains(pid) {
                entry.parent_id = Some(compaction_id.clone());
            }
        }
    }
    // If the first kept entry was a root, it must be reparented to the compaction entry.
    if let Some(first) = kept_entries.first_mut() {
        if first.parent_id.is_none() {
            first.parent_id = Some(compaction_id.clone());
        }
    }

    let compaction_entry = SessionEntry {
        id: compaction_id,
        parent_id,
        kind: EntryKind::Compaction {
            summary,
            first_kept_entry_id,
            tokens_before,
            details: None,
        },
        timestamp: current_timestamp(),
    };

    let mut result = vec![compaction_entry];
    result.extend(kept_entries);
    result
}

fn current_timestamp() -> u64 {
    crate::timestamp::current_timestamp()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{Agent, AgentOptions};
    use crate::llm::{LlmResult, Model, ModelCapabilities, ModelCost};
    use crate::message::{
        AgentMessage, AssistantMessage, Content, StopReason, TextContent, TokenUsage, ToolCall,
        ToolResultMessage,
    };
    use crate::types::{ApiName, ModelId, ModelName, ProviderName, ToolArguments, ToolCallId, ToolName};

    fn dummy_options() -> AgentOptions {
        AgentOptions {
            system_prompt: "test".to_string(),
            model: Model {
                id: ModelId("test".to_string()),
                name: ModelName("Test".to_string()),
                api: ApiName("test".to_string()),
                provider: ProviderName("test".to_string()),
                base_url: None,
                reasoning: false,
                context_window: 4096,
                max_tokens: 1024,
                capabilities: ModelCapabilities::default(),
                cost: ModelCost::default(),
            },
            thinking_level: Default::default(),
            steering_mode: Default::default(),
            follow_up_mode: Default::default(),
            tool_execution_mode: Default::default(),
            session_id: None,
            messages: vec![],
            session_state: None,
        }
    }

    #[test]
    fn session_state_default_has_no_projection() {
        let state = SessionState::default();
        assert!(state.entries.is_empty());
        assert_eq!(state.leaf_id, "");
        assert_eq!(state.name, "");
    }

    #[test]
    fn session_state_serialize_roundtrip() {
        let state = SessionState {
            entries: vec![SessionEntry {
                id: "e1".to_string(),
                parent_id: None,
                kind: EntryKind::Message {
                    message: AgentMessage::user("hello"),
                },
                timestamp: 1,
            }],
            leaf_id: "e1".to_string(),
            name: "test".to_string(),
        };
        let json = serde_json::to_string(&state).unwrap();
        let back: SessionState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, back);
    }

    #[test]
    fn session_state_deserialize_old_format() {
        let old_json = r#"{
            "entries": [],
            "leaf_id": "",
            "name": "legacy",
            "projection_state": {"tools": {}, "current_turn": 0},
            "artifacts": [{"id": "a1", "text": "old"}]
        }"#;
        let state: SessionState = serde_json::from_str(old_json).unwrap();
        assert_eq!(state.name, "legacy");
        assert!(state.entries.is_empty());
    }

    #[test]
    fn session_state_deserialize_no_projection() {
        let json = r#"{"entries": [], "leaf_id": "", "name": "clean"}"#;
        let state: SessionState = serde_json::from_str(json).unwrap();
        assert_eq!(state.name, "clean");
        assert!(state.entries.is_empty());
    }

    #[test]
    fn agent_new_without_projection_state() {
        let agent = Agent::new(dummy_options());
        assert!(agent.state().messages.is_empty());
    }

    #[test]
    fn agent_full_turn_without_projection() {
        let mut agent = Agent::new(dummy_options());
        let mut session_state = SessionState::default();
        let prompt = AgentMessage::user("hello");
        let (events, actions) = agent.start_turn(prompt, vec![], &mut session_state);
        assert!(!events.is_empty());
        assert!(actions.iter().any(|a| matches!(a, crate::events::AgentAction::StreamLlm { .. })));

        let assistant = AssistantMessage {
            content: vec![Content::Text(TextContent {
                text: "hi".to_string(),
            })],
            api: ApiName("test".to_string()),
            provider: ProviderName("test".to_string()),
            model: ModelId("test".to_string()),
            stop_reason: StopReason::EndTurn,
            error_message: None,
            timestamp: 1,
            usage: TokenUsage::default(),
        };
        let (events, actions) = agent.on_llm_done(LlmResult::Ok(assistant), &mut session_state);
        assert!(!events.is_empty());
        assert!(actions.iter().any(|a| matches!(a, crate::events::AgentAction::Finished { .. })));
        assert!(!session_state.entries.is_empty());
    }

    #[test]
    fn session_state_tree_ops_unchanged() {
        let mut state = SessionState::from_messages(&[
            AgentMessage::user("a"),
            AgentMessage::user("b"),
        ]);
        assert_eq!(state.entries.len(), 2);
        assert_eq!(state.leaf_id, "entry-1");

        let branch = state.get_branch();
        assert_eq!(branch.len(), 2);
        assert_eq!(branch[0].id, "entry-0");
        assert_eq!(branch[1].id, "entry-1");

        let summary = state.move_to("entry-0", Some(BranchSummary {
            summary: "moved".to_string(),
            details: None,
        }));
        assert!(summary.is_some());
        assert_eq!(state.leaf_id, "entry-0");
        assert_eq!(state.entries.len(), 3);
    }

    // --- Phase 2 compaction tests ---

    #[test]
    fn plan_compaction_empty_entries() {
        let entries: Vec<SessionEntry> = vec![];
        let budget = ContextProjectionBudget::default();
        assert!(plan_compaction(&entries, &budget).is_none());
    }

    #[test]
    fn plan_compaction_under_budget() {
        let entries = vec![
            SessionEntry {
                id: "e1".to_string(),
                parent_id: None,
                kind: EntryKind::Message {
                    message: AgentMessage::user("hello"),
                },
                timestamp: 1,
            },
            SessionEntry {
                id: "e2".to_string(),
                parent_id: Some("e1".to_string()),
                kind: EntryKind::Message {
                    message: AgentMessage::assistant_text("hi"),
                },
                timestamp: 2,
            },
        ];
        let budget = ContextProjectionBudget {
            max_context_tokens: 100_000,
            compaction_threshold: 0.75,
            ..Default::default()
        };
        assert!(plan_compaction(&entries, &budget).is_none());
    }

    #[test]
    fn plan_compaction_over_budget() {
        let mut entries = Vec::new();
        for i in 0..10 {
            entries.push(SessionEntry {
                id: format!("u{}", i),
                parent_id: if i == 0 { None } else { Some(format!("a{}", i - 1)) },
                kind: EntryKind::Message {
                    message: AgentMessage::user(&"A".repeat(10_000)),
                },
                timestamp: i as u64 * 2,
            });
            entries.push(SessionEntry {
                id: format!("a{}", i),
                parent_id: Some(format!("u{}", i)),
                kind: EntryKind::Message {
                    message: AgentMessage::assistant_text(&"B".repeat(10_000)),
                },
                timestamp: i as u64 * 2 + 1,
            });
        }
        let budget = ContextProjectionBudget {
            max_context_tokens: 1_000,
            compaction_threshold: 0.75,
            ..Default::default()
        };
        let plan = plan_compaction(&entries, &budget);
        assert!(plan.is_some());
        let plan = plan.unwrap();
        assert!(plan.cut_index > 0);
        assert!(plan.cut_index < entries.len());
        assert!(plan.tokens_to_free > 0);
    }

    #[test]
    fn plan_compaction_respects_keep_recent() {
        let mut entries = Vec::new();
        for i in 0..10 {
            entries.push(SessionEntry {
                id: format!("u{}", i),
                parent_id: if i == 0 { None } else { Some(format!("a{}", i - 1)) },
                kind: EntryKind::Message {
                    message: AgentMessage::user(&"A".repeat(1_000)),
                },
                timestamp: i as u64 * 2,
            });
            entries.push(SessionEntry {
                id: format!("a{}", i),
                parent_id: Some(format!("u{}", i)),
                kind: EntryKind::Message {
                    message: AgentMessage::assistant_text(&"B".repeat(1_000)),
                },
                timestamp: i as u64 * 2 + 1,
            });
        }
        let budget = ContextProjectionBudget {
            max_context_tokens: 1_000,
            compaction_threshold: 0.5,
            ..Default::default()
        };
        let plan = plan_compaction(&entries, &budget).unwrap();
        // The last turn starts at index 18, so cut_index should be <= 18
        assert!(plan.cut_index <= 18);
        assert!(plan.cut_index > 0);
    }

    #[test]
    fn plan_compaction_never_cuts_mid_tool() {
        let mut entries = Vec::new();
        // Turn 0: user, assistant with tool call, tool result
        entries.push(SessionEntry {
            id: "u0".to_string(),
            parent_id: None,
            kind: EntryKind::Message {
                message: AgentMessage::user(&"A".repeat(500)),
            },
            timestamp: 1,
        });
        entries.push(SessionEntry {
            id: "a0".to_string(),
            parent_id: Some("u0".to_string()),
            kind: EntryKind::Message {
                message: AgentMessage::Assistant(AssistantMessage {
                    content: vec![
                        Content::Text(TextContent {
                            text: "tool call".to_string(),
                        }),
                        Content::ToolCall(ToolCall {
                            id: ToolCallId::new("tc1"),
                            name: ToolName::new("bash"),
                            arguments: ToolArguments::new(serde_json::json!({})),
                        }),
                    ],
                    api: ApiName::new("test"),
                    provider: ProviderName::new("test"),
                    model: ModelId::new("test"),
                    stop_reason: StopReason::ToolUse,
                    error_message: None,
                    timestamp: 2,
                    usage: TokenUsage::default(),
                }),
            },
            timestamp: 2,
        });
        entries.push(SessionEntry {
            id: "t0".to_string(),
            parent_id: Some("a0".to_string()),
            kind: EntryKind::Message {
                message: AgentMessage::ToolResult(ToolResultMessage {
                    role: "tool_result".to_string(),
                    tool_call_id: ToolCallId::new("tc1"),
                    tool_name: ToolName::new("bash"),
                    content: vec![Content::Text(TextContent {
                        text: "B".repeat(500),
                    })],
                    details: None,
                    is_error: false,
                    timestamp: 3,
                }),
            },
            timestamp: 3,
        });
        // Turn 1: user, assistant
        entries.push(SessionEntry {
            id: "u1".to_string(),
            parent_id: Some("t0".to_string()),
            kind: EntryKind::Message {
                message: AgentMessage::user(&"C".repeat(500)),
            },
            timestamp: 4,
        });
        entries.push(SessionEntry {
            id: "a1".to_string(),
            parent_id: Some("u1".to_string()),
            kind: EntryKind::Message {
                message: AgentMessage::assistant_text(&"D".repeat(500)),
            },
            timestamp: 5,
        });

        let budget = ContextProjectionBudget {
            max_context_tokens: 500,
            compaction_threshold: 0.5,
            ..Default::default()
        };

        let plan = plan_compaction(&entries, &budget).unwrap();
        // The tool call is at index 1 and the tool result is at index 2.
        // The cut should never be at index 1 or 2 (which would split the pair).
        assert_ne!(plan.cut_index, 1);
        assert_ne!(plan.cut_index, 2);
    }

    #[test]
    fn apply_compaction_creates_entry() {
        let entries = vec![
            SessionEntry {
                id: "e1".to_string(),
                parent_id: None,
                kind: EntryKind::Message {
                    message: AgentMessage::user("hello"),
                },
                timestamp: 1,
            },
            SessionEntry {
                id: "e2".to_string(),
                parent_id: Some("e1".to_string()),
                kind: EntryKind::Message {
                    message: AgentMessage::assistant_text("hi"),
                },
                timestamp: 2,
            },
        ];

        let expected_tokens = estimate_tokens(&entry_messages(&entries[..1])) as u32;

        let plan = CompactionPlan {
            cut_index: 1,
            entries_to_summarize: entries[..1].to_vec(),
            tokens_to_free: expected_tokens as usize,
        };

        let result = apply_compaction(entries, plan, "summary".to_string());
        assert_eq!(result.len(), 2);
        assert!(matches!(result[0].kind, EntryKind::Compaction { .. }));
        if let EntryKind::Compaction {
            ref summary,
            ref first_kept_entry_id,
            ref tokens_before,
            ..
        } = result[0].kind
        {
            assert_eq!(summary, "summary");
            assert_eq!(first_kept_entry_id, "e2");
            assert_eq!(*tokens_before, expected_tokens);
        }
    }

    #[test]
    fn apply_compaction_replaces_old_entries() {
        let entries = vec![
            SessionEntry {
                id: "e1".to_string(),
                parent_id: None,
                kind: EntryKind::Message {
                    message: AgentMessage::user("hello"),
                },
                timestamp: 1,
            },
            SessionEntry {
                id: "e2".to_string(),
                parent_id: Some("e1".to_string()),
                kind: EntryKind::Message {
                    message: AgentMessage::assistant_text("hi"),
                },
                timestamp: 2,
            },
            SessionEntry {
                id: "e3".to_string(),
                parent_id: Some("e2".to_string()),
                kind: EntryKind::Message {
                    message: AgentMessage::user("world"),
                },
                timestamp: 3,
            },
        ];

        let plan = CompactionPlan {
            cut_index: 2,
            entries_to_summarize: entries[..2].to_vec(),
            tokens_to_free: 10,
        };

        let result = apply_compaction(entries, plan, "summary".to_string());
        let state = SessionState {
            entries: result,
            leaf_id: "e3".to_string(),
            name: "".to_string(),
        };

        let branch = state.get_branch();
        assert_eq!(branch.len(), 2);
        assert!(matches!(branch[0].kind, EntryKind::Compaction { .. }));
        assert!(matches!(branch[1].kind, EntryKind::Message { .. }));
        assert_eq!(branch[1].id, "e3");
    }

    #[test]
    fn apply_compaction_preserves_tree_integrity() {
        let entries = vec![
            SessionEntry {
                id: "e1".to_string(),
                parent_id: None,
                kind: EntryKind::Message {
                    message: AgentMessage::user("hello"),
                },
                timestamp: 1,
            },
            SessionEntry {
                id: "e2".to_string(),
                parent_id: Some("e1".to_string()),
                kind: EntryKind::Message {
                    message: AgentMessage::assistant_text("hi"),
                },
                timestamp: 2,
            },
            SessionEntry {
                id: "e3".to_string(),
                parent_id: Some("e2".to_string()),
                kind: EntryKind::Message {
                    message: AgentMessage::user("world"),
                },
                timestamp: 3,
            },
        ];

        let plan = CompactionPlan {
            cut_index: 2,
            entries_to_summarize: entries[..2].to_vec(),
            tokens_to_free: 10,
        };

        let result = apply_compaction(entries, plan, "summary".to_string());
        let id_set: std::collections::HashSet<&str> =
            result.iter().map(|e| e.id.as_str()).collect();

        for entry in &result {
            if let Some(ref pid) = entry.parent_id {
                assert!(
                    id_set.contains(pid.as_str()),
                    "parent_id {} not found for entry {}",
                    pid,
                    entry.id
                );
            }
        }
    }

    #[test]
    fn apply_compaction_accumulates_tokens_before() {
        let entries = vec![
            SessionEntry {
                id: "e1".to_string(),
                parent_id: None,
                kind: EntryKind::Message {
                    message: AgentMessage::user("hello"),
                },
                timestamp: 1,
            },
            SessionEntry {
                id: "e2".to_string(),
                parent_id: Some("e1".to_string()),
                kind: EntryKind::Message {
                    message: AgentMessage::assistant_text("hi there friend"),
                },
                timestamp: 2,
            },
            SessionEntry {
                id: "e3".to_string(),
                parent_id: Some("e2".to_string()),
                kind: EntryKind::Message {
                    message: AgentMessage::user("world"),
                },
                timestamp: 3,
            },
        ];

        let summarized = entries[..2].to_vec();
        let expected_tokens = estimate_tokens(&entry_messages(&summarized));

        let plan = CompactionPlan {
            cut_index: 2,
            entries_to_summarize: summarized,
            tokens_to_free: expected_tokens,
        };

        let result = apply_compaction(entries, plan, "summary".to_string());
        if let EntryKind::Compaction { tokens_before, .. } = result[0].kind {
            assert_eq!(tokens_before as usize, expected_tokens);
        } else {
            panic!("expected compaction entry");
        }
    }

    #[test]
    fn plan_compaction_split_turn() {
        let mut entries = Vec::new();
        // Turn 0: small
        entries.push(SessionEntry {
            id: "u0".to_string(),
            parent_id: None,
            kind: EntryKind::Message {
                message: AgentMessage::user(&"A".repeat(100)),
            },
            timestamp: 1,
        });
        entries.push(SessionEntry {
            id: "a0".to_string(),
            parent_id: Some("u0".to_string()),
            kind: EntryKind::Message {
                message: AgentMessage::assistant_text(&"B".repeat(100)),
            },
            timestamp: 2,
        });
        // Turn 1: huge, exceeds budget on its own
        entries.push(SessionEntry {
            id: "u1".to_string(),
            parent_id: Some("a0".to_string()),
            kind: EntryKind::Message {
                message: AgentMessage::user(&"C".repeat(10_000)),
            },
            timestamp: 3,
        });
        entries.push(SessionEntry {
            id: "a1".to_string(),
            parent_id: Some("u1".to_string()),
            kind: EntryKind::Message {
                message: AgentMessage::assistant_text(&"D".repeat(10_000)),
            },
            timestamp: 4,
        });

        let budget = ContextProjectionBudget {
            max_context_tokens: 1_000,
            compaction_threshold: 0.5,
            ..Default::default()
        };

        let plan = plan_compaction(&entries, &budget).unwrap();
        // The last turn alone exceeds budget, but we should still keep it
        // and summarize earlier turns.
        assert_eq!(plan.cut_index, 2);
        assert!(plan.tokens_to_free > 0);
    }

    #[test]
    fn apply_compaction_idempotent_branch() {
        let entries = vec![
            SessionEntry {
                id: "e1".to_string(),
                parent_id: None,
                kind: EntryKind::Message {
                    message: AgentMessage::user("hello"),
                },
                timestamp: 1,
            },
            SessionEntry {
                id: "e2".to_string(),
                parent_id: Some("e1".to_string()),
                kind: EntryKind::Message {
                    message: AgentMessage::assistant_text("hi"),
                },
                timestamp: 2,
            },
            SessionEntry {
                id: "e3".to_string(),
                parent_id: Some("e2".to_string()),
                kind: EntryKind::Message {
                    message: AgentMessage::user("world"),
                },
                timestamp: 3,
            },
            SessionEntry {
                id: "e4".to_string(),
                parent_id: Some("e3".to_string()),
                kind: EntryKind::Message {
                    message: AgentMessage::assistant_text("yes"),
                },
                timestamp: 4,
            },
        ];

        let plan = CompactionPlan {
            cut_index: 2,
            entries_to_summarize: entries[..2].to_vec(),
            tokens_to_free: 10,
        };

        let result = apply_compaction(entries.clone(), plan, "summary".to_string());
        // The kept entries (e3, e4) should have the same kind and id as before
        assert_eq!(result[1].id, "e3");
        assert_eq!(result[2].id, "e4");
        assert_eq!(result[1].kind, entries[2].kind);
        assert_eq!(result[2].kind, entries[3].kind);
    }

    #[test]
    fn entry_kind_compaction_serialize_roundtrip() {
        let entry = SessionEntry {
            id: "c1".to_string(),
            parent_id: None,
            kind: EntryKind::Compaction {
                summary: "test summary".to_string(),
                first_kept_entry_id: "e1".to_string(),
                tokens_before: 42,
                details: None,
            },
            timestamp: 1,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let back: SessionEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(entry, back);
    }
}
