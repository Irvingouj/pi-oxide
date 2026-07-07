use std::path::Path;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Text;
use ratatui::Frame;
use thiserror::Error;

use pi_core::{
    AgentAction, AgentMessage, AgentOptions, AgentRuntime, ApiName,
    ContextProjectionBudget, ExecutionMode, Model, ModelId, ModelName, ProviderName, QueueMode,
    SessionId, ThinkingLevel, ToolCallId, ToolCallPermission, ToolCallPreparation,
    ToolCallTransform, ToolDefinition, WaitMode,
};

use crate::agent_host::TransitionParts;
use crate::config::ResolvedConfig;
use crate::extension::{BashExtension, BuiltinExtension, Extension};
use crate::host_state::{HostDirective, HostState};
use crate::llm::{LlmClient, WireFormat};
use crate::session::FileSystemSessionBackend;
use crate::session_log::{SessionEvent, SessionEventLogger};

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Concrete error type for TUI operations.
#[derive(Debug, Error)]
pub enum TuiError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}



// ---------------------------------------------------------------------------
// Chat line types
// ---------------------------------------------------------------------------

pub(crate) enum ChatEntry {
    User(String),
    Assistant(Text<'static>),
    ToolStart {
        name: String,
        args_summary: String,
    },
    ToolResult {
        #[allow(dead_code)] // stored for future UI rendering of tool name
        name: String,
        output: String,
        is_error: bool,
    },
    System(String),
}

/// Approximate wrapped line count using Unicode display width.
/// Wide characters (CJK, emoji) count as 2 columns.
fn wrapped_lines(text: &str, width: usize) -> usize {
    if text.is_empty() {
        return 1;
    }
    let width = width.max(1);
    let display_len = unicode_width::UnicodeWidthStr::width(text);
    (display_len.saturating_add(width.saturating_sub(1))) / width
}

impl ChatEntry {
    /// Approximate wrapped line count at the given width.
    pub(crate) fn line_count(&self, width: usize) -> u16 {
        match self {
            ChatEntry::User(text) => {
                let mut count: u16 = 2; // header "▌ You:" + blank
                                        // Accent bar "▌ " (2 chars) prepended to each content line
                let eff = width.saturating_sub(2);
                for line in text.lines() {
                    count += wrapped_lines(line, eff.max(1)) as u16;
                }
                count
            }
            ChatEntry::Assistant(text) => {
                let mut count: u16 = 0;
                // Accent bar "▌ " (2 chars) prepended to each line
                let eff = width.saturating_sub(2);
                for line in &text.lines {
                    let s: String = line
                        .spans
                        .iter()
                        .map(|s| (*s.content).to_string())
                        .collect();
                    count += wrapped_lines(&s, eff.max(1)).max(1) as u16;
                }
                count + 1 // blank
            }
            ChatEntry::ToolStart { name, args_summary } => {
                // Must match emit_entry: " ╭─ {name} {args}"
                let full = format!(" ╭─ {} {}", name, args_summary);
                wrapped_lines(&full, width) as u16
            }
            ChatEntry::ToolResult {
                output,
                is_error: _,
                ..
            } => {
                let mut count: u16 = 0;
                for line in output.lines() {
                    // Must match emit_entry: " │ {line}"
                    let full = format!(" │ {}", line);
                    count += wrapped_lines(&full, width) as u16;
                }
                // Footer: " ╰─✓" or " ╰─✗", plus blank line
                count + 2
            }
            ChatEntry::System(text) => {
                // Must match emit_entry: "◇ {text}"
                let full = format!("◇ {}", text);
                wrapped_lines(&full, width) as u16 + 1 // + blank
            }
        }
    }
}

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

pub struct App {
    /// Host-side agent mediator — owns runtime, transcript, artifacts, turn_number.
    pub(crate) agent_host: Option<crate::agent_host::AgentHost>,
    pub(crate) entries: Vec<ChatEntry>,
    pub(crate) editor: crate::editor::Editor,
    pub(crate) scroll_offset: u16,
    pub(crate) auto_scroll: bool,
    pub(crate) should_quit: bool,
    pub(crate) running: bool,
    pub(crate) streaming_text: String,
    pub(crate) streaming_start: Option<std::time::Instant>,
    #[allow(dead_code)]
    pub(crate) current_tools: Vec<(String, String)>,
    pub(crate) tool_definitions: Vec<ToolDefinition>,
    pub(crate) llm_client: crate::llm::LlmBackend,
    pub(crate) host_state: Option<HostState>,
    pub(crate) last_usage: Option<(u32, u32, u32)>,
    pub(crate) session_id: Option<String>,
    pub(crate) session_backend: FileSystemSessionBackend,
    pub(crate) cwd: std::path::PathBuf,

