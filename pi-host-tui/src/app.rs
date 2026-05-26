use std::path::Path;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarOrientation,
    ScrollbarState, Wrap,
};
use ratatui::Frame;

use pi_core::{
    project, AgentAction, AgentMessage, AgentRuntime, Content, ContextProjectionBudget,
    ContextProjectionState, LlmChunk, LlmContext, LlmResult, Model, ProjectionInput, ToolCall,
    ToolTransition, UserInputDuringTools, WaitMode,
};

use crate::extension::{BashExtension, BuiltinExtension, Extension, ExtensionContext, ToolEvent};
use crate::llm::LlmClient;
use crate::markdown;
use crate::session::FileSystemSessionBackend;

// ---------------------------------------------------------------------------
// Command palette
// ---------------------------------------------------------------------------

const COMMANDS: &[&str] = &[
    "/clear",
    "/help",
    "/model",
    "/quit",
    "/session list",
    "/session load",
    "/session new",
    "/tokens",
    "/undo",
];

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
    agent: Option<AgentRuntime>,
    entries: Vec<ChatEntry>,
    input: String,
    cursor_pos: usize,
    scroll_offset: u16,
    auto_scroll: bool,
    should_quit: bool,
    running: bool,
    streaming_text: String,
    #[allow(dead_code)]
    current_tools: Vec<(String, String)>,
    llm_client: LlmClient,
    projection_state: ContextProjectionState,
    budget: ContextProjectionBudget,
    last_usage: Option<(u32, u32, u32)>,
    session_id: Option<String>,
    session_backend: FileSystemSessionBackend,
    cwd: std::path::PathBuf,

    // New: cancellation
    cancelled: bool,

    // New: history recall
    history: Vec<String>,
    history_index: Option<usize>,
    original_input: String,

    // New: command autocomplete
    suggestions: Vec<String>,
    show_suggestions: bool,
    suggestion_state: ListState,

    // Extension-based tool execution
    extensions: Vec<Box<dyn Extension>>,
    running_tasks: Vec<RunningTask>,
}

struct RunningTask {
    tool_call_id: pi_core::ToolCallId,
    tool_name: String,
    stream: Box<dyn crate::extension::ToolEventStream>,
}

impl App {
    fn agent(&self) -> &AgentRuntime {
        self.agent.as_ref().unwrap()
    }

    fn agent_mut(&mut self) -> &mut AgentRuntime {
        self.agent.as_mut().unwrap()
    }

    pub fn new(
        system_prompt: &str,
        model_id: &str,
        api_key: &str,
        base_url: &str,
        session_id: Option<String>,
        session_state: Option<pi_core::SessionState>,
        cwd: &Path,
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

        let extensions: Vec<Box<dyn Extension>> = vec![
            Box::new(BuiltinExtension::new()),
            Box::new(BashExtension::new()),
        ];
        let mut tool_defs = Vec::new();
        for ext in &extensions {
            tool_defs.extend(ext.tools());
        }
        let agent = AgentRuntime::new(pi_core::AgentOptions {
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
            "Ready. Type a message and press Enter.  /help for commands.".into(),
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
            agent: Some(agent),
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
            cwd: cwd.to_path_buf(),
            cancelled: false,
            history: Vec::new(),
            history_index: None,
            original_input: String::new(),
            suggestions: Vec::new(),
            show_suggestions: false,
            suggestion_state: ListState::default(),
            extensions,
            running_tasks: Vec::new(),
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
                        self.handle_key(terminal, key);
                    }
                }
            }

            // Poll running async tasks
            self.poll_running_tasks(terminal);

