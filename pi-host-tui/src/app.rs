use std::time::Duration;

use crossterm::event::{KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
};
use ratatui::Frame;

use pi_core::{
    project, Agent, AgentAction, AgentMessage, Content, ContextProjectionBudget,
    ContextProjectionState, LlmChunk, LlmContext, LlmResult, Model, ProjectionInput, ToolCall,
};

use crate::llm::LlmClient;
use crate::markdown;
use crate::session::FileSystemSessionBackend;
use crate::tools;

// ---------------------------------------------------------------------------
// Chat line types
// ---------------------------------------------------------------------------

#[allow(dead_code)]
enum ChatEntry {
    User(String),
    Assistant(Text<'static>),
    ToolStart {
        name: String,
        args_summary: String,
    },
    ToolResult {
        name: String,
        output: String,
        #[allow(dead_code)]
        is_error: bool,
    },
    System(String),
}

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

pub struct App {
    agent: Agent,
    entries: Vec<ChatEntry>,
    input: String,
    cursor_pos: usize,
    scroll_offset: u16,
    auto_scroll: bool,
    should_quit: bool,
    running: bool,
    streaming_text: String,
    #[allow(dead_code)]
    current_tools: Vec<(String, String)>, // (name, args_summary) for active tools
    llm_client: LlmClient,
    projection_state: ContextProjectionState,
    budget: ContextProjectionBudget,
    last_usage: Option<(u32, u32, u32)>, // (input, output, total)
    session_id: Option<String>,
    session_backend: FileSystemSessionBackend,
}

impl App {
    pub fn new(
        system_prompt: &str,
        model_id: &str,
        api_key: &str,
        base_url: &str,
        session_id: Option<String>,
        session_state: Option<pi_core::SessionState>,
    ) -> Self {
        let model = Model {
            id: pi_core::ModelId::new(model_id),
            name: pi_core::ModelName::new(model_id),
            api: pi_core::ApiName::new("anthropic"),
            provider: pi_core::ProviderName::new("anthropic"),
            base_url: Some(base_url.to_string()),
            reasoning: false,
            context_window: 200_000,
            max_tokens: 16_384,
            capabilities: Default::default(),
            cost: Default::default(),
        };

        let tool_defs = tools::definitions();
        let agent = Agent::new(pi_core::AgentOptions {
            system_prompt: system_prompt.to_string(),
            model,
            tools: tool_defs,
            thinking_level: pi_core::ThinkingLevel::Off,
            steering_mode: pi_core::QueueMode::OneAtATime,
            follow_up_mode: pi_core::QueueMode::OneAtATime,
            tool_execution_mode: pi_core::ToolExecutionMode::Parallel,
            session_id: session_id.as_ref().map(|id| pi_core::SessionId::new(id)),
            messages: Vec::new(),
            session_state,
        });

        let llm_client = LlmClient::new(api_key, base_url, model_id);

        let mut init_entries = vec![ChatEntry::System(
            "Ready. Type a message and press Enter.".into(),
        )];
        if api_key.is_empty() {
            init_entries.push(ChatEntry::System(
                "Warning: ANTHROPIC_API_KEY not set. LLM calls will fail.".into(),
            ));
        }
        if session_id.is_some() {
            init_entries.push(ChatEntry::System(
                "Session loaded. Previous context is active.".into(),
            ));
        }

        Self {
            agent,
            entries: init_entries,
            input: String::new(),
            cursor_pos: 0,
            scroll_offset: 0,
            auto_scroll: true,
            should_quit: false,
            running: false,
            streaming_text: String::new(),
            current_tools: Vec::new(),
            llm_client,
            projection_state: ContextProjectionState::default(),
            budget: ContextProjectionBudget::default(),
            last_usage: None,
            session_id,
            session_backend: FileSystemSessionBackend::new(),
        }
    }

    // -----------------------------------------------------------------------
    // Main event loop
    // -----------------------------------------------------------------------

