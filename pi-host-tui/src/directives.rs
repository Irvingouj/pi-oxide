//! Internal directives used by the TUI host to coordinate between
//! the core [`AgentAction`] stream and host-side behaviour.

use pi_core::LlmContext;

/// Directive emitted after an agent transition, telling the host what to do next.
#[derive(Debug, Clone, PartialEq)]
pub enum HostDirective {
    /// Stream an LLM response (normal turn or summarization).
    StreamLlm { context: LlmContext },
    /// Summarize the conversation (compaction-triggered stream).
    Summarize { context: LlmContext },
    /// Execute tool calls returned by the LLM.
    ExecuteTools { calls: Vec<pi_core::ToolCall> },
    /// Cancel in-flight tool calls.
    CancelTools {
        tool_call_ids: Vec<pi_core::ToolCallId>,
        reason: pi_core::CancelReason,
    },
    /// Persist the current session to disk.
    Persist,
    /// The agent turn is finished.
    Finished,
    /// Wait for user input before continuing.
    WaitForInput { mode: pi_core::WaitMode },
}
