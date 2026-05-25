use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{debug, trace};

use crate::events::ThinkingLevel;
use crate::message::AgentMessage;

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
        trace!(entries = state.entries.len(), "session state built from messages");
        state
    }

    /// Get the full branch from root to leaf.
    pub fn get_branch(&self) -> Vec<&SessionEntry> {
        let mut branch = Vec::new();
        let mut current = self.entries.iter().find(|e| e.id == self.leaf_id);
        trace!(leaf_id = self.leaf_id, total_entries = self.entries.len(), "get_branch called");

        while let Some(entry) = current {
            branch.push(entry);
            if let Some(pid) = &entry.parent_id {
                current = self.entries.iter().find(|e| e.id == *pid);
            } else {
                break;
            }
        }

        branch.reverse();
        branch
    }

    /// Build the LLM-visible context from the current branch.
    pub fn build_context(&self) -> Vec<AgentMessage> {
        let ctx = self.get_branch()
            .iter()
            .filter_map(|e| match &e.kind {
                EntryKind::Message { message } => Some(message.clone()),
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

fn current_timestamp() -> u64 {
    crate::timestamp::current_timestamp()
}