    pub fn run(
        mut self,
        terminal: &mut ratatui::DefaultTerminal,
        _session_backend: &FileSystemSessionBackend,
    ) -> Result<(), Box<dyn std::error::Error>> {
        loop {
            terminal.draw(|f| self.render(f))?;

            if crossterm::event::poll(Duration::from_millis(33))? {
                let event = crossterm::event::read()?;
                if let crossterm::event::Event::Key(key) = event {
                    if key.kind == KeyEventKind::Press {
                        self.handle_key(terminal, key.code);
                    }
                }
            }

            if self.should_quit {
                self.save_session();
                break;
            }
        }

        Ok(())
    }

    fn handle_key(&mut self, terminal: &mut ratatui::DefaultTerminal, key: KeyCode) {
        match key {
            KeyCode::Enter => {
                if !self.running && !self.input.trim().is_empty() {
                    let text = self.input.clone();
                    self.input.clear();
                    self.cursor_pos = 0;
                    self.submit_prompt(terminal, &text);
                }
            }
            KeyCode::Char(c) => {
                self.input.insert(self.cursor_pos, c);
                self.cursor_pos += c.len_utf8();
            }
            KeyCode::Backspace => {
                if self.cursor_pos > 0 {
                    let prev = self.input[..self.cursor_pos]
                        .chars()
                        .last()
                        .map(|c| c.len_utf8())
                        .unwrap_or(0);
                    self.cursor_pos -= prev;
                    self.input.remove(self.cursor_pos);
                }
            }
            KeyCode::Left => {
                if self.cursor_pos > 0 {
                    self.cursor_pos = self
                        .input
                        .char_indices()
                        .take_while(|(i, _)| *i < self.cursor_pos)
                        .last()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                }
            }
            KeyCode::Right => {
                if self.cursor_pos < self.input.len() {
                    self.cursor_pos = self.input[self.cursor_pos..]
                        .chars()
                        .next()
                        .map(|c| self.cursor_pos + c.len_utf8())
                        .unwrap_or(self.input.len());
                }
            }
            KeyCode::Esc => {
                self.should_quit = true;
            }
            _ => {}
        }
    }

    // -----------------------------------------------------------------------
    // Agent loop (synchronous — calls blocking HTTP via reqwest::blocking)
    // -----------------------------------------------------------------------

    fn submit_prompt(&mut self, terminal: &mut ratatui::DefaultTerminal, text: &str) {
        self.running = true;
        self.auto_scroll = true;
        self.entries.push(ChatEntry::User(text.to_string()));

        // Force immediate redraw so the user message is visible
        let _ = terminal.draw(|f| self.render(f));

        let (_events, actions) = self.agent.start_turn(AgentMessage::user(text));
        self.handle_actions(terminal, actions);
        self.save_session();
    }

    fn handle_actions(
        &mut self,
        terminal: &mut ratatui::DefaultTerminal,
        actions: Vec<AgentAction>,
    ) {
        for action in actions {
            match action {
                AgentAction::StreamLlm { context, .. } => {
                    self.stream_llm(terminal, context);
                }
                AgentAction::ExecuteTools { calls } => {
                    self.execute_tools(terminal, calls);
                }
                AgentAction::Finished { .. } => {
                    self.entries.push(ChatEntry::System("Done.".into()));
                    self.running = false;
                    let _ = terminal.draw(|f| self.render(f));
                }
                AgentAction::WaitForInput { .. } => {
                    self.running = false;
                    let _ = terminal.draw(|f| self.render(f));
                }
                _ => {}
            }
        }
    }

