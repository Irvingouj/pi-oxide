use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use ts_rs::TS;

use crate::context::LlmContext;
use crate::events::{
    AgentAction, AgentEvent, CancelReason, ContentDelta, QueueMode, ThinkingLevel,
    ToolExecutionUpdate, WaitMode,
};
use crate::llm::{LlmChunk, LlmResult, Model};
use crate::message::{AgentMessage, Content, StopReason, ToolCall, ToolResultMessage};
use crate::session::{BranchSummary, EntryKind, SessionEntry, SessionState};
use crate::tool::{ToolDefinition, ToolError, ToolExecutionMode, ToolResult};
use crate::types::{SessionId, ToolCallId};
use tracing::{debug, trace, warn};

/// Public agent state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AgentState {
    pub system_prompt: String,
    pub model: Model,
    pub thinking_level: ThinkingLevel,
    pub tools: Vec<ToolDefinition>,
    pub messages: Vec<AgentMessage>,
    pub is_streaming: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub streaming_message: Option<AgentMessage>,
    pub pending_tool_calls: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// Options for constructing an Agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct AgentOptions {
    pub system_prompt: String,
    pub model: Model,
    #[serde(default)]
    pub thinking_level: ThinkingLevel,
    #[serde(default)]
    pub tools: Vec<ToolDefinition>,
    #[serde(default)]
    pub steering_mode: QueueMode,
    #[serde(default)]
    pub follow_up_mode: QueueMode,
    #[serde(default)]
    pub tool_execution_mode: ToolExecutionMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    #[serde(default)]
    pub messages: Vec<AgentMessage>,
    #[serde(default)]
    #[ts(skip)]
    pub session_state: Option<SessionState>,
}

/// Internal phase of the agent state machine.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Idle,
    Streaming,
    WaitForInput,
}

/// The agent state machine. Purely synchronous; the host drives progress.
#[derive(Clone)]
pub struct Agent {
    state: AgentState,
    steering_queue: Vec<AgentMessage>,
    follow_up_queue: Vec<AgentMessage>,
    pub phase: Phase,
    pending_tool_calls: HashMap<ToolCallId, ToolCall>,
    completed_tool_results: Vec<ToolResultMessage>,
    completed_tool_terminations: Vec<bool>,
    steering_mode: QueueMode,
    follow_up_mode: QueueMode,
    #[allow(dead_code)]
    tool_execution_mode: ToolExecutionMode,
    session_id: Option<SessionId>,
    session_state: SessionState,
}

impl Agent {
    pub fn new(options: AgentOptions) -> Self {
        let mut session_state = options.session_state.unwrap_or_default();
        if session_state.entries.is_empty() && !options.messages.is_empty() {
            session_state = SessionState::from_messages(&options.messages);
        }
        Self {
            state: AgentState {
                system_prompt: options.system_prompt,
                model: options.model,
                thinking_level: options.thinking_level,
                tools: options.tools,
                messages: options.messages,
                is_streaming: false,
                streaming_message: None,
                pending_tool_calls: Vec::new(),
                error_message: None,
            },
            steering_queue: Vec::new(),
            follow_up_queue: Vec::new(),
            phase: Phase::Idle,
            pending_tool_calls: HashMap::new(),
            completed_tool_results: Vec::new(),
            completed_tool_terminations: Vec::new(),
            steering_mode: options.steering_mode,
            follow_up_mode: options.follow_up_mode,
            tool_execution_mode: options.tool_execution_mode,
            session_id: options.session_id,
            session_state,
        }
    }

    /// Start processing a new prompt.
    pub(crate) fn start_turn(
        &mut self,
        prompt: AgentMessage,
    ) -> (Vec<AgentEvent>, Vec<AgentAction>) {
        if self.phase == Phase::Streaming {
            warn!(phase = ?self.phase, "start_turn requested while LLM is streaming");
            return (
                vec![AgentEvent::AgentStart],
                vec![AgentAction::WaitForInput {
                    mode: WaitMode::Any,
                }],
            );
        }

        self.state.messages.push(prompt.clone());
        self.append_session_message(&prompt);
        debug!(
            message_count = self.state.messages.len(),
            "agent turn started"
        );
        let events = vec![
            AgentEvent::AgentStart,
            AgentEvent::TurnStart,
            AgentEvent::MessageStart { message: prompt },
            AgentEvent::MessageEnd {
                message: self.state.messages.last().unwrap().clone(),
            },
        ];

        self.phase = Phase::Streaming;
        self.state.is_streaming = true;

        let actions = vec![AgentAction::StreamLlm {
            context: self.build_llm_context(),
            session_id: self.session_id.clone(),
        }];

        (events, actions)
    }

