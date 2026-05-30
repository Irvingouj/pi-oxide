use super::Agent;
use crate::events::{AgentAction, AgentEvent, CancelReason, ToolExecutionUpdate};
use crate::message::{AgentMessage, Content, TextContent, ToolResultMessage};
use crate::session::SessionState;
use crate::tool::{ToolError, ToolResult};
use crate::types::ToolCallId;
use tracing::{debug, trace, warn};

impl Agent {
    /// Shared completion logic after a tool call is removed from pending.
    /// `terminate` is `Some(bool)` from the tool result; `None` for cancellations (never terminates).
    fn resolve_tool_completion(
        &mut self,
        result_msg: ToolResultMessage,
        tool_result: ToolResult,
        pre_events: Vec<AgentEvent>,
        terminate: Option<bool>,
        session_state: &mut SessionState,
    ) -> (Vec<AgentEvent>, Vec<AgentAction>) {
        let mut events = pre_events;

        events.push(AgentEvent::ToolExecutionEnd {
            tool_call_id: result_msg.tool_call_id.clone(),
            result: tool_result,
            is_error: result_msg.is_error,
        });

        let agent_msg = AgentMessage::ToolResult(result_msg);
        if let AgentMessage::ToolResult(msg) = &agent_msg {
            self.completed_tool_terminations
                .push(terminate == Some(true));
            self.completed_tool_results.push(msg.clone());
        }
        self.state.messages.push(agent_msg.clone());
        self.append_session_message(session_state, &agent_msg);
        events.push(AgentEvent::MessageStart {
            message: agent_msg.clone(),
        });
        events.push(AgentEvent::MessageEnd {
            message: agent_msg.clone(),
        });

        if !self.pending_tool_calls.is_empty() {
            trace!(
                pending_tool_calls = self.pending_tool_calls.len(),
                "waiting for remaining tools"
            );
            return (events, vec![]);
        }

        let assistant_msg = self
            .state
            .messages
            .iter()
            .rev()
            .find(|msg| matches!(msg, AgentMessage::Assistant(_)))
            .cloned()
            .unwrap_or_else(|| agent_msg.clone());
        let tool_results = std::mem::take(&mut self.completed_tool_results);
        let should_terminate = !self.completed_tool_terminations.is_empty()
            && self.completed_tool_terminations.iter().all(|t| *t);
        self.completed_tool_terminations.clear();

        events.push(AgentEvent::TurnEnd {
            message: assistant_msg,
            tool_results,
        });

        if should_terminate {
            debug!("tool batch requested termination");
            self.phase = super::Phase::Idle;
            events.push(AgentEvent::AgentEnd {
                messages: self.state.messages.clone(),
            });
            return (
                events,
                vec![AgentAction::Finished {
                    messages: self.state.messages.clone(),
                }],
            );
        }

        self.phase = super::Phase::WaitForInput;
        (events, vec![])
    }

    /// Called by the host when a tool finishes executing.
    pub(crate) fn on_tool_done(
        &mut self,
        tool_call_id: ToolCallId,
        result: Result<ToolResult, ToolError>,
        session_state: &mut SessionState,
    ) -> (Vec<AgentEvent>, Vec<AgentAction>) {
        let tool_call = match self.pending_tool_calls.remove(&tool_call_id) {
            Some(tc) => tc,
            None => {
                warn!(
                    tool_call_id = tool_call_id.as_str(),
                    "unknown tool completion ignored"
                );
                return (vec![], vec![]);
            }
        };
        self.state
            .pending_tool_calls
            .retain(|id| id != tool_call_id.as_str());

        let (tool_result, is_error) = match result {
            Ok(r) => (r, false),
            Err(e) => (
                ToolResult {
                    content: vec![Content::Text(TextContent {
                        text: e.message.clone(),
                    })],
                    details: None,
                    terminate: None,
                },
                true,
            ),
        };

        let result_msg = ToolResultMessage {
            role: "tool_result".to_string(),
            tool_call_id: tool_call_id.clone(),
            tool_name: tool_call.name.clone(),
            content: tool_result.content.clone(),
            details: tool_result.details.clone(),
            is_error,
            timestamp: super::current_timestamp(),
        };

        self.resolve_tool_completion(
            result_msg,
            tool_result.clone(),
            vec![],
            tool_result.terminate,
            session_state,
        )
    }