    fn stream_llm(
        &mut self,
        terminal: &mut ratatui::DefaultTerminal,
        context: LlmContext,
    ) {
        // Run context projection
        let projected = project(ProjectionInput {
            system_prompt: context.system_prompt.clone(),
            messages: context.messages.clone(),
            budget: self.budget.clone(),
            state: self.projection_state.clone(),
        });
        self.projection_state = projected.updated_state;

        let projected_messages = projected.projected_messages;

        self.entries.push(ChatEntry::Assistant(Text::raw("...")));
        self.streaming_text.clear();
        let _ = terminal.draw(|f| self.render(f));

        match self.llm_client.stream_sync(
            &context.system_prompt,
            &projected_messages,
            &context.tools,
        ) {
            Ok(mut stream) => {
                let mut full_text = String::new();
                let mut tool_calls = Vec::new();
                let stop_reason;

                for chunk in stream.by_ref() {
                    match chunk {
                        LlmChunk::TextDelta { text } => {
                            full_text.push_str(&text);
                            self.streaming_text = full_text.clone();
                            // Update last assistant entry
                            if let Some(ChatEntry::Assistant(_)) = self.entries.last() {
                                let rendered = markdown::render(&full_text, 80);
                                *self.entries.last_mut().unwrap() =
                                    ChatEntry::Assistant(rendered);
                            }
                        }
                        LlmChunk::ToolCallDelta {
                            tool_call_id,
                            delta,
                        } => {
                            // Accumulate tool call deltas — we'll process them on Done
                            tool_calls.push((tool_call_id, delta));
                        }
                        LlmChunk::Done => break,
                        LlmChunk::Error { message } => {
                            self.entries
                                .push(ChatEntry::System(format!("LLM Error: {message}")));
                            break;
                        }
                        _ => {}
                    }
                    // Redraw after each chunk so streaming is visible
                    let _ = terminal.draw(|f| self.render(f));
                }

                // Collect tool calls from the stream's final state
                let usage = stream.usage();
                if let Some((input, output, total)) = usage {
                    self.last_usage = Some((input, output, total));
                    self.projection_state.last_api_usage = Some(pi_core::ApiUsageSnapshot {
                        estimated_tokens: projected.report.estimated_tokens,
                        actual_input_tokens: input as usize,
                    });
                }

                let stop = stream.stop_reason().unwrap_or("end_turn");
                stop_reason = stop.to_string();

                // Build the final assistant message for core
                let tool_use_blocks: Vec<Content> = stream
                    .tool_calls()
                    .into_iter()
                    .map(|tc| {
                        Content::ToolCall(pi_core::ToolCall {
                            id: pi_core::ToolCallId::new(&tc.id),
                            name: pi_core::ToolName::new(&tc.name),
                            arguments: pi_core::ToolArguments::new(tc.input),
                        })
                    })
                    .collect();

                let text_block = if full_text.is_empty() && tool_use_blocks.is_empty() {
                    vec![Content::Text(pi_core::TextContent {
                        text: String::new(),
                    })]
                } else if full_text.is_empty() {
                    vec![]
                } else {
                    vec![Content::Text(pi_core::TextContent { text: full_text })]
                };

                let content: Vec<Content> =
                    text_block.into_iter().chain(tool_use_blocks).collect();
                let sr = if stop_reason == "tool_use" {
                    pi_core::StopReason::ToolUse
                } else {
                    pi_core::StopReason::EndTurn
                };

                let assistant_msg = pi_core::AssistantMessage {
                    content,
                    api: pi_core::ApiName::new("anthropic"),
                    provider: pi_core::ProviderName::new("anthropic"),
                    model: pi_core::ModelId::new(self.llm_client.model_id()),
                    stop_reason: sr,
                    error_message: None,
                    timestamp: pi_core::timestamp::current_timestamp(),
                    usage: pi_core::message::TokenUsage {
                        input: self.last_usage.map(|(i, _, _)| i).unwrap_or(0),
                        output: self.last_usage.map(|(_, o, _)| o).unwrap_or(0),
                        cache_read: 0,
                        cache_write: 0,
                        total_tokens: self.last_usage.map(|(_, _, t)| t).unwrap_or(0),
                    },
                };

                let result = LlmResult::Ok(assistant_msg);
                let (_events, actions) = self.agent.on_llm_done(result);
                self.handle_actions(terminal, actions);
            }
            Err(e) => {
                let err_result = LlmResult::Err {
                    error: pi_core::LlmError {
                        code: "call_failed".into(),
                        message: e.to_string(),
                        details: None,
                    },
                    aborted: false,
                };
                let (_events, actions) = self.agent.on_llm_done(err_result);
                self.handle_actions(terminal, actions);
            }
        }

        self.streaming_text.clear();
    }

