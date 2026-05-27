use super::{Agent, Phase};
use crate::events::{AgentAction, AgentEvent, ContentDelta};
use crate::llm::{LlmChunk, LlmResult};
use crate::message::{AgentMessage, Content, StopReason, TextContent, ToolCall};
use tracing::{debug, trace, warn};

impl Agent {
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
                partial.timestamp = super::current_timestamp();
                let msg = AgentMessage::Assistant(partial.clone());
                self.replace_last_assistant_or_push(msg.clone());
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
                        a.content.push(Content::Text(TextContent { text }));
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

        self.replace_last_assistant_or_push(msg.clone());
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
}
