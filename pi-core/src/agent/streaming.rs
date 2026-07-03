use super::{Agent, Phase};
use crate::context_projection::projection_scan;
use crate::events::{AgentAction, AgentEvent, ContentDelta, WaitMode};
use crate::llm::{LlmChunk, LlmResult};
use crate::message::{
    AgentMessage, Artifacts, Content, StopReason, TextContent, ToolCall, TrimmedMessage,
};
use tracing::{debug, trace, warn};

impl Agent {
    /// Feed a streaming chunk from the LLM.
    ///
    /// Accumulates into self.streaming_assistant. Does NOT modify T.
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
                self.streaming_assistant = Some(partial.clone());
                let msg = AgentMessage::Assistant(partial);
                events.push(AgentEvent::MessageStart { message: msg });
            }
            LlmChunk::TextDelta { text } => {
                trace!(bytes = text.len(), "llm text delta");
                if let Some(ref mut a) = self.streaming_assistant {
                    if let Some(Content::Text(ref mut t)) = a.content.last_mut() {
                        t.text.push_str(&text);
                    } else {
                        a.content
                            .push(Content::Text(TextContent { text: text.clone() }));
                    }
                    let msg = AgentMessage::Assistant(a.clone());
                    events.push(AgentEvent::MessageUpdate {
                        message: msg.clone(),
                        delta: ContentDelta::TextDelta { text },
                    });
                }
            }
            LlmChunk::ToolCallDelta {
                tool_call_id,
                delta,
            } => {
                if let Some(ref a) = self.streaming_assistant {
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
                if let Some(ref a) = self.streaming_assistant {
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
    ///
    /// Finalizes the assistant message, pushes to T, runs projection_scan at turn end
    /// (when EndTurn with no tools, or all tools completed).
    pub(crate) fn on_llm_done(
        &mut self,
        result: LlmResult,
        mut t: Vec<TrimmedMessage>,
        mut a: Artifacts,
        turn_number: u32,
        _budget: &crate::context_projection::ContextProjectionBudget,
    ) -> (
        Vec<AgentEvent>,
        Vec<AgentAction>,
        Vec<crate::context_projection::ChangeMarker>,
        Vec<TrimmedMessage>,
        Artifacts,
    ) {
        if self.phase != Phase::Streaming {
            warn!(phase = ?self.phase, "on_llm_done requested outside streaming phase");
            return (vec![], vec![], vec![], t, a);
        }

        let mut events = Vec::new();
        let mut actions = Vec::new();
        let mut markers = Vec::new();

        let assistant_msg = result.finalize_message();
        let msg = AgentMessage::Assistant(assistant_msg.clone());

        self.streaming_assistant = None;

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
            events.push(AgentEvent::AgentEnd);
            self.phase = Phase::Idle;
            actions.push(AgentAction::Finished);
            return (events, actions, markers, t, a);
        }

        // Push finalized assistant message to T — but never an empty one.
        // Failed streams are handled above and must not pollute T with partial
        // tool calls that have no matching ToolResult in the next provider turn.
        if !assistant_msg.is_empty() {
            t.push(TrimmedMessage::Assistant(assistant_msg.clone()));
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
            debug!("assistant turn finished without tool calls — running projection_scan");
            // EndTurn without tools: run projection_scan
            let scan_markers = projection_scan(&mut t, &mut a, turn_number);
            markers.extend(scan_markers);

            events.push(AgentEvent::TurnEnd {
                message: msg,
                tool_results: vec![],
            });

            // If a steering message was queued during this stream, do not
            // finish: hand control back so continue_turn drains the queue and
            // re-streams. Without this, a steer queued during a final-answer
            // stream is silently stranded (continue_turn is the only drain
            // site, and Finished never reaches it).
            if !self.steering_queue.is_empty() {
                debug!(
                    queued = self.steering_queue.len(),
                    "ending stream with pending steering — deferring to continue_turn"
                );
                self.phase = Phase::WaitForInput;
                events.push(AgentEvent::AgentEnd);
                actions.push(AgentAction::WaitForInput {
                    mode: WaitMode::Steering,
                });
                return (events, actions, markers, t, a);
            }

            events.push(AgentEvent::AgentEnd);
            self.phase = Phase::Idle;
            actions.push(AgentAction::Finished);
            return (events, actions, markers, t, a);
        }

        // Track proposed tools. Execution starts only after host preparation allows them.
        for tc in &tool_calls {
            self.pending_tool_calls.insert(tc.id.clone(), tc.clone());
            self.state.pending_tool_calls.push(tc.id.0.clone());
        }

        self.phase = Phase::PreToolCall;
        debug!(
            tool_count = tool_calls.len(),
            "assistant requested tool execution"
        );
        actions.push(AgentAction::PrepareToolCalls { calls: tool_calls });
        (events, actions, markers, t, a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AgentOptions, Phase};
    use crate::events::{QueueMode, ThinkingLevel};
    use crate::llm::{LlmError, Model, ModelCapabilities, ModelCost};
    use crate::message::{Artifacts, TrimmedMessage, UserMessage};
    use crate::tool::ExecutionMode;

    fn test_agent() -> Agent {
        let mut agent = Agent::new(AgentOptions {
            system_prompt: "test".to_string(),
            model: Model {
                id: crate::types::ModelId::new("test"),
                name: crate::types::ModelName::new("test"),
                api: crate::types::ApiName::new("test"),
                provider: crate::types::ProviderName::new("test"),
                base_url: None,
                reasoning: false,
                context_window: 1000,
                max_tokens: 100,
                capabilities: ModelCapabilities::default(),
                cost: ModelCost::default(),
            },
            thinking_level: ThinkingLevel::Off,
            steering_mode: QueueMode::OneAtATime,
            follow_up_mode: QueueMode::OneAtATime,
            tool_execution_mode: ExecutionMode::Sequential,
            session_id: None,
        });
        agent.phase = Phase::Streaming;
        agent
    }

    #[test]
    fn failed_llm_turn_does_not_persist_assistant_message() {
        let mut agent = test_agent();
        let t = vec![TrimmedMessage::User(UserMessage::new_text("continue"))];

        let (_events, _actions, _markers, t, _a) = agent.on_llm_done(
            LlmResult::Err {
                error: LlmError {
                    code: "stream_error".to_string(),
                    message: "network error".to_string(),
                    details: None,
                },
                aborted: false,
            },
            t,
            Artifacts::new(),
            1,
            &crate::context_projection::ContextProjectionBudget::default(),
        );

        assert_eq!(t.len(), 1);
        assert!(matches!(t[0], TrimmedMessage::User(_)));
    }
}