    // Cancellation
    pub(crate) cancelled: bool,

    // Model picker
    pub(crate) model_picker: Option<crate::model_picker::ModelPicker>,

    // Extension-based tool execution
    pub(crate) extensions: Vec<Box<dyn Extension>>,
    pub(crate) running_tasks: Vec<RunningTask>,

    // Session event logger
    pub(crate) session_logger: Option<SessionEventLogger>,

    // Context budget and window
    pub(crate) budget: ContextProjectionBudget,
    pub(crate) context_window: u32,
    /// Cached chat area from the last render frame.
    pub(crate) last_chat_area: Rect,
    pub(crate) resolved_config: ResolvedConfig,

    // Thinking level for input border coloring
    pub(crate) thinking_level: ThinkingLevel,
}

pub(crate) struct RunningTask {
    pub(crate) tool_call_id: ToolCallId,
    pub(crate) tool_name: String,
    pub(crate) stream: Box<dyn crate::extension::ToolEventStream>,
}

impl App {
    /// Get the current Braille spinner frame based on elapsed time.
    pub(crate) fn get_spinner_frame(&self) -> &'static str {
        const FRAMES: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
        let elapsed_ms = self
            .streaming_start
            .map(|start| start.elapsed().as_millis() as usize)
            .unwrap_or(0);
        FRAMES[(elapsed_ms / 120) % FRAMES.len()]
    }

    pub(crate) fn agent(&self) -> &AgentRuntime {
        self.agent_host.as_ref().expect("agent").runtime()
    }

