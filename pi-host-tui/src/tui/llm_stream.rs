use std::time::Duration;

use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
use ratatui::text::Text;

use pi_core::{message::TokenUsage, timestamp};
use pi_core::{
    AgentRuntime, Content, LlmChunk, LlmError, LlmResult, StopReason, TextContent, ToolArguments,
    ToolCallId, ToolName,
};

use crate::agent_host::TransitionParts;
use pi_core::{ApiName, AssistantMessage, ModelId, ProviderName};

use crate::app::{App, ChatEntry};
use crate::markdown;
use crate::session_log::SessionEvent;

impl App {
    pub(crate) fn stream_llm(
        &mut self,
        terminal: &mut ratatui::DefaultTerminal,
        context: pi_core::LlmContext,
    ) {
        self.running = true;
        self.streaming_start = Some(std::time::Instant::now());
        self.streaming_text.clear();

        // Log LLM request
        if let Some(ref logger) = self.session_logger {
            let turn = self.agent_host.as_ref().expect("agent").turn_number;
            let _ = logger.append(&SessionEvent::LlmRequest {
                turn,
                model: self.llm_client.model_id().to_string(),
                message_count: context.messages.len(),
            });
        }

        match self
            .llm_client
            .stream_sync(&context.system_prompt, &context.messages, &context.tools)
        {
            Ok(mut stream) => {
                // Extract the runtime so we can feed chunks directly without
                // borrowing self.agent_host inside the loop.
                let runtime = self
                    .agent_host
                    .as_mut()
                    .expect("agent")
                    .take_runtime();
                let AgentRuntime::Streaming(mut streaming) = runtime else {
                    self.agent_host
                        .as_mut()
                        .expect("agent")
                        .set_runtime(runtime);
                    return;
                };

                // Feed a synthetic Start chunk so the core sees the assistant message
                // and can accumulate partial state during streaming.
                let start_events = streaming.feed_llm_chunk(pi_core::LlmChunk::Start {
                    partial: AssistantMessage::empty(),
                });
                for event in start_events {
                    if let pi_core::AgentEvent::MessageStart { .. } = event {
                        self.entries.push(ChatEntry::Assistant(Text::raw("...")));
                    }
                }
                let _ = terminal.draw(|f| self.render(f));

                let mut full_text = String::new();

                for chunk in stream.by_ref() {
                    // Cooperative cancellation: poll keyboard without blocking
                    if crossterm::event::poll(Duration::from_millis(0)).unwrap_or(false) {
                        if let Ok(crossterm::event::Event::Key(key)) = crossterm::event::read() {
                            if key.kind == KeyEventKind::Press {
                                if key.code == KeyCode::Char('c')
                                    && key.modifiers.contains(KeyModifiers::CONTROL)
                                {
                                    self.cancelled = true;
                                } else if key.code == KeyCode::Esc {
                                    self.should_quit = true;
                                    self.cancelled = true;
                                }
                            }
                        }
                    }
                    if self.cancelled {
                        let (transcript, artifacts) = (
                            std::mem::take(
                                &mut self
                                    .agent_host
                                    .as_mut()
                                    .expect("agent")
                                    .transcript,
                            ),
                            std::mem::take(
                                &mut self
                                    .agent_host
                                    .as_mut()
                                    .expect("agent")
                                    .artifacts,
                            ),
                        );
                        let turn = self.agent_host.as_ref().expect("agent").turn_number;
                        let (events, actions, new_runtime, transcript, artifacts, turn_number, _markers) =
                            streaming.abort(transcript, artifacts, turn).into_parts();
                        self.agent_host
                            .as_mut()
                            .expect("agent")
                            .transcript = transcript;
                        self.agent_host
                            .as_mut()
                            .expect("agent")
                            .artifacts = artifacts;
                        self.agent_host
                            .as_mut()
                            .expect("agent")
                            .turn_number = turn_number;
                        self.agent_host
                            .as_mut()
                            .expect("agent")
                            .set_runtime(new_runtime.into_runtime());
                        let _ = (events, actions);
                        self.running = false;
                        self.streaming_start = None;
                        self.entries.push(ChatEntry::System("Cancelled.".into()));
                        let _ = terminal.draw(|f| self.render(f));
                        return;
                    }

                    // Feed chunk to core so the state machine tracks partial assistant state
                    let _core_events = streaming.feed_llm_chunk(chunk.clone());

                    match chunk {
                        LlmChunk::TextDelta { text } => {
                            full_text.push_str(&text);
                            self.streaming_text = full_text.clone();
                            if let Some(ChatEntry::Assistant(_)) = self.entries.last() {
                                let rendered = markdown::render(&full_text);
                                *self.entries.last_mut().unwrap() = ChatEntry::Assistant(rendered);
                            }
                        }
                        LlmChunk::Done => break,
                        LlmChunk::Error { message } => {
                            self.entries
                                .push(ChatEntry::System(format!("LLM Error: {message}")));
                            if let Some(ref logger) = self.session_logger {
                                let turn = self.agent_host.as_ref().expect("agent").turn_number;
                                let _ = logger.append(&SessionEvent::Error {
                                    turn,
                                    message: message.clone(),
                                });
                            }
                            break;
                        }
                        _ => {}
                    }
                    let _ = terminal.draw(|f| self.render(f));
                }

                let usage = stream.usage();
                if let Some((input, output, total)) = usage {
                    self.last_usage = Some((input, output, total));
                }

                let stop_reason = stream.stop_reason().unwrap_or("end_turn");

                // Log LLM response
                if let Some(ref logger) = self.session_logger {
                    let turn = self.agent_host.as_ref().expect("agent").turn_number;
                    let _ = logger.append(&SessionEvent::LlmResponse {
                        turn,
                        stop_reason: stop_reason.to_string(),
                    });
                }

                let tool_use_blocks: Vec<Content> = stream
                    .tool_calls()
                    .into_iter()
                    .map(|tc| {
                        Content::ToolCall(pi_core::ToolCall {
                            id: ToolCallId::new(&tc.id),
                            name: ToolName::new(&tc.name),
                            arguments: ToolArguments::new(tc.input),
                        })
                    })
                    .collect();

                tracing::debug!(
                    text_len = full_text.len(),
                    tool_calls = tool_use_blocks.len(),
                    %stop_reason,
                    "LLM stream completed"
                );

                let text_block = if full_text.is_empty() && tool_use_blocks.is_empty() {
                    vec![Content::Text(TextContent {
                        text: String::new(),
                    })]
                } else if full_text.is_empty() {
                    vec![]
                } else {
                    vec![Content::Text(TextContent { text: full_text })]
                };

                let content: Vec<Content> = text_block.into_iter().chain(tool_use_blocks).collect();
                let sr = if stop_reason == "tool_use" {
                    StopReason::ToolUse
                } else {
                    StopReason::EndTurn
                };

                tracing::debug!(content_blocks = content.len(), stop_reason = ?sr, "LLM stream assembled");

                let assistant_msg = AssistantMessage {
                    content,
                    api: ApiName::new("anthropic"),
                    provider: ProviderName::new("anthropic"),
                    model: ModelId::new(self.llm_client.model_id()),
                    stop_reason: sr,
                    error_message: None,
                    timestamp: timestamp::current_timestamp(),
                    usage: TokenUsage {
                        input: self.last_usage.map(|(i, _, _)| i).unwrap_or(0),
                        output: self.last_usage.map(|(_, o, _)| o).unwrap_or(0),
                        cache_read: 0,
                        cache_write: 0,
                        total_tokens: self.last_usage.map(|(_, _, t)| t).unwrap_or(0),
                    },
                };

                let result = LlmResult::Ok(assistant_msg);
                let budget = self.budget.clone();
                let (_events, actions) = self
                    .agent_host
                    .as_mut()
                    .expect("agent")
                    .transition(move |runtime, transcript, artifacts, turn| {
                        let AgentRuntime::Streaming(s) = runtime else {
                            unreachable!("streaming agent was extracted above");
                        };
                        TransitionParts::from(s.finish_llm(result, transcript, artifacts, turn, &budget).into_parts())
                    });
                tracing::debug!(?actions, "finish_llm");
                self.handle_actions(terminal, actions);
            }
            Err(e) => {
                tracing::error!(error = ?e, "LLM stream failed to start");
                if let Some(ref logger) = self.session_logger {
                    let turn = self.agent_host.as_ref().expect("agent").turn_number;
                    let _ = logger.append(&SessionEvent::Error {
                        turn,
                        message: e.to_string(),
                    });
                }
                let err_result = LlmResult::Err {
                    error: LlmError {
                        code: "call_failed".into(),
                        message: e.to_string(),
                        details: None,
                    },
                    aborted: false,
                };
                let budget = self.budget.clone();
                let (_events, actions) = self
                    .agent_host
                    .as_mut()
                    .expect("agent")
                    .transition(|runtime, transcript, artifacts, turn| {
                        match runtime {
                            AgentRuntime::Streaming(streaming) => {
                                TransitionParts::from(streaming.finish_llm(
                                    err_result.clone(),
                                    transcript,
                                    artifacts,
                                    turn,
                                    &budget,
                                ).into_parts())
                            }
                            other => crate::agent_host::AgentHost::abort_compacting_or_pass_through(
                                other, transcript, artifacts, turn,
                            ),
                        }
                    });
                self.handle_actions(terminal, actions);
            }
        }

        self.cancelled = false;
        self.streaming_start = None;
        self.streaming_text.clear();
    }
}