            if self.should_quit {
                self.save_session();
                break;
            }
        }

        Ok(())
    }

    fn handle_key(&mut self, terminal: &mut ratatui::DefaultTerminal, key: KeyEvent) {
        // Ctrl+C: cancel running LLM
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            if self.running {
                self.cancelled = true;
            }
            return;
        }

        match key.code {
            KeyCode::Enter => {
                if self.show_suggestions {
                    if let Some(idx) = self.suggestion_state.selected() {
                        if let Some(cmd) = self.suggestions.get(idx).cloned() {
                            self.input = cmd;
                            self.cursor_pos = self.input.len();
                            self.show_suggestions = false;
                            return;
                        }
                    }
                }
                if !self.running && !self.input.trim().is_empty() {
                    let text = self.input.clone();
                    self.input.clear();
                    self.cursor_pos = 0;
                    self.show_suggestions = false;
                    self.submit_prompt(terminal, &text);
                }
            }
            KeyCode::Tab => {
                if self.input.starts_with('/') {
                    self.update_suggestions();
                } else if self.show_suggestions {
                    self.suggestion_state.select_next();
                }
            }
            KeyCode::Up => {
                if self.show_suggestions {
                    self.suggestion_state.select_previous();
                } else {
                    self.history_recall_previous();
                }
            }
            KeyCode::Down => {
                if self.show_suggestions {
                    self.suggestion_state.select_next();
                } else {
                    self.history_recall_next();
                }
            }
            KeyCode::Char(c) => {
                self.input.insert(self.cursor_pos, c);
                self.cursor_pos += c.len_utf8();
                if self.show_suggestions && !self.input.starts_with('/') {
                    self.show_suggestions = false;
                } else if self.input.starts_with('/') {
                    self.update_suggestions();
                }
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
                if self.input.is_empty() || !self.input.starts_with('/') {
                    self.show_suggestions = false;
                } else {
                    self.update_suggestions();
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
                if self.show_suggestions {
                    self.show_suggestions = false;
                } else {
                    self.should_quit = true;
                }
            }
            _ => {}
        }
    }

    // -----------------------------------------------------------------------
    // History recall
    // -----------------------------------------------------------------------

    fn history_recall_previous(&mut self) {
        if self.history.is_empty() {
            return;
        }
        if self.history_index.is_none() {
            self.original_input = self.input.clone();
            self.history_index = Some(self.history.len().saturating_sub(1));
        } else {
            self.history_index = Some(self.history_index.unwrap().saturating_sub(1));
        }
        if let Some(idx) = self.history_index {
            self.input = self.history[idx].clone();
            self.cursor_pos = self.input.len();
        }
    }

    fn history_recall_next(&mut self) {
        match self.history_index {
            None => {}
            Some(idx) => {
                if idx + 1 < self.history.len() {
                    self.history_index = Some(idx + 1);
                    self.input = self.history[idx + 1].clone();
                    self.cursor_pos = self.input.len();
                } else {
                    self.input = self.original_input.clone();
                    self.cursor_pos = self.input.len();
                    self.history_index = None;
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Suggestions
    // -----------------------------------------------------------------------

    fn update_suggestions(&mut self) {
        if !self.input.starts_with('/') {
            self.show_suggestions = false;
            self.suggestions.clear();
            self.suggestion_state.select(None);
            return;
        }
        let filtered: Vec<String> = COMMANDS
            .iter()
            .filter(|c| c.starts_with(&self.input))
            .cloned()
            .map(|s| s.to_string())
            .collect();
        self.show_suggestions = !filtered.is_empty();
        self.suggestions = filtered;
        self.suggestion_state
            .select(self.show_suggestions.then_some(0));
    }

    // -----------------------------------------------------------------------
    // Agent loop
    // -----------------------------------------------------------------------

    fn submit_prompt(&mut self, terminal: &mut ratatui::DefaultTerminal, text: &str) {
        // Command dispatch
        if text.starts_with('/') {
            self.handle_command(terminal, text);
            return;
        }

        self.running = true;
        self.auto_scroll = true;
        self.cancelled = false;
        self.entries.push(ChatEntry::User(text.to_string()));
        self.history.push(text.to_string());
        self.history_index = None;
        self.original_input.clear();

        let _ = terminal.draw(|f| self.render(f));

        let runtime = self.agent.take().unwrap();
        let (_events, actions, new_runtime) = match runtime {
            AgentRuntime::Idle(idle) => {
                let t = idle.start_turn(AgentMessage::user(text));
                (t.events, t.actions, t.state.into_runtime())
            }
            AgentRuntime::ReadyToContinue(ready) => {
                let t1 = ready.wait_for_input();
                let t2 = t1.state.start_turn(AgentMessage::user(text));
                (t2.events, t2.actions, t2.state.into_runtime())
            }
            AgentRuntime::Finished(finished) => {
                let t1 = finished.restart();
                let t2 = t1.state.start_turn(AgentMessage::user(text));
                (t2.events, t2.actions, t2.state.into_runtime())
            }
            AgentRuntime::Aborted(aborted) => {
                let t1 = aborted.restart();
                let t2 = t1.state.start_turn(AgentMessage::user(text));
                (t2.events, t2.actions, t2.state.into_runtime())
            }
            AgentRuntime::WaitingTools(mut waiting) => {
                let disposition = waiting.submit_user_message(AgentMessage::user(text));
                let (events, actions) = disposition.into_events_actions();
                (events, actions, waiting.into_runtime())
            }
            other => (
                vec![],
                vec![AgentAction::WaitForInput {
                    mode: WaitMode::Any,
                }],
                other,
            ),
        };
        self.agent = Some(new_runtime);
        self.handle_actions(terminal, actions);
        self.save_session();
    }

    fn handle_command(&mut self, terminal: &mut ratatui::DefaultTerminal, text: &str) {
        let parts: Vec<&str> = text.split_whitespace().collect();
        let cmd = parts.first().copied().unwrap_or("");

        match cmd {
            "/clear" => {
                let agent = self.agent.take().unwrap().reset();
                self.agent = Some(agent);
                self.entries.clear();
                self.entries.push(ChatEntry::System("Chat cleared.".into()));
            }
            "/help" => {
                let list = COMMANDS.join("  ");
                self.entries
                    .push(ChatEntry::System(format!("Commands: {list}")));
            }
            "/quit" => {
                self.should_quit = true;
            }
            "/model" => {
                if parts.len() >= 2 {
                    let model_id = parts[1];
                    self.llm_client.set_model(model_id);
                    self.agent_mut().state_mut().model.id = pi_core::ModelId::new(model_id);
                    self.agent_mut().state_mut().model.name = pi_core::ModelName::new(model_id);
                    self.entries
                        .push(ChatEntry::System(format!("Model switched to {model_id}")));
                } else {
                    self.entries
                        .push(ChatEntry::System("Usage: /model <model_id>".into()));
                }
            }
            "/session" => {
                let sub = parts.get(1).copied().unwrap_or("");
                match sub {
                    "list" => {
                        let ids = self.session_backend.list();
                        let msg = if ids.is_empty() {
                            "No saved sessions.".into()
                        } else {
                            format!("Sessions: {}", ids.join(", "))
                        };
                        self.entries.push(ChatEntry::System(msg));
                    }
                    "load" => {
                        if let Some(id) = parts.get(2) {
                            if let Some(state) = self.session_backend.load(id) {
                                let agent = self.agent.take().unwrap().reset();
                                self.agent = Some(agent);
                                self.agent_mut().set_session_state(state);
                                self.session_id = Some(id.to_string());
                                self.entries.clear();
                                self.entries
                                    .push(ChatEntry::System(format!("Session '{id}' loaded.")));
                            } else {
                                self.entries
                                    .push(ChatEntry::System(format!("Session '{id}' not found.")));
                            }
                        } else {
                            self.entries
                                .push(ChatEntry::System("Usage: /session load <id>".into()));
                        }
                    }
                    "new" => {
                        let agent = self.agent.take().unwrap().reset();
                        self.agent = Some(agent);
                        self.session_id = None;
                        self.entries.clear();
                        self.entries
                            .push(ChatEntry::System("New session started.".into()));
                    }
                    _ => {
                        self.entries.push(ChatEntry::System(
                            "Usage: /session list | load <id> | new".into(),
                        ));
                    }
                }
            }
            "/tokens" => {
                if let Some((input, output, total)) = self.last_usage {
                    let ctx_pct = if self.budget.max_context_tokens > 0 {
                        (input as f64 / self.budget.max_context_tokens as f64 * 100.0) as u16
                    } else {
                        0
                    };
                    self.entries.push(ChatEntry::System(format!(
                        "Tokens: in={input} out={output} total={total} ctx={ctx_pct}%"
                    )));
                } else {
                    self.entries
                        .push(ChatEntry::System("No token usage recorded yet.".into()));
                }
            }
            "/undo" => {
                let msgs = &self.agent_mut().state().messages;
                if let Some(last_user_idx) = msgs
                    .iter()
                    .rposition(|m| matches!(m, pi_core::AgentMessage::User(_)))
                {
                    let new_len = last_user_idx;
                    self.agent_mut().state_mut().messages.truncate(new_len);
                    // Also truncate entries to remove the last user+assistant+tools round
                    if let Some(last_user_entry) = self
                        .entries
                        .iter()
                        .rposition(|e| matches!(e, ChatEntry::User(_)))
                    {
                        self.entries.truncate(last_user_entry);
                    }
                    self.entries
                        .push(ChatEntry::System("Last turn undone.".into()));
                } else {
                    self.entries
                        .push(ChatEntry::System("Nothing to undo.".into()));
                }
            }
            _ => {
                self.entries.push(ChatEntry::System(format!(
                    "Unknown command: {cmd}. Type /help for list."
                )));
            }
        }

        let _ = terminal.draw(|f| self.render(f));
    }

    fn handle_actions(
        &mut self,
        terminal: &mut ratatui::DefaultTerminal,
        actions: Vec<AgentAction>,
    ) {
        for action in actions {
            if self.cancelled {
                let runtime = self.agent.take().unwrap();
                let (_events, new_runtime) = match runtime {
                    AgentRuntime::Streaming(streaming) => {
                        let t = streaming.abort();
                        (t.events, t.state.into_runtime())
                    }
                    other => (vec![], other),
                };
                self.agent = Some(new_runtime);
                self.running = false;
                self.entries.push(ChatEntry::System("Cancelled.".into()));
                let _ = terminal.draw(|f| self.render(f));
                return;
            }
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

    fn stream_llm(&mut self, terminal: &mut ratatui::DefaultTerminal, context: LlmContext) {
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
                        let runtime = self.agent.take().unwrap();
                        let (_events, new_runtime) = match runtime {
                            AgentRuntime::Streaming(streaming) => {
                                let t = streaming.abort();
                                (t.events, t.state.into_runtime())
                            }
                            other => (vec![], other),
                        };
                        self.agent = Some(new_runtime);
                        self.running = false;
                        self.entries.push(ChatEntry::System("Cancelled.".into()));
                        let _ = terminal.draw(|f| self.render(f));
                        return;
                    }
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
                            break;
                        }
                        _ => {}
                    }
                    let _ = terminal.draw(|f| self.render(f));
                }

                let usage = stream.usage();
                if let Some((input, output, total)) = usage {
                    self.last_usage = Some((input, output, total));
                    self.projection_state.last_api_usage = Some(pi_core::ApiUsageSnapshot {
                        estimated_tokens: projected.report.estimated_tokens,
                        actual_input_tokens: input as usize,
                    });
                }

                let stop_reason = stream.stop_reason().unwrap_or("end_turn");

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

                let content: Vec<Content> = text_block.into_iter().chain(tool_use_blocks).collect();
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
                let runtime = self.agent.take().unwrap();
                let (_events, actions, new_runtime) = match runtime {
                    AgentRuntime::Streaming(streaming) => {
                        let transition = streaming.finish_llm(result);
                        transition.into_parts()
                    }
                    other => (vec![], vec![], other),
                };
                self.agent = Some(new_runtime);
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
                let runtime = self.agent.take().unwrap();
                let (_events, actions, new_runtime) = match runtime {
                    AgentRuntime::Streaming(streaming) => {
                        let transition = streaming.finish_llm(err_result);
                        transition.into_parts()
                    }
                    other => (vec![], vec![], other),
                };
                self.agent = Some(new_runtime);
                self.handle_actions(terminal, actions);
            }
        }

        self.streaming_text.clear();
    }

    fn execute_tools(&mut self, terminal: &mut ratatui::DefaultTerminal, calls: Vec<ToolCall>) {
        for call in calls {
            let args_summary = format_tool_args(&call.arguments);
            self.entries.push(ChatEntry::ToolStart {
                name: call.name.as_str().to_string(),
                args_summary: args_summary.clone(),
            });
            let _ = terminal.draw(|f| self.render(f));

            // Find the extension that handles this tool
            let ext_idx = self
                .extensions
                .iter()
                .position(|ext| ext.tools().iter().any(|def| def.name == call.name));

            let ctx = ExtensionContext {
                cwd: self.cwd.clone(),
            };

            if let Some(idx) = ext_idx {
                let outcome = self.extensions[idx].execute(&call, &ctx);
                match outcome {
                    crate::extension::ExtensionOutcome::Complete(result) => {
                        self.on_tool_result(terminal, call.id, result);
                    }
                    crate::extension::ExtensionOutcome::Running(stream) => {
                        self.running_tasks.push(RunningTask {
                            tool_call_id: call.id.clone(),
                            tool_name: call.name.as_str().to_string(),
                            stream,
                        });
                    }
                }
            } else {
                self.on_tool_result(
                    terminal,
                    call.id,
                    Err(pi_core::ToolError::new(
                        "unknown_tool",
                        format!("No extension provides tool: {}", call.name.as_str()),
                    )),
                );
            }
        }

        // If async tools are running, let the user keep typing
        if !self.running_tasks.is_empty() {
            self.running = false;
        }

        // If all tools were sync and are now done, auto-continue
        if self.agent().state().pending_tool_calls.is_empty() && self.running_tasks.is_empty() {
            let runtime = self.agent.take().unwrap();
            let (_events, actions, new_runtime) = match runtime {
                AgentRuntime::ReadyToContinue(ready) => {
                    let t = ready.continue_turn();
                    (t.events, t.actions, t.state.into_runtime())
                }
                other => (vec![], vec![], other),
            };
            self.agent = Some(new_runtime);
            self.handle_actions(terminal, actions);
        }
    }

    fn on_tool_result(
        &mut self,
        terminal: &mut ratatui::DefaultTerminal,
        tool_call_id: pi_core::ToolCallId,
        result: Result<pi_core::ToolResult, pi_core::ToolError>,
    ) {
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
                    name: tool_call_id.as_str().to_string(),
                    output: display,
                    is_error: false,
                });
                let _ = terminal.draw(|f| self.render(f));

                let runtime = self.agent.take().unwrap();
                let (_events, actions, new_runtime) = match runtime {
                    AgentRuntime::WaitingTools(waiting) => {
                        let transition = waiting.on_tool_done(tool_call_id, Ok(tool_result));
                        transition.into_parts()
                    }
                    other => (vec![], vec![], other),
                };
                self.agent = Some(new_runtime);
                self.handle_actions(terminal, actions);
            }
            Err(err) => {
                self.entries.push(ChatEntry::ToolResult {
                    name: tool_call_id.as_str().to_string(),
                    output: err.message.clone(),
                    is_error: true,
                });
                let _ = terminal.draw(|f| self.render(f));

                let runtime = self.agent.take().unwrap();
                let (_events, actions, new_runtime) = match runtime {
                    AgentRuntime::WaitingTools(waiting) => {
                        let transition = waiting.on_tool_done(tool_call_id, Err(err));
                        transition.into_parts()
                    }
                    other => (vec![], vec![], other),
                };
                self.agent = Some(new_runtime);
                self.handle_actions(terminal, actions);
            }
        }
    }

    fn poll_running_tasks(&mut self, terminal: &mut ratatui::DefaultTerminal) {
        let mut remaining = Vec::new();
        let mut just_completed: Vec<(
            pi_core::ToolCallId,
            Result<pi_core::ToolResult, pi_core::ToolError>,
        )> = Vec::new();

        for mut task in std::mem::take(&mut self.running_tasks) {
            let mut done = false;
            while let Some(event) = task.stream.try_recv() {
                match event {
                    ToolEvent::Update(update) => {
                        let _events = self.agent_mut().on_tool_update(update);
                    }
                    ToolEvent::Done(result) => {
                        just_completed.push((task.tool_call_id.clone(), result));
                        done = true;
                        break;
                    }
                }
            }
            if !done {
                remaining.push(task);
            }
        }
        self.running_tasks = remaining;

        // Process completed tasks — UI first, then typestate transitions
        for (tool_call_id, result) in &just_completed {
            let output_text = match result {
                Ok(r) => r
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
                    .join("\n"),
                Err(e) => e.message.clone(),
            };
            self.entries.push(ChatEntry::ToolResult {
                name: tool_call_id.as_str().to_string(),
                output: output_text,
                is_error: result.is_err(),
            });
            let _ = terminal.draw(|f| self.render(f));
        }

        // Apply typestate transitions for all completed async tools
        let mut runtime = self.agent.take().unwrap();
        for (tool_call_id, result) in just_completed {
            runtime = match runtime {
                AgentRuntime::WaitingTools(waiting) => {
                    let transition = waiting.on_tool_done(tool_call_id, result);
                    match transition {
                        ToolTransition::WaitingTools(t) => t.state.into_runtime(),
                        ToolTransition::Ready(t) => t.state.into_runtime(),
                        ToolTransition::Finished(t) => t.state.into_runtime(),
                    }
                }
                other => other,
            };
        }

        // Auto-continue if all pending tools are done and phase is Ready
        match runtime {
            AgentRuntime::ReadyToContinue(ready) => {
                let transition = ready.continue_turn();
                self.agent = Some(transition.state.into_runtime());
                self.handle_actions(terminal, transition.actions);
            }
            other => {
                self.agent = Some(other);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Session persistence
    // -----------------------------------------------------------------------

    fn save_session(&self) {
        if let Some(ref id) = self.session_id {
            let state = self.agent().session_state();
            if let Err(e) = self.session_backend.save(id, state) {
                tracing::warn!(session_id = id.as_str(), error = ?e, "failed to save session");
            }
        }
    }

    // -----------------------------------------------------------------------
    // Rendering
    // -----------------------------------------------------------------------

    fn render(&mut self, frame: &mut Frame) {
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

        // Streaming indicator (only when LLM is actually streaming, not waiting for async tools)
        if self.running && self.streaming_text.is_empty() && self.running_tasks.is_empty() {
            lines.push(Line::styled(
                "  ● Thinking...",
                Style::default().fg(Color::DarkGray),
            ));
        }

        let total_lines = lines.len() as u16;
        let visible = area.height.saturating_sub(2);

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

    fn render_input(&mut self, frame: &mut Frame, area: Rect) {
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
                    } else if self.show_suggestions {
                        Color::Yellow
                    } else {
                        Color::Cyan
                    }))
                    .title(if self.running {
                        " thinking... "
                    } else if self.show_suggestions {
                        " commands "
                    } else {
                        " > "
                    })
                    .title_style(Style::default().fg(if self.running {
                        Color::Yellow
                    } else if self.show_suggestions {
                        Color::Yellow
                    } else {
                        Color::Cyan
                    })),
            )
            .wrap(Wrap { trim: false });

        frame.render_widget(input, area);

        if !self.running {
            let cursor_x = area.x + 1 + (self.cursor_pos as u16).min(area.width.saturating_sub(3));
            let cursor_y = area.y + 1;
            frame.set_cursor_position((cursor_x, cursor_y));
        }

        // Suggestion popup
        if self.show_suggestions && !self.suggestions.is_empty() {
            let max_visible = 5u16;
            let list_height = (self.suggestions.len() as u16).min(max_visible);
            let popup_height = list_height + 2;

            let popup_area = Rect {
                x: area.x,
                y: area.y.saturating_sub(popup_height),
                width: area.width,
                height: popup_height,
            };

            frame.render_widget(Clear, popup_area);

            let items: Vec<ListItem> = self
                .suggestions
                .iter()
                .map(|s| ListItem::new(s.as_str()))
                .collect();

            let list = List::new(items)
                .block(Block::bordered().title(" commands "))
                .highlight_style(
                    Style::new()
                        .add_modifier(Modifier::REVERSED)
                        .fg(Color::Cyan),
                )
                .highlight_symbol("> ");

            frame.render_stateful_widget(list, popup_area, &mut self.suggestion_state);
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

        if !self.running_tasks.is_empty() {
            let count = self.running_tasks.len();
            spans.push(Span::styled(
                format!(" ● {count} tools"),
                Style::default().fg(Color::Yellow),
            ));
        } else if self.running {
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