    fn execute_tools(
        &mut self,
        terminal: &mut ratatui::DefaultTerminal,
        calls: Vec<ToolCall>,
    ) {
        for call in &calls {
            let args_summary = format_tool_args(&call.arguments);
            self.entries.push(ChatEntry::ToolStart {
                name: call.name.as_str().to_string(),
                args_summary: args_summary.clone(),
            });
            let _ = terminal.draw(|f| self.render(f));

            let result = tools::execute(call);

            match result {
                Ok(tool_result) => {
                    let output_text = tool_result
                        .content
                        .iter()
                        .filter_map(|c| {
                            if let Content::Text(t) = c {
                                Some(t.text.as_str())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n");

                    let display = if output_text.len() > 500 {
                        format!(
                            "{}...\n({} chars total)",
                            &output_text[..500],
                            output_text.len()
                        )
                    } else {
                        output_text
                    };

                    self.entries.push(ChatEntry::ToolResult {
                        name: call.name.as_str().to_string(),
                        output: display,
                        is_error: false,
                    });
                    let _ = terminal.draw(|f| self.render(f));

                    let (_events, actions) =
                        self.agent.on_tool_done(call.id.clone(), Ok(tool_result));
                    self.handle_actions(terminal, actions);
                }
                Err(err) => {
                    self.entries.push(ChatEntry::ToolResult {
                        name: call.name.as_str().to_string(),
                        output: err.message.clone(),
                        is_error: true,
                    });
                    let _ = terminal.draw(|f| self.render(f));

                    let (_events, actions) = self.agent.on_tool_done(call.id.clone(), Err(err));
                    self.handle_actions(terminal, actions);
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Session persistence
    // -----------------------------------------------------------------------

    fn save_session(&self) {
        if let Some(ref id) = self.session_id {
            let state = self.agent.session_state();
            if let Err(e) = self.session_backend.save(id, state) {
                tracing::warn!(session_id = id.as_str(), error = ?e, "failed to save session");
            }
        }
    }

    // -----------------------------------------------------------------------
    // Rendering
    // -----------------------------------------------------------------------

    fn render(&self, frame: &mut Frame) {
        let [chat_area, input_area, status_area] = Layout::vertical([
            Constraint::Fill(1),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .areas(frame.area());

        self.render_chat(frame, chat_area);
        self.render_input(frame, input_area);
        self.render_status(frame, status_area);
    }

    fn render_chat(&self, frame: &mut Frame, area: Rect) {
        let mut lines: Vec<Line> = Vec::new();

        for entry in &self.entries {
            match entry {
                ChatEntry::User(text) => {
                    lines.push(Line::from(vec![
                        Span::styled(
                            "You",
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(": "),
                    ]));
                    for line in text.lines() {
                        lines.push(Line::from(Span::styled(
                            line.to_string(),
                            Style::default().fg(Color::White),
                        )));
                    }
                    lines.push(Line::raw(""));
                }
                ChatEntry::Assistant(text) => {
                    for line in text.lines.iter().cloned() {
                        lines.push(line);
                    }
                    lines.push(Line::raw(""));
                }
                ChatEntry::ToolStart { name, args_summary } => {
                    lines.push(Line::from(vec![
                        Span::styled(" ┌─ ", Style::default().fg(Color::Yellow)),
                        Span::styled(
                            name.as_str(),
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!(" {args_summary}"),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ]));
                }
                ChatEntry::ToolResult {
                    name: _,
                    output,
                    is_error,
                } => {
                    let color = if *is_error { Color::Red } else { Color::Green };
                    let border = if *is_error { " ┃ " } else { " │ " };
                    for line in output.lines() {
                        lines.push(Line::from(vec![
                            Span::styled(border, Style::default().fg(color)),
                            Span::styled(line.to_string(), Style::default().fg(color)),
                        ]));
                    }
                    lines.push(Line::styled(
                        format!(" └{}─", if *is_error { "─" } else { "─" }),
                        Style::default().fg(color),
                    ));
                    lines.push(Line::raw(""));
                }
                ChatEntry::System(text) => {
                    lines.push(Line::styled(
                        format!("  {text}"),
                        Style::default().fg(Color::DarkGray),
                    ));
                    lines.push(Line::raw(""));
                }
            }
        }

        // Streaming indicator
        if self.running && self.streaming_text.is_empty() {
            lines.push(Line::styled(
                "  ● Thinking...",
                Style::default().fg(Color::DarkGray),
            ));
        }

        let total_lines = lines.len() as u16;
        let visible = area.height.saturating_sub(2); // minus borders

        // Auto-scroll: keep bottom visible
        let scroll = if total_lines > visible {
            if self.auto_scroll {
                total_lines - visible
            } else {
                self.scroll_offset.min(total_lines - visible)
            }
        } else {
            0
        };

        let paragraph = Paragraph::new(Text::from(lines))
            .scroll((scroll, 0))
            .block(Block::new().borders(Borders::NONE))
            .wrap(Wrap { trim: false });

        frame.render_widget(paragraph, area);

        // Scrollbar
        if total_lines > visible {
            let mut scrollbar_state =
                ScrollbarState::new(total_lines as usize).position(scroll as usize);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight).thumb_symbol("█"),
                area,
                &mut scrollbar_state,
            );
        }
    }

    fn render_input(&self, frame: &mut Frame, area: Rect) {
        let style = if self.running {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().fg(Color::White)
        };

        let input = Paragraph::new(self.input.as_str())
            .style(style)
            .block(
                Block::new()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(if self.running {
                        Color::DarkGray
                    } else {
                        Color::Cyan
                    }))
                    .title(if self.running { " thinking... " } else { " > " })
                    .title_style(Style::default().fg(if self.running {
                        Color::Yellow
                    } else {
                        Color::Cyan
                    })),
            )
            .wrap(Wrap { trim: false });

        frame.render_widget(input, area);

        // Cursor
        if !self.running {
            let cursor_x =
                area.x + 1 + (self.cursor_pos as u16).min(area.width.saturating_sub(3));
            let cursor_y = area.y + 1;
            frame.set_cursor_position((cursor_x, cursor_y));
        }
    }

    fn render_status(&self, frame: &mut Frame, area: Rect) {
        let model_name = self.llm_client.model_id();
        let parts = vec![Span::styled(
            format!(" {model_name}"),
            Style::default().fg(Color::DarkGray),
        )];

        let mut spans = parts;

        if let Some((input, output, _total)) = self.last_usage {
            let ctx_pct = if self.budget.max_context_tokens > 0 {
                let est = (input as f64 / self.budget.max_context_tokens as f64 * 100.0) as u16;
                est.min(100)
            } else {
                0
            };
            let ctx_color = if ctx_pct > 90 {
                Color::Red
            } else if ctx_pct > 70 {
                Color::Yellow
            } else {
                Color::Green
            };
            let bar_full = ctx_pct / 10;
            let bar_empty = 10 - bar_full;
            let bar = "█".repeat(bar_full as usize) + &"░".repeat(bar_empty as usize);

            spans.push(Span::raw(" │ "));
            spans.push(Span::styled(
                format!("in:{:.1}k", input as f64 / 1000.0),
                Style::default().fg(Color::DarkGray),
            ));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                format!("out:{:.1}k", output as f64 / 1000.0),
                Style::default().fg(Color::DarkGray),
            ));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                format!("ctx:{ctx_pct}% {bar}"),
                Style::default().fg(ctx_color),
            ));
        }

        if self.running {
            spans.push(Span::styled(" ●", Style::default().fg(Color::Yellow)));
        }

        let status = Paragraph::new(Line::from(spans)).style(Style::default().bg(Color::DarkGray));
        frame.render_widget(status, area);
    }
}

fn format_tool_args(args: &pi_core::ToolArguments) -> String {
    let obj = match args.0.as_object() {
        Some(o) => o,
        None => return serde_json::to_string(&args.0).unwrap_or_default(),
    };
    obj.iter()
        .take(3)
        .map(|(k, v)| {
            let s = match v.as_str() {
                Some(s) => s.to_string(),
                None => v.to_string(),
            };
            let truncated = if s.len() > 60 { &s[..60] } else { &s };
            format!("{k}={truncated}")
        })
        .collect::<Vec<_>>()
        .join(", ")
}
