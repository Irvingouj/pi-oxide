use super::{Agent, Phase};
use crate::context_projection::{ChangeMarker, ContextProjectionBudget};
use crate::events::{AgentAction, AgentEvent, WaitMode};
use crate::message::{AgentMessage, TrimmedMessage, UserMessage};
use crate::tool::ToolDefinition;
use tracing::{debug, warn};

impl Agent {
    /// Start processing a new prompt.
    ///
    /// Pushes the user message to transcript, increments turn_number, builds LLM context.
    pub(crate) fn start_turn(
        &mut self,
        prompt: AgentMessage,
        tools: Vec<ToolDefinition>,
        mut t: Vec<TrimmedMessage>,
        turn_number: u32,
        budget: &ContextProjectionBudget,
        compaction_prompt: &str,
    ) -> (
        Vec<AgentEvent>,
        Vec<AgentAction>,
        Vec<ChangeMarker>,
        Vec<TrimmedMessage>,
        u32,
    ) {
        if self.phase == Phase::Streaming {
            warn!(phase = ?self.phase, "start_turn requested while LLM is streaming");
            return (
                vec![AgentEvent::AgentStart],
                vec![AgentAction::WaitForInput {
                    mode: WaitMode::Any,
                }],
                vec![],
                t,
                turn_number,
            );
        }

        // Push user message to T
        if let AgentMessage::User(u) = prompt.clone() {
            t.push(TrimmedMessage::User(u));
        } else {
            warn!("start_turn received non-User message, wrapping");
            t.push(TrimmedMessage::User(UserMessage::new_text(format!(
                "{:?}",
                prompt
            ))));
        }
        let next_turn = turn_number + 1;

        self.turn_tools = tools;
        debug!(
            message_count = t.len(),
            turn = next_turn,
            "agent turn started"
        );
        let events = vec![
            AgentEvent::AgentStart,
            AgentEvent::TurnStart,
            AgentEvent::MessageStart {
                message: prompt.clone(),
            },
            AgentEvent::MessageEnd {
                message: prompt.clone(),
            },
        ];

        self.phase = Phase::Streaming;

        let (context, markers) = self.build_llm_context(&t);

        // Check compaction (using plan_compaction on T)
        let total_chars: usize = t.iter().map(trimmed_message_chars).sum();
        let threshold = (budget.compaction_threshold * budget.max_context_tokens as f32) as usize;
        let estimated_tokens = total_chars.div_ceil(4);

        if estimated_tokens > threshold {
            if let Some(action) = self.build_summary_action(&t, budget, compaction_prompt) {
                self.phase = Phase::Compacting;
                return (events, vec![action], markers, t, next_turn);
            }
            warn!(
                estimated_tokens,
                threshold,
                "over budget but compaction cannot find a valid cut point — streaming with oversized context"
            );
        }

        let actions = vec![AgentAction::StreamLlm {
            context,
            session_id: self.session_id.clone(),
        }];

        (events, actions, markers, t, next_turn)
    }

    /// Continue from the current transcript without adding a new message.
    ///
    /// Takes transcript but not artifacts: continue_turn only reads transcript for
    /// LLM context building. Artifacts are carried through the typestate layer unchanged
    /// and only modified by projection_scan or apply_compaction.
    pub(crate) fn continue_turn(
        &mut self,
        mut t: Vec<TrimmedMessage>,
        _turn_number: u32,
        budget: &ContextProjectionBudget,
        compaction_prompt: &str,
    ) -> (
        Vec<AgentEvent>,
        Vec<AgentAction>,
        Vec<ChangeMarker>,
        Vec<TrimmedMessage>,
    ) {
        if self.phase == Phase::Streaming {
            warn!(phase = ?self.phase, "continue_turn requested while LLM is streaming");
            return (vec![], vec![], vec![], t);
        }

        let mut events = vec![];

        // Drain steering queue
        let drained = self.drain_steering();
        for msg in &drained {
            if let AgentMessage::User(u) = msg {
                t.push(TrimmedMessage::User(u.clone()));
            }
            events.push(AgentEvent::MessageStart {
                message: msg.clone(),
            });
            events.push(AgentEvent::MessageEnd {
                message: msg.clone(),
            });
        }

        // Drain follow-up queue
        let follow = self.drain_follow_up();
        for msg in &follow {
            if let AgentMessage::User(u) = msg {
                t.push(TrimmedMessage::User(u.clone()));
            }
            events.push(AgentEvent::MessageStart {
                message: msg.clone(),
            });
            events.push(AgentEvent::MessageEnd {
                message: msg.clone(),
            });
        }

        let last = t.last();
        if let Some(TrimmedMessage::Assistant(_)) = last {
            if drained.is_empty() && follow.is_empty() {
                return (
                    vec![],
                    vec![AgentAction::WaitForInput {
                        mode: WaitMode::Any,
                    }],
                    vec![],
                    t,
                );
            }
        }

        self.phase = Phase::Streaming;

        events.push(AgentEvent::AgentStart);
        events.push(AgentEvent::TurnStart);

        let (context, markers) = self.build_llm_context(&t);

        // Check compaction
        let total_chars: usize = t.iter().map(trimmed_message_chars).sum();
        let threshold = (budget.compaction_threshold * budget.max_context_tokens as f32) as usize;
        let estimated_tokens = total_chars.div_ceil(4);

        if estimated_tokens > threshold {
            if let Some(action) = self.build_summary_action(&t, budget, compaction_prompt) {
                self.phase = Phase::Compacting;
                return (events, vec![action], markers, t);
            }
            warn!(
                estimated_tokens,
                threshold,
                "over budget but compaction cannot find a valid cut point — streaming with oversized context"
            );
        }

        let actions = vec![AgentAction::StreamLlm {
            context,
            session_id: self.session_id.clone(),
        }];

        (events, actions, markers, t)
    }
}

/// Estimate character count for a TrimmedMessage.
fn trimmed_message_chars(msg: &TrimmedMessage) -> usize {
    match msg {
        TrimmedMessage::User(u) => u
            .content
            .iter()
            .filter_map(|c| {
                if let crate::message::Content::Text(t) = c {
                    Some(t.text.chars().count())
                } else {
                    None
                }
            })
            .sum(),
        TrimmedMessage::Assistant(a) => a
            .content
            .iter()
            .map(|c| match c {
                crate::message::Content::Text(t) => t.text.chars().count(),
                crate::message::Content::ToolCall(tc) => {
                    tc.name.as_str().chars().count()
                        + serde_json::to_string(&tc.arguments)
                            .map(|s| s.chars().count())
                            .unwrap_or(0)
                }
                _ => 0,
            })
            .sum(),
        TrimmedMessage::OriginalTool(tool) => tool
            .content
            .iter()
            .filter_map(|c| {
                if let crate::message::Content::Text(t) = c {
                    Some(t.text.chars().count())
                } else {
                    None
                }
            })
            .sum(),
        TrimmedMessage::ProjectedTool(tool) => tool.preview.chars().count(),
        TrimmedMessage::Compaction(c) => c.summary.chars().count(),
    }
}
