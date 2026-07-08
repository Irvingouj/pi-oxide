use std::time::Duration;

use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
use ratatui::text::Text;

use pi_core::{message::TokenUsage, timestamp};
use pi_core::{
    AgentRuntime, Content, LlmChunk, LlmError, LlmResult, StopReason, TextContent, ToolArguments,
    ToolCallId, ToolName,
};

use crate::agent_host::{CollectedStreamData, CollectedToolCall, StreamOutcome, TransitionParts};
use pi_core::{ApiName, AssistantMessage, ModelId, ProviderName};

use crate::app::{App, ChatEntry};
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
        self.streaming_start = Some(std::time::Instant::now());
        self.streaming_text.clear();

        // Log LLM request
        if let Some(ref logger) = self.session_logger {
            let turn = self.agent_host.turn_number;
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
            Ok(stream) => {
                // Extract entries so the feed closure doesn't need to borrow self.
                let mut entries = std::mem::take(&mut self.entries);

                let budget = self.budget.clone();
                let model_id = self.llm_client.model_id().to_string();

                // Local variables for state updated in the feed closure.
                // Synced back to self after stream_and_transition returns.
                let mut was_cancelled = false;
                let mut should_quit = false;
                let mut streaming_text: String = String::new();

                let (_events, actions) = self.agent_host.stream_and_transition(
                    stream,
                    |streaming, stream| {
                        // Feed a synthetic Start chunk so the core sees the assistant message
                        let start_events = streaming.feed_llm_chunk(pi_core::LlmChunk::Start {
                            partial: AssistantMessage::empty(),
                        });
                        for event in start_events {
                            if let pi_core::AgentEvent::MessageStart { .. } = event {
                                entries.push(ChatEntry::Assistant(Text::raw("...")));
                            }
                        }

                        let mut full_text = String::new();

                        for chunk in stream.by_ref() {
                            // Cooperative cancellation
                            if crossterm::event::poll(Duration::from_millis(0)).unwrap_or(false) {
                                if let Ok(crossterm::event::Event::Key(key)) =
                                    crossterm::event::read()
                                {
                                    if key.kind == KeyEventKind::Press {
                                        if key.code == KeyCode::Char('c')
                                            && key.modifiers.contains(KeyModifiers::CONTROL)
                                        {
                                            was_cancelled = true;
                                        } else if key.code == KeyCode::Esc {
                                            should_quit = true;
                                            was_cancelled = true;
                                        }
                                    }
                                }
                            }
                            if was_cancelled {
                                entries.push(ChatEntry::System("Cancelled.".into()));
                                return StreamOutcome::Cancelled;
                            }

                            // Feed chunk to core
                            let _core_events = streaming.feed_llm_chunk(chunk.clone());

                            match chunk {
                                LlmChunk::TextDelta { text } => {
                                    full_text.push_str(&text);
                                    streaming_text = full_text.clone();
                                    if let Some(ChatEntry::Assistant(_)) = entries.last() {
                                        let rendered = markdown::render(&full_text);
                                        *entries.last_mut().unwrap() =
                                            ChatEntry::Assistant(rendered);
                                    }
                                }
                                LlmChunk::Done => break,
                                LlmChunk::Error { message } => {
                                    entries
                                        .push(ChatEntry::System(format!("LLM Error: {message}")));
                                    break;
                                }
                                _ => {}
                            }
                        }

                        // Collect stream metadata
                        let usage = stream.usage();
                        let stop_reason = stream.stop_reason().unwrap_or("end_turn").to_string();
                        let tool_calls: Vec<CollectedToolCall> = stream
                            .tool_calls()
                            .into_iter()
                            .map(|tc| CollectedToolCall {
                                id: tc.id,
                                name: tc.name,
                                input: tc.input,
                            })
                            .collect();

                        tracing::debug!(
                            text_len = full_text.len(),
                            tool_calls = tool_calls.len(),
                            %stop_reason,
                            "LLM stream completed"
                        );

                        StreamOutcome::Finished(CollectedStreamData {
                            text: full_text,
                            usage,
                            stop_reason,
                            tool_calls,
                        })
                    },
                    |runtime, data, transcript, artifacts, turn| {
                        // Build tool call content blocks
                        let tool_use_blocks: Vec<Content> = data
                            .tool_calls
                            .into_iter()
                            .map(|tc| {
                                Content::ToolCall(pi_core::ToolCall {
                                    id: ToolCallId::new(&tc.id),
                                    name: ToolName::new(&tc.name),
                                    arguments: ToolArguments::new(tc.input),
                                })
                            })
                            .collect();

                        let text_block = if data.text.is_empty() && tool_use_blocks.is_empty() {
                            vec![Content::Text(TextContent {
                                text: String::new(),
                            })]
                        } else if data.text.is_empty() {
                            vec![]
                        } else {
                            vec![Content::Text(TextContent { text: data.text })]
                        };

                        let content: Vec<Content> =
                            text_block.into_iter().chain(tool_use_blocks).collect();

                        let sr = if data.stop_reason == "tool_use"
                            || content.iter().any(|c| matches!(c, Content::ToolCall(_)))
                        {
                            StopReason::ToolUse
                        } else {
                            StopReason::EndTurn
                        };

                        let assistant_msg = AssistantMessage {
                            content,
                            api: ApiName::new("anthropic"),
                            provider: ProviderName::new("anthropic"),
                            model: ModelId::new(&model_id),
                            stop_reason: sr,
                            error_message: None,
                            timestamp: timestamp::current_timestamp(),
                            usage: TokenUsage {
                                input: data.usage.map(|(i, _, _)| i).unwrap_or(0),
                                output: data.usage.map(|(_, o, _)| o).unwrap_or(0),
                                cache_read: 0,
                                cache_write: 0,
                                total_tokens: data.usage.map(|(_, _, t)| t).unwrap_or(0),
                            },
                        };

                        let AgentRuntime::Streaming(s) = runtime else {
                            unreachable!("stream_and_transition guarantees Streaming runtime");
                        };

                        TransitionParts::from(
                            s.finish_llm(
                                LlmResult::Ok(assistant_msg),
                                transcript,
                                artifacts,
                                turn,
                                &budget,
                            )
                            .into_parts(),
                        )
                    },
                );

                // Sync local state back to self
                self.entries = entries;
                self.streaming_text = streaming_text;
                self.cancelled = was_cancelled;
                self.should_quit = should_quit;

                // Render final state
                let _ = terminal.draw(|f| self.render(f));

                // If cancelled, the abort transition was already handled
                if was_cancelled {
                    self.running = false;
                    self.streaming_start = None;
                    self.cancelled = false;
                    self.streaming_text.clear();
                    return;
                }

                // Log LLM response
                if let Some(ref logger) = self.session_logger {
                    let turn = self.agent_host.turn_number;
                    let _ = logger.append(&SessionEvent::LlmResponse {
                        turn,
                        stop_reason: "completed".to_string(),
                    });
                }

                tracing::debug!(?actions, "finish_llm");
                self.handle_actions(terminal, actions);
            }
            Err(e) => {
                tracing::error!(error = ?e, "LLM stream failed to start");
                if let Some(ref logger) = self.session_logger {
                    let turn = self.agent_host.turn_number;
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
                let (_events, actions) = self.agent_host.transition(
                    |runtime, transcript, artifacts, turn| match runtime {
                        AgentRuntime::Streaming(streaming) => TransitionParts::from(
                            streaming
                                .finish_llm(
                                    err_result.clone(),
                                    transcript,
                                    artifacts,
                                    turn,
                                    &budget,
                                )
                                .into_parts(),
                        ),
                        other => crate::agent_host::AgentHost::abort_compacting_or_pass_through(
                            other, transcript, artifacts, turn,
                        ),
                    },
                );
                self.handle_actions(terminal, actions);
            }
        }

        self.cancelled = false;
        self.streaming_start = None;
        self.streaming_text.clear();
    }
}
