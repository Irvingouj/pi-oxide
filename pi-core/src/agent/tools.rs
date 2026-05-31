use super::Agent;
use crate::context_projection::projection_scan;
use crate::events::{AgentAction, AgentEvent, CancelReason, ToolExecutionUpdate};
use crate::message::{
    AgentMessage, Artifacts, Content, OriginalToolResult, TextContent, ToolResultMessage,
    TrimmedMessage,
};
use crate::tool::{ToolError, ToolResult};
use crate::types::ToolCallId;
use tracing::{debug, trace, warn};

impl Agent {
    /// Shared logic when all pending tools have completed or been cancelled.
    ///
    /// Takes completed_tool_results, finds last assistant, emits TurnEnd,
    /// runs projection_scan. Returns the scan markers.
    fn finalize_tool_batch(
        &mut self,
        t: &mut Vec<TrimmedMessage>,
        a: &mut Artifacts,
        turn_number: u32,
        events: &mut Vec<AgentEvent>,
        fallback_msg: AgentMessage,
    ) -> Vec<crate::context_projection::ChangeMarker> {
        let tool_results = std::mem::take(&mut self.completed_tool_results);
        self.completed_tool_terminations.clear();

        let assistant_msg = t
            .iter()
            .rev()
            .find(|msg| matches!(msg, TrimmedMessage::Assistant(_)))
            .cloned()
            .map(|m| match m {
                TrimmedMessage::Assistant(a) => AgentMessage::Assistant(a),
                _ => unreachable!(),
            })
            .unwrap_or(fallback_msg);

        events.push(AgentEvent::TurnEnd {
            message: assistant_msg,
            tool_results,
        });

        debug!("finalizing tool batch — running projection_scan");
        projection_scan(t, a, turn_number)
    }

    /// Called by the host when a tool finishes executing.
    ///
    /// Creates an OriginalToolResult, pushes to T. When all tools are done,
    /// runs projection_scan.
    pub(crate) fn on_tool_done(
        &mut self,
        tool_call_id: ToolCallId,
        result: Result<ToolResult, ToolError>,
        mut t: Vec<TrimmedMessage>,
        mut a: Artifacts,
        turn_number: u32,
    ) -> (
        Vec<AgentEvent>,
        Vec<AgentAction>,
        Vec<crate::context_projection::ChangeMarker>,
        Vec<TrimmedMessage>,
        Artifacts,
    ) {
        let tool_call = match self.pending_tool_calls.remove(&tool_call_id) {
            Some(tc) => tc,
            None => {
                warn!(
                    tool_call_id = tool_call_id.as_str(),
                    "unknown tool completion ignored"
                );
                return (vec![], vec![], vec![], t, a);
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

        let entry_id = self.next_entry_id();

        let original_tool = OriginalToolResult {
            entry_id: entry_id.clone(),
            tool_call_id: tool_call_id.clone(),
            tool_name: tool_call.name.clone(),
            content: tool_result.content.clone(),
            is_error,
            turn: turn_number,
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

        self.completed_tool_terminations
            .push(tool_result.terminate == Some(true));
        self.completed_tool_results.push(result_msg.clone());

        // Push OriginalTool to T
        t.push(TrimmedMessage::OriginalTool(original_tool));

        let mut events = Vec::new();
        let actions = Vec::new();
        let mut markers = Vec::new();

        events.push(AgentEvent::ToolExecutionEnd {
            tool_call_id: result_msg.tool_call_id.clone(),
            result: tool_result.clone(),
            is_error: result_msg.is_error,
        });

        let agent_msg = AgentMessage::ToolResult(result_msg.clone());
        events.push(AgentEvent::MessageStart {
            message: agent_msg.clone(),
        });
        events.push(AgentEvent::MessageEnd { message: agent_msg });

        if !self.pending_tool_calls.is_empty() {
            trace!(
                pending_tool_calls = self.pending_tool_calls.len(),
                "waiting for remaining tools"
            );
            return (events, actions, markers, t, a);
        }

        // All tools done — check termination
        let should_terminate = !self.completed_tool_terminations.is_empty()
            && self.completed_tool_terminations.iter().all(|t| *t);

        let fallback = AgentMessage::ToolResult(ToolResultMessage {
            role: "tool_result".to_string(),
            tool_call_id: tool_call_id.clone(),
            tool_name: tool_call.name.clone(),
            content: vec![],
            details: None,
            is_error: false,
            timestamp: 0,
        });
        let scan_markers =
            self.finalize_tool_batch(&mut t, &mut a, turn_number, &mut events, fallback);
        markers.extend(scan_markers);

        if should_terminate {
            self.phase = super::Phase::Idle;
            events.push(AgentEvent::AgentEnd);
            return (events, vec![AgentAction::Finished], markers, t, a);
        }

        self.phase = super::Phase::WaitForInput;
        (events, actions, markers, t, a)
    }

    /// Called by the host when a tool is cancelled.
    pub(crate) fn on_tool_cancelled(
        &mut self,
        tool_call_id: ToolCallId,
        reason: CancelReason,
        mut t: Vec<TrimmedMessage>,
        mut a: Artifacts,
        turn_number: u32,
    ) -> (
        Vec<AgentEvent>,
        Vec<AgentAction>,
        Vec<crate::context_projection::ChangeMarker>,
        Vec<TrimmedMessage>,
        Artifacts,
    ) {
        let tool_call = match self.pending_tool_calls.remove(&tool_call_id) {
            Some(tc) => tc,
            None => {
                warn!(
                    tool_call_id = tool_call_id.as_str(),
                    "on_tool_cancelled for unknown tool"
                );
                return (vec![], vec![], vec![], t, a);
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

        let entry_id = self.next_entry_id();

        let original_tool = OriginalToolResult {
            entry_id: entry_id.clone(),
            tool_call_id: tool_call_id.clone(),
            tool_name: tool_call.name.clone(),
            content: vec![Content::Text(TextContent {
                text: format!("Tool execution was cancelled: {}", reason_str),
            })],
            is_error: true,
            turn: turn_number,
        };

        let result_msg = ToolResultMessage {
            role: "tool_result".to_string(),
            tool_call_id: tool_call_id.clone(),
            tool_name: tool_call.name.clone(),
            content: original_tool.content.clone(),
            details: None,
            is_error: true,
            timestamp: super::current_timestamp(),
        };

        let mut events = vec![AgentEvent::ToolExecutionCancelled {
            tool_call_id: tool_call_id.clone(),
            reason,
        }];

        // Push OriginalTool to T
        t.push(TrimmedMessage::OriginalTool(original_tool));

        events.push(AgentEvent::ToolExecutionEnd {
            tool_call_id: result_msg.tool_call_id.clone(),
            result: ToolResult {
                content: result_msg.content.clone(),
                details: None,
                terminate: None,
            },
            is_error: result_msg.is_error,
        });

        let agent_msg = AgentMessage::ToolResult(result_msg);
        events.push(AgentEvent::MessageStart {
            message: agent_msg.clone(),
        });
        events.push(AgentEvent::MessageEnd { message: agent_msg });

        if !self.pending_tool_calls.is_empty() {
            return (events, vec![], vec![], t, a);
        }

        // All tools done after cancel
        let scan_markers = self.finalize_tool_batch(
            &mut t,
            &mut a,
            turn_number,
            &mut events,
            AgentMessage::user("cancelled"),
        );

        self.phase = super::Phase::WaitForInput;
        (events, vec![], scan_markers, t, a)
    }

    /// Called by the host when a tool starts executing.
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