    /// Continue from the current transcript without adding a new message.
    pub(crate) fn continue_turn(&mut self) -> (Vec<AgentEvent>, Vec<AgentAction>) {
        if self.phase == Phase::Streaming {
            warn!(phase = ?self.phase, "continue_turn requested while LLM is streaming");
            return (vec![], vec![]);
        }

        let last = self.state.messages.last();
        if let Some(AgentMessage::Assistant(_)) = last {
            // Check steering queue first
            let drained = self.drain_steering();
            if !drained.is_empty() {
                return self.inject_messages_and_stream(drained);
            }

            let follow = self.drain_follow_up();
            if !follow.is_empty() {
                return self.inject_messages_and_stream(follow);
            }

            return (
                vec![],
                vec![AgentAction::WaitForInput {
                    mode: WaitMode::Any,
                }],
            );
        }

        self.phase = Phase::Streaming;
        self.state.is_streaming = true;

        let events = vec![AgentEvent::AgentStart, AgentEvent::TurnStart];
        let actions = vec![AgentAction::StreamLlm {
            context: self.build_llm_context(),
            session_id: self.session_id.clone(),
        }];

        (events, actions)
    }

    /// Feed a streaming chunk from the LLM.
    pub(crate) fn feed_llm_chunk(&mut self, chunk: LlmChunk) -> Vec<AgentEvent> {
        if self.phase != Phase::Streaming {
            trace!(phase = ?self.phase, "ignored llm chunk outside streaming phase");
            return vec![];
        }

        let mut events = Vec::new();

        match chunk {
            LlmChunk::Start { mut partial } => {
                trace!("llm stream started");
                partial.timestamp = current_timestamp();
                let msg = AgentMessage::Assistant(partial.clone());
                self.state.messages.push(msg.clone());
                self.state.streaming_message = Some(msg.clone());
                events.push(AgentEvent::MessageStart { message: msg });
            }
            LlmChunk::TextDelta { text } => {
                trace!(bytes = text.len(), "llm text delta");
                if let Some(AgentMessage::Assistant(ref mut a)) = self.state.messages.last_mut() {
                    let delta_text = text.clone();
                    if let Some(Content::Text(ref mut t)) = a.content.last_mut() {
                        t.text.push_str(&text);
                    } else {
                        a.content
                            .push(Content::Text(crate::message::TextContent { text }));
                    }
                    let msg = AgentMessage::Assistant(a.clone());
                    self.state.streaming_message = Some(msg.clone());
                    events.push(AgentEvent::MessageUpdate {
                        message: msg.clone(),
                        delta: ContentDelta::TextDelta { text: delta_text },
                    });
                }
            }
            LlmChunk::ToolCallDelta {
                tool_call_id,
                delta,
            } => {
                if let Some(AgentMessage::Assistant(ref mut a)) = self.state.messages.last_mut() {
                    events.push(AgentEvent::MessageUpdate {
                        message: AgentMessage::Assistant(a.clone()),
                        delta: ContentDelta::ToolCallDelta {
                            tool_call_id,
                            delta,
                        },
                    });
                }
            }
            LlmChunk::ThinkingDelta { text } => {
                if let Some(AgentMessage::Assistant(ref mut a)) = self.state.messages.last_mut() {
                    events.push(AgentEvent::MessageUpdate {
                        message: AgentMessage::Assistant(a.clone()),
                        delta: ContentDelta::ThinkingDelta { text },
                    });
                }
            }
            LlmChunk::Done | LlmChunk::Error { .. } => {
                // Handled in on_llm_done
            }
        }

        events
    }