    pub(crate) fn agent_mut(&mut self) -> &mut AgentRuntime {
        self.agent_host.as_mut().expect("agent").runtime_mut()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        system_prompt: &str,
        model_id: &str,
        api_key: &str,
        base_url: &str,
        session_id: Option<String>,
        host_state: Option<HostState>,
        cwd: &Path,
        wire_format: WireFormat,
        provider: &str,
        #[cfg(feature = "record")] record_to: Option<std::path::PathBuf>,
        #[cfg(feature = "replay")] replay_from: Option<std::path::PathBuf>,
        resolved_config: ResolvedConfig,
    ) -> Result<Self, TuiError> {
        let model = Model {
            id: ModelId::new(model_id),
            name: ModelName::new(model_id),
            api: ApiName::new(provider),
            provider: ProviderName::new(provider),
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
        let context_window = model.context_window;

        let agent = AgentRuntime::new(AgentOptions {
            system_prompt: system_prompt.to_string(),
            model,
            thinking_level: ThinkingLevel::Off,
            steering_mode: QueueMode::OneAtATime,
            follow_up_mode: QueueMode::OneAtATime,
            tool_execution_mode: ExecutionMode::Parallel,
            session_id: session_id.as_ref().map(SessionId::new),
        });

        let llm_client = Self::build_llm_client(
            api_key,
            base_url,
            model_id,
            wire_format,
            #[cfg(feature = "record")]
            record_to,
            #[cfg(feature = "replay")]
            replay_from,
        )?;

        let mut init_entries = vec![ChatEntry::System(
            "Ready. Type a message and press Enter.  /help for commands.".into(),
        )];
        if api_key.is_empty() {
            init_entries.push(ChatEntry::System(
                "Warning: API key not set. LLM calls will fail.".into(),
            ));
        }
        if session_id.is_some() {
            init_entries.push(ChatEntry::System(
                "Session loaded. Previous context is active.".into(),
            ));
        }

        let agent_host = crate::agent_host::AgentHost::new(agent);

        Ok(Self {
            agent_host: Some(agent_host),
            entries: init_entries,
            editor: crate::editor::Editor::new(),
            scroll_offset: 0,
            auto_scroll: true,
            should_quit: false,
            running: false,
            streaming_text: String::new(),
            streaming_start: None,
            current_tools: Vec::new(),
            tool_definitions: tool_defs,
            llm_client,
            host_state: Some(host_state.unwrap_or_else(|| HostState::new(system_prompt.to_string(), "Summarize the following conversation into a concise summary that preserves the key information, decisions, and context.".to_string()))),
            last_usage: None,
            session_logger: session_id
                .as_ref()
                .and_then(|id| SessionEventLogger::new(id).ok()),
            session_id,
            session_backend: FileSystemSessionBackend::new(),
            cwd: cwd.to_path_buf(),
            cancelled: false,
            model_picker: None,
            extensions,
            running_tasks: Vec::new(),
            budget: ContextProjectionBudget::default(),
            context_window,
            last_chat_area: ratatui::layout::Rect::ZERO,
            resolved_config,
            thinking_level: ThinkingLevel::Off,
        })
    }

    #[allow(unused_variables)]
    fn build_llm_client(
        api_key: &str,
        base_url: &str,
        model_id: &str,
        wire_format: WireFormat,
        #[cfg(feature = "record")] record_to: Option<std::path::PathBuf>,
        #[cfg(feature = "replay")] replay_from: Option<std::path::PathBuf>,
    ) -> Result<crate::llm::LlmBackend, TuiError> {
        #[cfg(not(any(feature = "record", feature = "replay")))]
        {
            Ok(LlmClient::new(api_key, base_url, model_id, wire_format))
        }
        #[cfg(feature = "record")]
        {
            Ok(crate::llm_record::RecordingLlmClient::new(
                api_key,
                base_url,
                model_id,
                wire_format,
                record_to.unwrap_or_else(|| std::path::PathBuf::from("cassette.json")),
            ))
        }
        #[cfg(all(feature = "replay", not(feature = "record")))]
        {
            Ok(crate::llm_replay::ReplayLlmClient::load(
                replay_from
                    .as_deref()
                    .expect("replay mode requires --replay-from <path>"),
            )?)
        }
    }

    // -----------------------------------------------------------------------
    // Main event loop
    // -----------------------------------------------------------------------

    pub fn run(
        mut self,
        terminal: &mut ratatui::DefaultTerminal,
        _session_backend: &FileSystemSessionBackend,
    ) -> Result<(), TuiError> {
        loop {
            terminal.draw(|f| self.render(f))?;

            if crossterm::event::poll(Duration::from_millis(33))? {
                let event = crossterm::event::read()?;
                if let crossterm::event::Event::Key(key) = event {
                    if key.kind == KeyEventKind::Press && !self.handle_key(key) {
                        self.handle_terminal_key(terminal, key);
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

    /// Handle keys that don't need the terminal. Returns `true` if the key was consumed.
    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> bool {
        let is_ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let is_alt = key.modifiers.contains(KeyModifiers::ALT);
        let is_shift = key.modifiers.contains(KeyModifiers::SHIFT);

        // Ctrl+C: cancel running LLM, or quit when idle
        if key.code == KeyCode::Char('c') && is_ctrl {
            if self.running {
                self.cancelled = true;
            } else {
                self.should_quit = true;
            }
            return true;
        }

        // Scroll keys — handled before the main match
        // Ctrl+B/F are now cursor movement, not scroll
        if let Some(intent) = crate::scroll::derive_scroll_intent(&key) {
            let visible = self.last_chat_area.height.saturating_sub(2);
            let total_lines = self.wrapped_line_count(self.last_chat_area.width as usize);
            let (off, auto) = crate::scroll::apply_scroll(
                intent,
                total_lines,
                visible,
                self.scroll_offset,
                self.auto_scroll,
            );
            self.scroll_offset = off;
            self.auto_scroll = auto;
            return true;
        }

        // Model picker keys — handled before the main match
        if self.model_picker.is_some() {
            return self.handle_model_picker_key(key);
        }

        // Emacs-style editing keys (Ctrl+letter)
        if is_ctrl {
            return self.editor.handle_ctrl_key(key);
        }

        // Alt-style editing keys
        if is_alt {
            return self.editor.handle_alt_key(key);
        }

        match key.code {
            KeyCode::Enter => {
                if self.editor.handle_enter(is_shift) {
                    return true;
                }
                if !self.running && !self.editor.input.trim().is_empty() {
                    // Defer to handle_terminal_key for submit (needs terminal)
                    return false;
                }
                true
            }
            KeyCode::Tab => {
                self.editor.handle_tab();
                true
            }
            KeyCode::Up => {
                self.editor.handle_up();
                true
            }
            KeyCode::Down => {
                self.editor.handle_down();
                true
            }
            KeyCode::Char(_) => {
                // Handled below
                false
            }
            KeyCode::Backspace => {
                self.editor.handle_backspace();
                true
            }
            KeyCode::Left => {
                self.editor.move_left();
                true
            }
            KeyCode::Right => {
                self.editor.move_right();
                true
            }
            KeyCode::Home => {
                self.editor.move_home();
                true
            }
            KeyCode::End => {
                self.editor.move_end();
                true
            }
            KeyCode::Esc => {
                if self.editor.dismiss_suggestions() {
                    return true;
                }
                if self.running {
                    self.cancelled = true;
                } else {
                    self.should_quit = true;
                }
                true
            }
            _ => false,
        }
    }

    /// Handle keys that need the terminal (submit_prompt).
    fn handle_terminal_key(&mut self, terminal: &mut ratatui::DefaultTerminal, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => {
                if !self.running && !self.editor.input.trim().is_empty() {
                    let text = self.editor.input.clone();
                    self.editor.clear_input();
                    self.submit_prompt(terminal, &text);
                }
            }
            KeyCode::Char(c) => {
                self.editor.push_char(c);
            }
            _ => {}
        }
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
        self.streaming_start = Some(std::time::Instant::now());
        self.auto_scroll = true;
        self.cancelled = false;
        self.entries.push(ChatEntry::User(text.to_string()));
        self.editor.push_to_history(text);

        let _ = terminal.draw(|f| self.render(f));

        let tool_defs = self.tool_definitions.clone();
        let compaction_prompt = self
            .host_state
            .as_ref()
            .map(|h| h.compaction_prompt.clone())
            .unwrap_or_default();
        let budget = self.budget.clone();

        let (_events, actions) = self
            .agent_host
            .as_mut()
            .expect("agent")
            .transition(|runtime, transcript, artifacts, turn| {
                match runtime {
                    AgentRuntime::Idle(idle) => TransitionParts::from(idle
                        .start_turn(
                            AgentMessage::user(text),
                            tool_defs,
                            transcript,
                            artifacts,
                            turn,
                            &budget,
                            &compaction_prompt,
                        )
                        .into_parts()),
                    AgentRuntime::ReadyToContinue(ready) => {
                        let (_ev, _act, idle, transcript, artifacts, turn, _m) = ready.wait_for_input(transcript, artifacts, turn).into_parts();
                        TransitionParts::from(idle.start_turn(
                            AgentMessage::user(text),
                            tool_defs,
                            transcript,
                            artifacts,
                            turn,
                            &budget,
                            &compaction_prompt,
                        ).into_parts())
                    }
                    AgentRuntime::Finished(finished) => {
                        let (idle, transcript, artifacts, turn) =
                            finished.into_idle(transcript, artifacts, turn);
                        TransitionParts::from(idle.start_turn(
                            AgentMessage::user(text),
                            tool_defs,
                            transcript,
                            artifacts,
                            turn,
                            &budget,
                            &compaction_prompt,
                        ).into_parts())
                    }
                    AgentRuntime::Aborted(aborted) => {
                        let (idle, transcript, artifacts, turn) =
                            aborted.into_idle(transcript, artifacts, turn);
                        TransitionParts::from(idle.start_turn(
                            AgentMessage::user(text),
                            tool_defs,
                            transcript,
                            artifacts,
                            turn,
                            &budget,
                            &compaction_prompt,
                        ).into_parts())
                    }
                    AgentRuntime::PreToolCall(mut pre) => {
                        let disposition = pre.submit_user_message(AgentMessage::user(text));
                        let (events, actions) = disposition.into_events_actions();
                        TransitionParts::from((
                            events,
                            actions,
                            pre.into_runtime(),
                            transcript,
                            artifacts,
                            turn,
                            vec![],
                        ))
                    }
                    AgentRuntime::ExecutingTools(mut exec) => {
                        let disposition = exec.submit_user_message(AgentMessage::user(text));
                        let (events, actions) = disposition.into_events_actions();
                        TransitionParts::from((
                            events,
                            actions,
                            exec.into_runtime(),
                            transcript,
                            artifacts,
                            turn,
                            vec![],
                        ))
                    }
                    AgentRuntime::Compacting(compacting) => TransitionParts::from((
                        vec![],
                        vec![AgentAction::WaitForInput {
                            mode: WaitMode::Any,
                        }],
                        compacting.into_runtime(),
                        transcript,
                        artifacts,
                        turn,
                        vec![],
                    )),
                    other => TransitionParts::from((
                        vec![],
                        vec![AgentAction::WaitForInput {
                            mode: WaitMode::Any,
                        }],
                        other,
                        transcript,
                        artifacts,
                        turn,
                        vec![],
                    )),
                }
            });

        if let Some(ref logger) = self.session_logger {
            let _ = logger.append(&SessionEvent::TurnStart {
                turn: self.agent_host.as_ref().expect("agent").turn_number,
            });
        }
        self.handle_actions(terminal, actions);
        self.save_session();
    }

    fn actions_to_directives(&mut self, actions: Vec<AgentAction>) -> Vec<HostDirective> {
        let mut directives = Vec::new();
        for action in actions {
            match action {
                AgentAction::StreamLlm { context, .. } => {
                    directives.push(HostDirective::StreamLlm { context });
                }
                AgentAction::Summarize { context, .. } => {
                    directives.push(HostDirective::Summarize { context });
                }
                AgentAction::PrepareToolCalls { calls } => {
                    // TUI bypasses transform/permission hooks and auto-allows all calls.
                    // We must call prepare_tool_calls to transition from PreToolCall to
                    // ExecutingTools before executing tools.
                    let preps: Vec<ToolCallPreparation> = calls
                        .iter()
                        .map(|c| ToolCallPreparation {
                            tool_call_id: c.id.clone(),
                            transform: ToolCallTransform::None,
                            permission: ToolCallPermission::Allow,
                        })
                        .collect();
                    let (_events, new_actions) = self
                        .agent_host
                        .as_mut()
                        .expect("agent")
                        .transition(|runtime, transcript, artifacts, turn| {
                            if let AgentRuntime::PreToolCall(pre) = runtime {
                                TransitionParts::from(pre.prepare_tool_calls(preps.clone(), transcript, artifacts, turn).into_parts())
                            } else {
                                // Not in PreToolCall state — pass through unchanged
                                TransitionParts::from((vec![], vec![], runtime, transcript, artifacts, turn, vec![]))
                            }
                        });
                    for action in new_actions {
                        if let AgentAction::ExecuteTools { calls } = action {
                            directives.push(HostDirective::ExecuteTools { calls });
                        }
                    }
                }
                AgentAction::ExecuteTools { calls } => {
                    directives.push(HostDirective::ExecuteTools { calls });
                }
                AgentAction::CancelTools {
                    tool_call_ids,
                    reason,
                } => {
                    directives.push(HostDirective::CancelTools {
                        tool_call_ids,
                        reason,
                    });
                }
                AgentAction::WaitForInput { mode } => {
                    directives.push(HostDirective::WaitForInput { mode });
                }
                AgentAction::Finished => {
                    directives.push(HostDirective::Finished);
                    directives.push(HostDirective::Persist);
                }
            }
        }
        directives
    }

    fn handle_summarize(
        &mut self,
        terminal: &mut ratatui::DefaultTerminal,
        context: pi_core::LlmContext,
    ) {
        self.running = true;
        self.streaming_start = Some(std::time::Instant::now());
        let mut summary_text = String::new();

        match self
            .llm_client
            .stream_sync(&context.system_prompt, &context.messages, &context.tools)
        {
            Ok(mut stream) => {
                for chunk in stream.by_ref() {
                    match chunk {
                        pi_core::LlmChunk::TextDelta { text } => {
                            summary_text.push_str(&text);
                        }
                        pi_core::LlmChunk::Done => break,
                        _ => {}
                    }
                }
                let budget = self.budget.clone();
                let (_events, actions) = self
                    .agent_host
                    .as_mut()
                    .expect("agent")
                    .transition(|runtime, transcript, artifacts, turn| {
                        let AgentRuntime::Compacting(compacting) = runtime else {
                            return TransitionParts::from((vec![], vec![], runtime, transcript, artifacts, turn, vec![]));
                        };
                        let (events, actions, state, transcript, artifacts, turn, markers) = compacting
                            .accept_summary(summary_text.clone(), transcript, artifacts, turn, &budget)
                            .into_parts();
                        TransitionParts::from((
                            events,
                            actions,
                            state.into_runtime(),
                            transcript,
                            artifacts,
                            turn,
                            markers,
                        ))
                    });
                self.handle_actions(terminal, actions);
            }
            Err(e) => {
                self.entries
                    .push(ChatEntry::System(format!("Summary LLM Error: {e}")));
                let (_events, _actions) = self
                    .agent_host
                    .as_mut()
                    .expect("agent")
                    .transition(|runtime, transcript, artifacts, turn| {
                        crate::agent_host::AgentHost::abort_compacting_or_pass_through(runtime, transcript, artifacts, turn)
                    });
                self.running = false;
                self.streaming_start = None;
            }
        }
    }

    pub(crate) fn handle_actions(
        &mut self,
        terminal: &mut ratatui::DefaultTerminal,
        actions: Vec<AgentAction>,
    ) {
        let directives = self.actions_to_directives(actions);
        let directive_names: Vec<String> = directives
            .iter()
            .map(|d| match d {
                HostDirective::StreamLlm { .. } => "StreamLlm".to_string(),
                HostDirective::Summarize { .. } => "Summarize".to_string(),
                HostDirective::ExecuteTools { calls } => format!("ExecuteTools({})", calls.len()),
                HostDirective::CancelTools { .. } => "CancelTools".to_string(),
                HostDirective::Persist => "Persist".to_string(),
                HostDirective::Finished => "Finished".to_string(),
                HostDirective::WaitForInput { .. } => "WaitForInput".to_string(),
            })
            .collect();
        tracing::debug!(?directive_names, "handle_actions");

        // Track whether TurnEnd has been emitted for this batch
        let mut turn_ended = false;

        for directive in directives {
            if self.cancelled {
                let (_events, _actions) = self
                    .agent_host
                    .as_mut()
                    .expect("agent")
                    .transition(|runtime, transcript, artifacts, turn| {
                        match runtime {
                            AgentRuntime::Streaming(streaming) => {
                                let (ev, act, state, transcript, artifacts, tn, m) = streaming
                                    .abort(transcript, artifacts, turn)
                                    .into_parts();
                                TransitionParts::from((ev, act, state.into_runtime(), transcript, artifacts, tn, m))
                            }
                            other => crate::agent_host::AgentHost::abort_compacting_or_pass_through(
                                other, transcript, artifacts, turn,
                            ),
                        }
                    });
                self.running = false;
                self.streaming_start = None;
                self.entries.push(ChatEntry::System("Cancelled.".into()));
                let _ = terminal.draw(|f| self.render(f));
                return;
            }
            match directive {
                HostDirective::StreamLlm { context } => {
                    self.stream_llm(terminal, context);
                }
                HostDirective::Summarize { context } => {
                    self.handle_summarize(terminal, context);
                }
                HostDirective::ExecuteTools { calls } => {
                    self.execute_tools(terminal, calls);
                }
                HostDirective::Finished => {
                    self.entries.push(ChatEntry::System("Done.".into()));
                    self.running = false;
                    self.streaming_start = None;
                    if !turn_ended {
                        if let Some(ref logger) = self.session_logger {
                            let _ = logger.append(&SessionEvent::TurnEnd {
                                turn: self.agent_host.as_ref().expect("agent").turn_number,
                            });
                            turn_ended = true;
                        }
                    }
                    let _ = terminal.draw(|f| self.render(f));
                }
                HostDirective::WaitForInput { .. } => {
                    self.running = false;
                    self.streaming_start = None;
                    if !turn_ended {
                        if let Some(ref logger) = self.session_logger {
                            let _ = logger.append(&SessionEvent::TurnEnd {
                                turn: self.agent_host.as_ref().expect("agent").turn_number,
                            });
                            turn_ended = true;
                        }
                    }
                    self.save_session();
                    let _ = terminal.draw(|f| self.render(f));
                }
                HostDirective::Persist => {
                    self.save_session();
                }
                _ => {}
            }
        }
    }

    // -----------------------------------------------------------------------
    // Rendering coordinator
    // -----------------------------------------------------------------------

    pub(crate) fn render(&mut self, frame: &mut Frame) {
        let [chat_area, input_area, status_area] = Layout::vertical([
            Constraint::Fill(1),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .areas(frame.area());

        self.last_chat_area = chat_area;
        self.render_chat(frame, chat_area);
        self.render_input(frame, input_area);
        self.render_status(frame, status_area);
    }
}

#[cfg(test)]
impl App {
    /// Build a minimal App for render/scroll E2E tests — no agent, no tools, dummy LLM.
    pub(crate) fn with_entries_for_test(entries: Vec<ChatEntry>) -> Self {
        Self {
            agent_host: None,
            entries,
            editor: crate::editor::Editor::new(),
            scroll_offset: 0,
            auto_scroll: true,
            should_quit: false,
            running: false,
            streaming_text: String::new(),
            streaming_start: None,
            current_tools: Vec::new(),
            tool_definitions: Vec::new(),
            llm_client: LlmClient::new("x", "x", "test", WireFormat::OpenAI),
            host_state: None,
            last_usage: None,
            session_id: None,
            session_backend: FileSystemSessionBackend::new(),
            cwd: std::path::PathBuf::from("."),
            cancelled: false,
            model_picker: None,
            extensions: Vec::new(),
            running_tasks: Vec::new(),
            session_logger: None,
            budget: ContextProjectionBudget::default(),
            context_window: 0,
            last_chat_area: Rect::ZERO,
            resolved_config: crate::config::ResolvedConfig {
                model: "test".into(),
                provider: "openai".into(),
                api_key: "***".into(),
                base_url: "x".into(),
                config_path: None,
            },
            thinking_level: ThinkingLevel::Off,
        }
    }
}


