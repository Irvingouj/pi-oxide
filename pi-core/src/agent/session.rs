use super::Agent;
use crate::message::AgentMessage;
use crate::session::{BranchSummary, EntryKind, SessionEntry};

impl Agent {
    /// Get the current branch (root to leaf) as cloned entries.
    pub fn session_branch(&self) -> Vec<SessionEntry> {
        self.session_state
            .get_branch()
            .into_iter()
            .cloned()
            .collect()
    }

    /// Move the leaf to a target entry, optionally creating a branch summary.
    pub fn move_to(&mut self, target_id: &str, summary: Option<BranchSummary>) -> Option<String> {
        self.session_state.move_to(target_id, summary)
    }

    /// Append a custom entry to the session tree.
    pub fn append_session_entry(&mut self, entry: SessionEntry) {
        self.session_state.leaf_id = entry.id.clone();
        self.session_state.entries.push(entry);
    }

    /// Append a message to the session tree as an EntryKind::Message.
    pub(crate) fn append_session_message(&mut self, message: &AgentMessage) {
        let id = format!("entry-{}", self.session_state.entries.len());
        let parent_id = self.session_state.entries.last().map(|e| e.id.clone());
        let entry = SessionEntry {
            id,
            parent_id,
            kind: EntryKind::Message {
                message: message.clone(),
            },
            timestamp: super::current_timestamp(),
        };
        self.session_state.leaf_id = entry.id.clone();
        self.session_state.entries.push(entry);
    }
}