    /// Called by the host when a tool is cancelled.
    /// Treats cancellation like a tool error result so the state machine can advance.
    pub(crate) fn on_tool_cancelled(
        &mut self,
        tool_call_id: ToolCallId,
        reason: CancelReason,
        session_state: &mut SessionState,
    ) -> (Vec<AgentEvent>, Vec<AgentAction>) {
        let tool_call = match self.pending_tool_calls.remove(&tool_call_id) {
            Some(tc) => tc,
            None => {
                warn!(
                    tool_call_id = tool_call_id.as_str(),
                    "on_tool_cancelled for unknown tool"
                );
                return (vec![], vec![]);
            }
        };
        self.state
            .pending_tool_calls
            .retain(|id| id != tool_call_id.as_str());

        let reason_str = match &reason {
            CancelReason::UserRequested => "cancelled by user".to_string(),
            CancelReason::Timeout => "cancelled due to timeout".to_string(),
            CancelReason::AgentAborted => "cancelled due to agent abort".to_string(),
            CancelReason::DependencyFailed { cause_tool_call_id } => {
                format!(
                    "cancelled because dependency {} failed",
                    cause_tool_call_id.as_str()
                )
            }
        };

        let result_msg = ToolResultMessage {
            role: "tool_result".to_string(),
            tool_call_id: tool_call_id.clone(),
            tool_name: tool_call.name.clone(),
            content: vec![Content::Text(TextContent {
                text: format!("Tool execution was cancelled: {}", reason_str),
            })],
            details: None,
            is_error: true,
            timestamp: super::current_timestamp(),
        };

        let pre_events = vec![AgentEvent::ToolExecutionCancelled {
            tool_call_id: tool_call_id.clone(),
            reason,
        }];

        let tool_result = ToolResult {
            content: result_msg.content.clone(),
            details: None,
            terminate: None,
        };

        self.resolve_tool_completion(result_msg, tool_result, pre_events, None, session_state)
    }

    /// Called by the host when a tool starts executing.
    /// Emits a ToolExecutionUpdate event for trace/observability.
    /// Does not change the core state machine phase.
    pub(crate) fn on_tool_started(&mut self, tool_call_id: ToolCallId) -> Vec<AgentEvent> {
        if !self.pending_tool_calls.contains_key(&tool_call_id) {
            trace!(
                tool_call_id = tool_call_id.as_str(),
                "on_tool_started for unknown tool"
            );
            return vec![];
        }
        trace!(
            tool_call_id = tool_call_id.as_str(),
            "tool execution started"
        );
        vec![AgentEvent::ToolExecutionUpdate {
            tool_call_id,
            stream: crate::events::ToolOutputStream::Status,
            chunk: "[started]".to_string(),
            sequence: 0,
            timestamp: super::current_timestamp(),
        }]
    }

    /// Called by the host with a streaming chunk from a running tool.
    /// Emits a ToolExecutionUpdate event for trace/observability only.
    /// Does NOT add to the canonical model transcript.
    pub(crate) fn on_tool_update(&mut self, update: ToolExecutionUpdate) -> Vec<AgentEvent> {
        if !self.pending_tool_calls.contains_key(&update.tool_call_id) {
            return vec![];
        }
        trace!(
            tool_call_id = update.tool_call_id.as_str(),
            stream = ?update.stream,
            seq = update.sequence,
            bytes = update.chunk.len(),
            "tool execution update"
        );
        vec![AgentEvent::ToolExecutionUpdate {
            tool_call_id: update.tool_call_id,
            stream: update.stream,
            chunk: update.chunk,
            sequence: update.sequence,
            timestamp: update.timestamp,
        }]
    }
}