    /// Called by the host when the LLM stream ends.
    pub(crate) fn on_llm_done(&mut self, result: LlmResult) -> (Vec<AgentEvent>, Vec<AgentAction>) {
        if self.phase != Phase::Streaming {
            warn!(phase = ?self.phase, "on_llm_done requested outside streaming phase");
            return (vec![], vec![]);
        }

        let mut events = Vec::new();
        let mut actions = Vec::new();

        let assistant_msg = result.finalize_message();
        let msg = AgentMessage::Assistant(assistant_msg.clone());

        // Update the last message (which may have been a streaming partial)
        if let Some(AgentMessage::Assistant(_)) = self.state.messages.last() {
            *self.state.messages.last_mut().unwrap() = msg.clone();
        } else {
            self.state.messages.push(msg.clone());
        }

        self.state.streaming_message = None;
        self.append_session_message(&msg);
        events.push(AgentEvent::MessageEnd {
            message: msg.clone(),
        });

        // Check for error / abort
        if matches!(
            assistant_msg.stop_reason,
            StopReason::Error | StopReason::Aborted
        ) {
            warn!(
                stop_reason = ?assistant_msg.stop_reason,
                error = ?assistant_msg.error_message,
                "llm stream ended with failure"
            );
            self.state.error_message = assistant_msg.error_message.clone();
            events.push(AgentEvent::TurnEnd {
                message: msg,
                tool_results: vec![],
            });
            events.push(AgentEvent::AgentEnd {
                messages: self.state.messages.clone(),
            });
            self.phase = Phase::Idle;
            self.state.is_streaming = false;
            actions.push(AgentAction::Finished {
                messages: self.state.messages.clone(),
            });
            return (events, actions);
        }

        // Check for tool calls
        let tool_calls: Vec<ToolCall> = assistant_msg
            .content
            .iter()
            .filter_map(|c| match c {
                Content::ToolCall(tc) => Some(tc.clone()),
                _ => None,
            })
            .collect();

        if tool_calls.is_empty() {
            debug!("assistant turn finished without tool calls");
            // No tools: turn ends
            events.push(AgentEvent::TurnEnd {
                message: msg,
                tool_results: vec![],
            });

            // Check steering / follow-up queues
            let steering = self.drain_steering();
            if !steering.is_empty() {
                return self.inject_messages_and_stream(steering);
            }

            let follow = self.drain_follow_up();
            if !follow.is_empty() {
                return self.inject_messages_and_stream(follow);
            }

            // Run complete
            events.push(AgentEvent::AgentEnd {
                messages: self.state.messages.clone(),
            });
            self.phase = Phase::Idle;
            self.state.is_streaming = false;
            actions.push(AgentAction::Finished {
                messages: self.state.messages.clone(),
            });
            return (events, actions);
        }

        // Track all tools uniformly — execution strategy is a host concern
        for tc in &tool_calls {
            self.pending_tool_calls.insert(tc.id.clone(), tc.clone());
            self.state.pending_tool_calls.push(tc.id.0.clone());
            events.push(AgentEvent::ToolExecutionStart {
                tool_call_id: tc.id.clone(),
                tool_name: tc.name.clone(),
                args: Some(tc.arguments.clone()),
            });
        }

        self.phase = Phase::Idle;
        self.state.is_streaming = false;
        debug!(
            tool_count = tool_calls.len(),
            "assistant requested tool execution"
        );
        actions.push(AgentAction::ExecuteTools { calls: tool_calls });
        (events, actions)
    }

