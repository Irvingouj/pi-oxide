use super::{Agent, Phase};
use crate::events::{AgentAction, AgentEvent, WaitMode};
use crate::message::AgentMessage;
use crate::session::SessionState;
use crate::tool::ToolDefinition;
use tracing::{debug, warn};

impl Agent {
    /// Start processing a new prompt.
    pub(crate) fn start_turn(
        &mut self,
        prompt: AgentMessage,
        tools: Vec<ToolDefinition>,
        session_state: &mut SessionState,
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

        self.rebuild_messages(session_state);
        self.state.messages.push(prompt.clone());
        self.append_session_message(session_state, &prompt);
        self.turn_tools = tools;
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
    pub(crate) fn continue_turn(
        &mut self,
        session_state: &mut SessionState,
    ) -> (Vec<AgentEvent>, Vec<AgentAction>) {
        if self.phase == Phase::Streaming {
            warn!(phase = ?self.phase, "continue_turn requested while LLM is streaming");
            return (vec![], vec![]);
        }

        let last = self.state.messages.last();
        if let Some(AgentMessage::Assistant(_)) = last {
            // Check steering queue first
            let drained = self.drain_steering();
            if !drained.is_empty() {
                return self.inject_messages_and_stream(drained, session_state);
            }

            let follow = self.drain_follow_up();
            if !follow.is_empty() {
                return self.inject_messages_and_stream(follow, session_state);
            }

            return (
                vec![],
                vec![AgentAction::WaitForInput {
                    mode: WaitMode::Any,
                }],
            );
        }

        self.rebuild_messages(session_state);
        self.phase = Phase::Streaming;
        self.state.is_streaming = true;

        let events = vec![AgentEvent::AgentStart, AgentEvent::TurnStart];
        let actions = vec![AgentAction::StreamLlm {
            context: self.build_llm_context(),
            session_id: self.session_id.clone(),
        }];

        (events, actions)
    }
}
