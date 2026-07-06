use std::time::Duration;

use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
use ratatui::text::Text;

use pi_core::{message::TokenUsage, timestamp};
use pi_core::{
    AgentRuntime, Content, LlmChunk, LlmError, LlmResult, StopReason, TextContent, ToolArguments,
    ToolCallId, ToolName,
};
use pi_core::{ApiName, AssistantMessage, ModelId, ProviderName};

use crate::app::{App, ChatEntry};
#[allow(unused_imports)]
use crate::llm::{LlmProvider, LlmStreamState};
use crate::markdown;
use crate::session_log::SessionEvent;

impl App {
    pub(crate) fn stream_llm(
        &mut self,
        terminal: &mut ratatui::DefaultTerminal,
        context: pi_core::LlmContext,
    ) {
        self.running = true;
        self.streaming_text.clear();

        // Log LLM request
        if let Some(ref logger) = self.session_logger {
            let _ = logger.append(&SessionEvent::LlmRequest {
                turn: self.turn_number,
                model: self.llm_client.model_id().to_string(),
                message_count: context.messages.len(),
            });
        }

        match self
            .llm_client
            .stream_sync(&context.system_prompt, &context.messages, &context.tools)
        {
            Ok(mut stream) => {
                // Take streaming agent out so we can feed chunks directly without
                // borrowing self.agent inside the loop.
                let runtime = self.agent.take().unwrap();
                let AgentRuntime::Streaming(mut streaming) = runtime else {
                    self.agent = Some(runtime);
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
                        let transcript = std::mem::take(&mut self.transcript);
                        let artifacts = std::mem::take(&mut self.artifacts);
                        let transition = streaming.abort(transcript, artifacts, self.turn_number);
                        let (
                            _events,
                            _actions,
                            runtime,
                            transcript,
                            artifacts,
                            turn_number,
                            _markers,
                        ) = transition.into_parts();
                        self.transcript = transcript;
                        self.artifacts = artifacts;
                        self.turn_number = turn_number;
                        self.agent = Some(runtime.into_runtime());
                        self.running = false;
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
                                let rendered = markdown::render(&full_text, 80);
                                *self.entries.last_mut().unwrap() = ChatEntry::Assistant(rendered);
                            }
                        }
                        LlmChunk::Done => break,
                        LlmChunk::Error { message } => {
                            self.entries
                                .push(ChatEntry::System(format!("LLM Error: {message}")));
                            if let Some(ref logger) = self.session_logger {
                                let _ = logger.append(&SessionEvent::Error {
                                    turn: self.turn_number,
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
                    let _ = logger.append(&SessionEvent::LlmResponse {
                        turn: self.turn_number,
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
                let transcript = std::mem::take(&mut self.transcript);
                let artifacts = std::mem::take(&mut self.artifacts);
                let budget = self.budget.clone();
                let transition =
                    streaming.finish_llm(result, transcript, artifacts, self.turn_number, &budget);
                let (_events, actions, runtime, transcript, artifacts, turn_number, _markers) =
                    transition.into_parts();
                tracing::debug!(?actions, "finish_llm");
                self.transcript = transcript;
                self.artifacts = artifacts;
                self.turn_number = turn_number;
                self.agent = Some(runtime);
                self.handle_actions(terminal, actions);
            }
            Err(e) => {
                tracing::error!(error = ?e, "LLM stream failed to start");
                if let Some(ref logger) = self.session_logger {
                    let _ = logger.append(&SessionEvent::Error {
                        turn: self.turn_number,
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
                let runtime = self.agent.take().unwrap();
                let transcript = std::mem::take(&mut self.transcript);
                let artifacts = std::mem::take(&mut self.artifacts);
                let (_events, actions, new_runtime, transcript, artifacts, turn_number, _markers) =
                    match runtime {
                        AgentRuntime::Streaming(streaming) => {
                            let budget = self.budget.clone();
                            let transition = streaming.finish_llm(
                                err_result,
                                transcript,
                                artifacts,
                                self.turn_number,
                                &budget,
                            );
                            transition.into_parts()
                        }
                        AgentRuntime::Compacting(compacting) => {
                            let (ev, act, state, transcript, artifacts, tn, m) = compacting
                                .abort(transcript, artifacts, self.turn_number)
                                .into_parts();
                            (ev, act, state.into_runtime(), transcript, artifacts, tn, m)
                        }
                        other => (
                            vec![],
                            vec![],
                            other,
                            transcript,
                            artifacts,
                            self.turn_number,
                            vec![],
                        ),
                    };
                self.transcript = transcript;
                self.artifacts = artifacts;
                self.turn_number = turn_number;
                self.agent = Some(new_runtime);
                self.handle_actions(terminal, actions);
            }
        }

        self.cancelled = false;
        self.streaming_text.clear();
    }
}
