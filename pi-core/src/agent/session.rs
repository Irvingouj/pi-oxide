use super::Agent;
use crate::message::AgentMessage;
use crate::session::{BranchSummary, EntryKind, SessionEntry, SessionState};

impl Agent {
    /// Get the current branch (root to leaf) as cloned entries.
    pub fn session_branch(&self, session_state: &SessionState) -> Vec<SessionEntry> {
        session_state
            .get_branch()
            .into_iter()
            .cloned()
            .collect()
    }

    /// Move the leaf to a target entry, optionally creating a branch summary.
    pub fn move_to(&mut self, session_state: &mut SessionState, target_id: &str, summary: Option<BranchSummary>) -> Option<String> {
        session_state.move_to(target_id, summary)
    }

    /// Append a custom entry to the session tree.
    pub fn append_session_entry(&mut self, session_state: &mut SessionState, entry: SessionEntry) {
        session_state.leaf_id = entry.id.clone();
        session_state.entries.push(entry);
    }

    /// Append a message to the session tree as an EntryKind::Message.
    pub(crate) fn append_session_message(&mut self, session_state: &mut SessionState, message: &AgentMessage) {
        let id = format!("entry-{}", session_state.entries.len());
        let parent_id = session_state.entries.last().map(|e| e.id.clone());
        let entry = SessionEntry {
            id,
            parent_id,
            kind: EntryKind::Message {
                message: message.clone(),
            },
            timestamp: super::current_timestamp(),
        };
        session_state.leaf_id = entry.id.clone();
        session_state.entries.push(entry);
    }
}