    /// Called by the host when a tool finishes executing.
    pub(crate) fn on_tool_done(
        &mut self,
        tool_call_id: ToolCallId,
        result: Result<ToolResult, ToolError>,
    ) -> (Vec<AgentEvent>, Vec<AgentAction>) {
        let mut events = Vec::new();
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
                    content: vec![Content::Text(crate::message::TextContent {
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
            timestamp: current_timestamp(),
        };

        events.push(AgentEvent::ToolExecutionEnd {
            tool_call_id: tool_call_id.clone(),
            result: tool_result.clone(),
            is_error,
        });

        let agent_msg = AgentMessage::ToolResult(result_msg);
        if let AgentMessage::ToolResult(msg) = &agent_msg {
            self.completed_tool_terminations
                .push(tool_result.terminate == Some(true));
            self.completed_tool_results.push(msg.clone());
        }
        self.state.messages.push(agent_msg.clone());
        self.append_session_message(&agent_msg);
        events.push(AgentEvent::MessageStart {
            message: agent_msg.clone(),
        });
        events.push(AgentEvent::MessageEnd {
            message: agent_msg.clone(),
        });

        // If more tools are still pending, just return events
        if !self.pending_tool_calls.is_empty() {
            trace!(
                pending_tool_calls = self.pending_tool_calls.len(),
                "waiting for remaining tools"
            );
            return (events, vec![]);
        }

        // All tools done: emit TurnEnd. Host should call continue_turn() to stream next LLM response.
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
            && self
                .completed_tool_terminations
                .iter()
                .all(|terminate| *terminate);
        self.completed_tool_terminations.clear();

        events.push(AgentEvent::TurnEnd {
            message: assistant_msg,
            tool_results,
        });

        if should_terminate {
            debug!("tool batch requested termination");
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

        self.phase = Phase::WaitForInput;
        (events, vec![])
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
            timestamp: current_timestamp(),
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

    /// Called by the host when a tool is cancelled.
    /// Treats cancellation like a tool error result so the state machine can advance.
    pub(crate) fn on_tool_cancelled(
        &mut self,
        tool_call_id: ToolCallId,
        reason: CancelReason,
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
            content: vec![Content::Text(crate::message::TextContent {
                text: format!("Tool execution was cancelled: {}", reason_str),
            })],
            details: None,
            is_error: true,
            timestamp: current_timestamp(),
        };

        let mut events = vec![
            AgentEvent::ToolExecutionCancelled {
                tool_call_id: tool_call_id.clone(),
                reason,
            },
            AgentEvent::ToolExecutionEnd {
                tool_call_id: tool_call_id.clone(),
                result: ToolResult {
                    content: result_msg.content.clone(),
                    details: None,
                    terminate: None,
                },
                is_error: true,
            },
        ];

        let agent_msg = AgentMessage::ToolResult(result_msg);
        self.completed_tool_results.push(match &agent_msg {
            AgentMessage::ToolResult(m) => m.clone(),
            _ => unreachable!(),
        });
        self.completed_tool_terminations.push(false);
        self.state.messages.push(agent_msg.clone());
        self.append_session_message(&agent_msg);
        events.push(AgentEvent::MessageStart {
            message: agent_msg.clone(),
        });
        events.push(AgentEvent::MessageEnd {
            message: agent_msg.clone(),
        });

        // If more tools are still pending, just return events
        if !self.pending_tool_calls.is_empty() {
            return (events, vec![]);
        }

        // All tools done: emit TurnEnd. Host should call continue_turn() to stream next LLM response.
        let assistant_msg = self
            .state
            .messages
            .iter()
            .rev()
            .find(|msg| matches!(msg, AgentMessage::Assistant(_)))
            .cloned()
            .unwrap_or_else(|| agent_msg.clone());
        let tool_results = std::mem::take(&mut self.completed_tool_results);
        self.completed_tool_terminations.clear();

        events.push(AgentEvent::TurnEnd {
            message: assistant_msg,
            tool_results,
        });

        (events, vec![])
    }

    /// Inject a steering message mid-run.
    pub(crate) fn steer(&mut self, message: AgentMessage) -> Vec<AgentEvent> {
        self.steering_queue.push(message);
        debug!(
            queued = self.steering_queue.len(),
            "steering message queued"
        );
        vec![AgentEvent::QueueUpdate {
            steer: self.steering_queue.clone(),
            follow_up: self.follow_up_queue.clone(),
        }]
    }

    /// Queue a follow-up message for after the run would otherwise stop.
    pub(crate) fn follow_up(&mut self, message: AgentMessage) {
        self.follow_up_queue.push(message);
        debug!(
            queued = self.follow_up_queue.len(),
            "follow-up message queued"
        );
    }

    /// Abort the current run.
    pub(crate) fn abort(&mut self) -> Vec<AgentEvent> {
        warn!(phase = ?self.phase, "agent aborted");
        self.steering_queue.clear();
        self.follow_up_queue.clear();
        self.pending_tool_calls.clear();
        self.completed_tool_results.clear();
        self.completed_tool_terminations.clear();
        self.state.pending_tool_calls.clear();
        self.state.is_streaming = false;
        self.state.streaming_message = None;
        self.phase = Phase::Idle;
        self.session_state = SessionState::default();

        vec![
            AgentEvent::QueueUpdate {
                steer: vec![],
                follow_up: vec![],
            },
            AgentEvent::AgentEnd {
                messages: self.state.messages.clone(),
            },
        ]
    }

    /// Read-only access to current state.
    pub fn state(&self) -> &AgentState {
        &self.state
    }

    /// Mutable access to current state.
    pub fn state_mut(&mut self) -> &mut AgentState {
        &mut self.state
    }

    /// Read-only access to the session tree.
    pub fn session_state(&self) -> &SessionState {
        &self.session_state
    }

    /// Replace the in-memory session tree.
    pub(crate) fn set_session_state(&mut self, state: SessionState) {
        self.session_state = state;
    }

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
    fn append_session_message(&mut self, message: &AgentMessage) {
        let id = format!("entry-{}", self.session_state.entries.len());
        let parent_id = self.session_state.entries.last().map(|e| e.id.clone());
        let entry = SessionEntry {
            id,
            parent_id,
            kind: EntryKind::Message {
                message: message.clone(),
            },
            timestamp: current_timestamp(),
        };
        self.session_state.leaf_id = entry.id.clone();
        self.session_state.entries.push(entry);
    }

    /// Reset state (clear messages, queues, runtime state).
    pub(crate) fn reset(&mut self) {
        debug!("agent state reset");
        self.state.messages.clear();
        self.state.is_streaming = false;
        self.state.streaming_message = None;
        self.state.pending_tool_calls.clear();
        self.state.error_message = None;
        self.steering_queue.clear();
        self.follow_up_queue.clear();
        self.pending_tool_calls.clear();
        self.completed_tool_results.clear();
        self.session_state = SessionState::default();
        self.completed_tool_terminations.clear();
        self.phase = Phase::Idle;
    }

    // --- private helpers ---

    fn build_llm_context(&self) -> LlmContext {
        LlmContext {
            system_prompt: self.state.system_prompt.clone(),
            messages: self.state.messages.clone(),
            tools: self.state.tools.clone(),
        }
    }

    fn drain_steering(&mut self) -> Vec<AgentMessage> {
        if self.steering_mode == QueueMode::All {
            std::mem::take(&mut self.steering_queue)
        } else {
            if self.steering_queue.is_empty() {
                vec![]
            } else {
                vec![self.steering_queue.remove(0)]
            }
        }
    }

    fn drain_follow_up(&mut self) -> Vec<AgentMessage> {
        if self.follow_up_mode == QueueMode::All {
            std::mem::take(&mut self.follow_up_queue)
        } else {
            if self.follow_up_queue.is_empty() {
                vec![]
            } else {
                vec![self.follow_up_queue.remove(0)]
            }
        }
    }

    fn inject_messages_and_stream(
        &mut self,
        messages: Vec<AgentMessage>,
    ) -> (Vec<AgentEvent>, Vec<AgentAction>) {
        let mut events = Vec::new();

        for msg in messages {
            events.push(AgentEvent::MessageStart {
                message: msg.clone(),
            });
            self.state.messages.push(msg.clone());
            self.append_session_message(&msg);
            events.push(AgentEvent::MessageEnd {
                message: msg.clone(),
            });
        }

        self.phase = Phase::Streaming;
        self.state.is_streaming = true;
        events.push(AgentEvent::TurnStart);

        let actions = vec![AgentAction::StreamLlm {
            context: self.build_llm_context(),
            session_id: self.session_id.clone(),
        }];

        (events, actions)
    }

    /// Internal accessor for the typestate layer.
    /// Returns the first pending tool call without removing it.
    pub(crate) fn peek_pending_tool_call(&self) -> Option<&ToolCall> {
        self.pending_tool_calls.values().next()
    }
}

fn current_timestamp() -> u64 {
    crate::timestamp::current_timestamp()
}

/// Helper trait for extracting assistant text.
pub trait AssistantTextExt {
    fn assistant_text(&self) -> String;
}

impl AssistantTextExt for AgentMessage {
    fn assistant_text(&self) -> String {
        match self {
            AgentMessage::Assistant(a) => a
                .content
                .iter()
                .filter_map(|c| match c {
                    Content::Text(t) => Some(t.text.clone()),
                    _ => None,
                })
                .collect(),
            _ => String::new(),
        }
    }
}
