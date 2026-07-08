use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Text;
use ratatui::Frame;
use thiserror::Error;

use pi_core::{
    AgentAction, AgentMessage, AgentOptions, AgentRuntime, ApiName, ContextProjectionBudget,
    ExecutionMode, Model, ModelId, ModelName, ProviderName, QueueMode, SessionId, ThinkingLevel,
    ToolCallId, ToolCallPermission, ToolCallPreparation, ToolCallTransform, ToolDefinition,
    WaitMode,
};

use crate::agent_host::TransitionParts;
use crate::config::ResolvedConfig;
use crate::extension::{BashExtension, BuiltinExtension, Extension};
use crate::host_state::{HostDirective, HostState};
use crate::llm::{LlmClient, LlmProvider, WireFormat};
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
    #[error("LLM error: {0}")]
    Llm(#[from] Box<dyn std::error::Error>),
}

// ---------------------------------------------------------------------------
// Chat line types
// ---------------------------------------------------------------------------

#[derive(Clone)]
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
// Actor commands + render snapshot
// ---------------------------------------------------------------------------

/// Commands sent to the actor via the message channel.
pub(crate) enum AppCmd {
    Key(KeyEvent),
    Submit(String),
    Cancel,
    Resize(u16, u16),
}

/// Lock-free render snapshot — published by actor, read by render task at 30fps.
pub(crate) struct RenderSnapshot {
    pub entries: Arc<[ChatEntry]>,
    pub input_text: String,
    pub input_cursor_pos: usize,
    pub show_suggestions: bool,
    pub suggestions: Vec<String>,
    pub suggestion_selection: Option<usize>,
    pub running: bool,
    pub streaming_start: Option<std::time::Instant>,
    pub scroll_offset: u16,
    pub auto_scroll: bool,
    pub last_chat_area: Rect,
    pub model_name: String,
    pub thinking_level: ThinkingLevel,
    pub show_quit_prompt: bool,
}

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

pub struct App {
    /// Host-side agent mediator — owns runtime, transcript, artifacts, turn_number.
    pub(crate) agent_host: crate::agent_host::AgentHost,
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

    // Double-press Ctrl+C protection
    pub(crate) pending_quit: bool,

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

    /// Lock-free render snapshot — updated by actor, read at 30fps by render task.
    #[allow(dead_code)]
    pub(crate) snapshot: std::sync::Arc<ArcSwap<RenderSnapshot>>,

    /// Wire format derived from provider (Anthropic or OpenAI).
    pub(crate) wire_format: WireFormat,

    /// Pending LLM context to be streamed by the async actor loop.
    pub(crate) pending_llm_context: Option<pi_core::LlmContext>,

    /// Pre-collected chunks from stream_sync (replay/record compatible).
    /// When set, process_stream_llm_async drains these instead of creating a new client.
    pub(crate) pending_chunks: Option<Vec<pi_core::LlmChunk>>,

    /// Stream metadata collected alongside pending_chunks.
    #[allow(dead_code)]
    pub(crate) pending_stream_usage: Option<(u32, u32, u32)>,
    #[allow(dead_code)]
    pub(crate) pending_stop_reason: String,
    #[allow(dead_code)]
    pub(crate) pending_tool_calls: Vec<crate::agent_host::CollectedToolCall>,
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
        self.agent_host.runtime()
    }

    pub(crate) fn agent_mut(&mut self) -> &mut AgentRuntime {
        self.agent_host.runtime_mut()
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
            agent_host,
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
            pending_quit: false,
            model_picker: None,
            extensions,
            running_tasks: Vec::new(),
            budget: ContextProjectionBudget::default(),
            context_window,
            last_chat_area: ratatui::layout::Rect::ZERO,
            resolved_config,
            thinking_level: ThinkingLevel::Off,
            wire_format,
            pending_llm_context: None,
            pending_chunks: None,
            pending_stream_usage: None,
            pending_stop_reason: String::new(),
            pending_tool_calls: Vec::new(),
            snapshot: std::sync::Arc::new(ArcSwap::from_pointee(RenderSnapshot {
                entries: Default::default(),
                input_text: String::new(),
                input_cursor_pos: 0,
                show_suggestions: false,
                suggestions: Vec::new(),
                suggestion_selection: None,
                running: false,
                streaming_start: None,
                scroll_offset: 0,
                auto_scroll: true,
                last_chat_area: Rect::ZERO,
                model_name: String::from(model_id),
                thinking_level: ThinkingLevel::Off,
                show_quit_prompt: false,
            })),
        })
    }

    /// Publish current state as a render snapshot — updates ArcSwap for 30fps reads.
    pub(crate) fn publish_snapshot(&self) {
        let snap = RenderSnapshot {
            entries: Arc::from(self.entries.clone()),
            input_text: self.editor.input.clone(),
            input_cursor_pos: self.editor.cursor_pos,
            show_suggestions: self.editor.show_suggestions,
            suggestions: self.editor.suggestions.clone(),
            suggestion_selection: self.editor.suggestion_state.selected(),
            running: self.running || !self.running_tasks.is_empty(),
            streaming_start: self.streaming_start,
            scroll_offset: self.scroll_offset,
            auto_scroll: self.auto_scroll,
            last_chat_area: self.last_chat_area,
            model_name: self.llm_client.model_id().to_string(),
            thinking_level: self.thinking_level,
            show_quit_prompt: self.pending_quit,
        };
        self.snapshot.store(Arc::new(snap));
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
// NOTE: The run() loop is replaced by actor_loop + render_task in the async model.
// This method kept for backward compat (record/replay tests).

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

        // Ctrl+C: cancel running LLM, or clear input / double-press to quit
        if key.code == KeyCode::Char('c') && is_ctrl {
            if self.running {
                self.cancelled = true;
            } else {
                if self.editor.input.is_empty() {
                    if self.pending_quit {
                        self.should_quit = true;
                    } else {
                        self.pending_quit = true;
                    }
                } else {
                    self.editor.clear_input();
                    self.pending_quit = false;
                }
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
            KeyCode::Char(c) => {
                self.editor.push_char(c);
                true
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
                // Esc: dismiss suggestions or clear input. Never exits TUI.
                if self.editor.dismiss_suggestions() {
                    return true;
                }
                if !self.editor.input.is_empty() {
                    self.editor.clear_input();
                }
                self.pending_quit = false;
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

        let (_events, actions) =
            self.agent_host
                .transition(|runtime, transcript, artifacts, turn| match runtime {
                    AgentRuntime::Idle(idle) => TransitionParts::from(
                        idle.start_turn(
                            AgentMessage::user(text),
                            tool_defs,
                            transcript,
                            artifacts,
                            turn,
                            &budget,
                            &compaction_prompt,
                        )
                        .into_parts(),
                    ),
                    AgentRuntime::ReadyToContinue(ready) => {
                        let (_ev, _act, idle, transcript, artifacts, turn, _m) = ready
                            .wait_for_input(transcript, artifacts, turn)
                            .into_parts();
                        TransitionParts::from(
                            idle.start_turn(
                                AgentMessage::user(text),
                                tool_defs,
                                transcript,
                                artifacts,
                                turn,
                                &budget,
                                &compaction_prompt,
                            )
                            .into_parts(),
                        )
                    }
                    AgentRuntime::Finished(finished) => {
                        let (idle, transcript, artifacts, turn) =
                            finished.into_idle(transcript, artifacts, turn);
                        TransitionParts::from(
                            idle.start_turn(
                                AgentMessage::user(text),
                                tool_defs,
                                transcript,
                                artifacts,
                                turn,
                                &budget,
                                &compaction_prompt,
                            )
                            .into_parts(),
                        )
                    }
                    AgentRuntime::Aborted(aborted) => {
                        let (idle, transcript, artifacts, turn) =
                            aborted.into_idle(transcript, artifacts, turn);
                        TransitionParts::from(
                            idle.start_turn(
                                AgentMessage::user(text),
                                tool_defs,
                                transcript,
                                artifacts,
                                turn,
                                &budget,
                                &compaction_prompt,
                            )
                            .into_parts(),
                        )
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
                });

        if let Some(ref logger) = self.session_logger {
            let _ = logger.append(&SessionEvent::TurnStart {
                turn: self.agent_host.turn_number,
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
                    let (_events, new_actions) =
                        self.agent_host
                            .transition(|runtime, transcript, artifacts, turn| {
                                if let AgentRuntime::PreToolCall(pre) = runtime {
                                    TransitionParts::from(
                                        pre.prepare_tool_calls(
                                            preps.clone(),
                                            transcript,
                                            artifacts,
                                            turn,
                                        )
                                        .into_parts(),
                                    )
                                } else {
                                    // Not in PreToolCall state — pass through unchanged
                                    TransitionParts::from((
                                        vec![],
                                        vec![],
                                        runtime,
                                        transcript,
                                        artifacts,
                                        turn,
                                        vec![],
                                    ))
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
                let (_events, actions) =
                    self.agent_host
                        .transition(|runtime, transcript, artifacts, turn| {
                            let AgentRuntime::Compacting(compacting) = runtime else {
                                return TransitionParts::from((
                                    vec![],
                                    vec![],
                                    runtime,
                                    transcript,
                                    artifacts,
                                    turn,
                                    vec![],
                                ));
                            };
                            let (events, actions, state, transcript, artifacts, turn, markers) =
                                compacting
                                    .accept_summary(
                                        summary_text.clone(),
                                        transcript,
                                        artifacts,
                                        turn,
                                        &budget,
                                    )
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
                let (_events, _actions) =
                    self.agent_host
                        .transition(|runtime, transcript, artifacts, turn| {
                            crate::agent_host::AgentHost::abort_compacting_or_pass_through(
                                runtime, transcript, artifacts, turn,
                            )
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
                let (_events, _actions) = self.agent_host.transition(
                    |runtime, transcript, artifacts, turn| match runtime {
                        AgentRuntime::Streaming(streaming) => {
                            let (ev, act, state, transcript, artifacts, tn, m) =
                                streaming.abort(transcript, artifacts, turn).into_parts();
                            TransitionParts::from((
                                ev,
                                act,
                                state.into_runtime(),
                                transcript,
                                artifacts,
                                tn,
                                m,
                            ))
                        }
                        other => crate::agent_host::AgentHost::abort_compacting_or_pass_through(
                            other, transcript, artifacts, turn,
                        ),
                    },
                );
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
                                turn: self.agent_host.turn_number,
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
                                turn: self.agent_host.turn_number,
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

    /// Submit text without terminal rendering — for async actor loop.
    pub(crate) fn submit_text(&mut self, text: &str) {
        if text.starts_with('/') {
            // Commands handled inline (no terminal rendering needed — snapshots handle it)
            self.handle_command_inline(text);
            return;
        }

        self.running = true;
        self.streaming_start = Some(std::time::Instant::now());
        self.auto_scroll = true;
        self.cancelled = false;
        self.entries.push(ChatEntry::User(text.to_string()));
        self.editor.push_to_history(text);

        // Start the agent turn — produces actions (typically StreamLlm)
        let tool_defs = self.tool_definitions.clone();
        let compaction_prompt = self
            .host_state
            .as_ref()
            .map(|h| h.compaction_prompt.clone())
            .unwrap_or_default();
        let budget = self.budget.clone();

        let (_events, actions) =
            self.agent_host
                .transition(|runtime, transcript, artifacts, turn| match runtime {
                    AgentRuntime::Idle(idle) => TransitionParts::from(
                        idle.start_turn(
                            AgentMessage::user(text),
                            tool_defs,
                            transcript,
                            artifacts,
                            turn,
                            &budget,
                            &compaction_prompt,
                        )
                        .into_parts(),
                    ),
                    AgentRuntime::ReadyToContinue(ready) => {
                        let (_ev, _act, idle, transcript, artifacts, turn, _m) = ready
                            .wait_for_input(transcript, artifacts, turn)
                            .into_parts();
                        TransitionParts::from(
                            idle.start_turn(
                                AgentMessage::user(text),
                                tool_defs,
                                transcript,
                                artifacts,
                                turn,
                                &budget,
                                &compaction_prompt,
                            )
                            .into_parts(),
                        )
                    }
                    AgentRuntime::Finished(finished) => {
                        let (idle, transcript, artifacts, turn) =
                            finished.into_idle(transcript, artifacts, turn);
                        TransitionParts::from(
                            idle.start_turn(
                                AgentMessage::user(text),
                                tool_defs,
                                transcript,
                                artifacts,
                                turn,
                                &budget,
                                &compaction_prompt,
                            )
                            .into_parts(),
                        )
                    }
                    AgentRuntime::Aborted(aborted) => {
                        let (idle, transcript, artifacts, turn) =
                            aborted.into_idle(transcript, artifacts, turn);
                        TransitionParts::from(
                            idle.start_turn(
                                AgentMessage::user(text),
                                tool_defs,
                                transcript,
                                artifacts,
                                turn,
                                &budget,
                                &compaction_prompt,
                            )
                            .into_parts(),
                        )
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
                });

        if let Some(ref logger) = self.session_logger {
            let _ = logger.append(&SessionEvent::TurnStart {
                turn: self.agent_host.turn_number,
            });
        }

        // Convert to directives and handle inline where possible; defer LLM streaming
        // to the async actor loop via pending_llm_context.
        let directives = self.actions_to_directives(actions);
        for directive in directives {
            match directive {
                HostDirective::StreamLlm { context } | HostDirective::Summarize { context } => {
                    // Store LLM context for async processing by process_stream_llm_async.
                    self.pending_llm_context = Some(context);
                }
                HostDirective::Persist => {
                    self.save_session();
                }
                HostDirective::Finished => {
                    self.running = false;
                    self.streaming_start = None;
                }
                HostDirective::WaitForInput { .. } => {
                    self.running = false;
                    self.streaming_start = None;
                    self.save_session();
                }
                _ => {
                    // ExecuteTools / CancelTools shouldn't appear on user-message
                    // submission — the first action is always StreamLlm. If they do
                    // arrive, they'll be handled after the LLM stream completes.
                }
            }
        }
    }

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

// ---------------------------------------------------------------------------
// Async actor loop + render task (replaces blocking run() in tokio model)
// ---------------------------------------------------------------------------

/// Spawn the async LLM streaming task. Returns immediately; chunks are sent back via channel.
pub(crate) fn spawn_stream_llm(
    api_key: String,
    base_url: String,
    model_id: String,
    wire_format: crate::llm::WireFormat,
    context: pi_core::LlmContext,
    chunk_tx: tokio::sync::mpsc::Sender<pi_core::LlmChunk>,
) -> tokio::task::JoinHandle<()> {
    let async_client = crate::llm::AsyncLlmClient::new(
        &api_key,
        &base_url,
        &model_id,
        wire_format,
    );

    tokio::spawn(async move {
        // Collect the response — narrow scope to avoid Box<dyn Error> living across awaits
        let mut stream = { 
            match async_client.stream_async(
                &context.system_prompt,
                &context.messages,
                &context.tools,
            ).await {
                Ok(s) => s,
                Err(_e) => return, // Box<dyn StdError> is !Send
            }
        };

        // Stream chunks through channel. next_chunk is non-blocking (buffer was collected at init).
        while let Some(chunk) = stream.next_chunk() {
            if chunk_tx.send(chunk).await.is_err() {
                break;  // receiver dropped — cancelled
            }
        }
    })
}

/// Process pre-collected chunks through the streaming agent (replay/record path).
fn process_stream_llm_from_chunks(app: &mut App, mut chunks: Vec<pi_core::LlmChunk>) -> Option<Vec<HostDirective>> {
    use pi_core::{
        message::TokenUsage, timestamp, AssistantMessage, Content, LlmResult, StopReason,
        TextContent, ToolArguments, ToolCallId, ToolName,
    };
    use crate::agent_host::CollectedToolCall;
    use crate::markdown;

    // Take ownership of the StreamingAgent from agent_host
    let runtime = app.agent_host.take_runtime();
    let mut streaming = match runtime {
        AgentRuntime::Streaming(s) => s,
        _ => {
            app.agent_host.set_runtime(runtime);
            return None;
        }
    };

    let model_id = app.llm_client.model_id().to_string();
    let budget = app.budget.clone();

    // Feed a synthetic Start chunk so the core sees the assistant message
    let _ = streaming.feed_llm_chunk(pi_core::LlmChunk::Start {
        partial: AssistantMessage::empty(),
    });
    app.entries.push(ChatEntry::Assistant(Text::raw("...")));
    app.publish_snapshot();

    let mut full_text = String::new();
    app.streaming_text.clear();

    // Process pre-collected chunks through the agent
    for chunk in chunks.drain(..) {
        if app.cancelled {
            app.entries.push(ChatEntry::System("Cancelled.".into()));
            let (events, actions, aborted, transcript, artifacts, turn, _markers) =
                streaming.abort(
                    std::mem::take(&mut app.agent_host.transcript),
                    std::mem::take(&mut app.agent_host.artifacts),
                    app.agent_host.turn_number,
                ).into_parts();
            app.agent_host.set_runtime(aborted.into_runtime());
            app.agent_host.transcript = transcript;
            app.agent_host.artifacts = artifacts;
            app.agent_host.turn_number = turn;
            let _ = (events, actions);
            app.running = false;
            app.streaming_start = None;
            app.cancelled = false;
            app.streaming_text.clear();
            app.publish_snapshot();
            return Some(vec![]);
        }

        let _core_events = streaming.feed_llm_chunk(chunk.clone());

        match chunk {
            pi_core::LlmChunk::TextDelta { text } => {
                full_text.push_str(&text);
                app.streaming_text = full_text.clone();
                if let Some(ChatEntry::Assistant(_)) = app.entries.last() {
                    let rendered = markdown::render(&full_text);
                    *app.entries.last_mut().unwrap() = ChatEntry::Assistant(rendered);
                }
                app.publish_snapshot();
            }
            pi_core::LlmChunk::Done => continue,
            pi_core::LlmChunk::Error { message } => {
                app.entries.push(ChatEntry::System(format!("LLM Error: {message}")));
                break;
            }
            _ => {}
        }
    }

    // Use pre-collected metadata from App fields (set by submit_text)
    let usage = std::mem::take(&mut app.pending_stream_usage);
    let mut stop_reason = std::mem::take(&mut app.pending_stop_reason);
    if stop_reason.is_empty() { stop_reason.push_str("end_turn"); }
    let tool_calls: Vec<CollectedToolCall> = std::mem::take(&mut app.pending_tool_calls);

    tracing::debug!(
        text_len = full_text.len(),
        tool_calls = tool_calls.len(),
        %stop_reason,
        "LLM stream completed (replay/record)"
    );

    // Build tool call content blocks
    let tool_use_blocks: Vec<Content> = tool_calls.iter().map(|tc| {
        Content::ToolCall(pi_core::ToolCall {
            id: ToolCallId::new(&tc.id),
            name: ToolName::new(&tc.name),
            arguments: ToolArguments::new(tc.input.clone()),
        })
    }).collect();

    let text_block = if full_text.is_empty() && tool_use_blocks.is_empty() {
        vec![Content::Text(TextContent { text: String::new() })]
    } else if full_text.is_empty() {
        vec![]
    } else {
        vec![Content::Text(TextContent { text: full_text })]
    };

    let content: Vec<Content> = text_block.into_iter().chain(tool_use_blocks).collect();

    let sr = if stop_reason == "tool_use"
        || content.iter().any(|c| matches!(c, Content::ToolCall(_))) {
            StopReason::ToolUse
        } else { StopReason::EndTurn };

    let assistant_msg = AssistantMessage {
        content,
        api: pi_core::ApiName::new("anthropic"),
        provider: pi_core::ProviderName::new("anthropic"),
        model: ModelId::new(&model_id),
        stop_reason: sr,
        error_message: None,
        timestamp: timestamp::current_timestamp(),
        usage: TokenUsage {
            input: usage.map(|(i, _, _)| i).unwrap_or(0),
            output: usage.map(|(_, o, _)| o).unwrap_or(0),
            cache_read: 0,
            cache_write: 0,
            total_tokens: usage.map(|(_, _, t)| t).unwrap_or(0),
        },
    };

    let (events, actions, new_runtime, transcript, artifacts, turn, _markers) =
        streaming.finish_llm(
            LlmResult::Ok(assistant_msg),
            std::mem::take(&mut app.agent_host.transcript),
            std::mem::take(&mut app.agent_host.artifacts),
            app.agent_host.turn_number,
            &budget,
        ).into_parts();

    let _ = events;
    app.agent_host.set_runtime(new_runtime);
    app.agent_host.transcript = transcript;
    app.agent_host.artifacts = artifacts;
    app.agent_host.turn_number = turn;

    app.cancelled = false;
    app.streaming_start = None;
    app.streaming_text.clear();
    app.publish_snapshot();

    if let Some(ref logger) = app.session_logger {
        let _ = logger.append(&SessionEvent::LlmResponse {
            turn: app.agent_host.turn_number,
            stop_reason: "completed".to_string(),
        });
    }

    Some(app.actions_to_directives(actions))
}

/// Process a pending LLM stream asynchronously — feeds chunks to the streaming
/// agent, publishes snapshots for rendering, and handles resulting directives.
///
/// Takes ownership of the `StreamingAgent` from `app.agent_host`, streams
/// chunks through it, then calls `finish_llm` and recurses into follow-up
/// directives (more streaming, tool execution, etc.).
pub(crate) async fn process_stream_llm_async(app: &mut App) -> Option<Vec<HostDirective>> {
    use pi_core::{
        message::TokenUsage, timestamp, AssistantMessage, Content, LlmResult, StopReason,
        TextContent, ToolArguments, ToolCallId, ToolName,
    };
    use crate::agent_host::CollectedToolCall;
    use crate::markdown;

    let context = app.pending_llm_context.take()?;

    // Log LLM request
    if let Some(ref logger) = app.session_logger {
        let turn = app.agent_host.turn_number;
        let _ = logger.append(&SessionEvent::LlmRequest {
            turn,
            model: app.llm_client.model_id().to_string(),
            message_count: context.messages.len(),
        });
    }

    // Replay/record path — use pre-collected chunks (no network I/O)
    if let Some(chunks) = app.pending_chunks.take() {
        return process_stream_llm_from_chunks(app, chunks);
    }

    // Production path — create AsyncLlmClient and fetch chunks asynchronously.
    let async_client = crate::llm::AsyncLlmClient::new(
        &app.resolved_config.api_key,
        &app.resolved_config.base_url,
        &app.resolved_config.model,
        app.wire_format,
    );

    let mut stream = match async_client
        .stream_async(&context.system_prompt, &context.messages, &context.tools)
        .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = ?e, "LLM stream failed to start");
            if let Some(ref logger) = app.session_logger {
                let turn = app.agent_host.turn_number;
                let _ = logger.append(&SessionEvent::Error {
                    turn,
                    message: e.to_string(),
                });
            }
            let err_result = LlmResult::Err {
                error: pi_core::LlmError {
                    code: "call_failed".into(),
                    message: e.to_string(),
                    details: None,
                },
                aborted: false,
            };
            let budget = app.budget.clone();
            let (_events, actions) = app.agent_host.transition(
                |runtime, transcript, artifacts, turn| match runtime {
                    AgentRuntime::Streaming(streaming) => TransitionParts::from(
                        streaming
                            .finish_llm(err_result, transcript, artifacts, turn, &budget)
                            .into_parts(),
                    ),
                    other => crate::agent_host::AgentHost::abort_compacting_or_pass_through(
                        other, transcript, artifacts, turn,
                    ),
                },
            );
            app.cancelled = false;
            app.streaming_start = None;
            app.streaming_text.clear();
            app.running = false;
            app.publish_snapshot();
            let directives = app.actions_to_directives(actions);
            return Some(directives);
        }
    };

    // Take ownership of the StreamingAgent from agent_host
    let runtime = app.agent_host.take_runtime();
    let mut streaming = match runtime {
        AgentRuntime::Streaming(s) => s,
        _ => {
            app.agent_host.set_runtime(runtime);
            return None;
        }
    };

    let mut full_text = String::new();
    let model_id = app.llm_client.model_id().to_string();
    let budget = app.budget.clone();

    // Feed a synthetic Start chunk so the core sees the assistant message
    let _ = streaming.feed_llm_chunk(pi_core::LlmChunk::Start {
        partial: AssistantMessage::empty(),
    });
    app.entries.push(ChatEntry::Assistant(Text::raw("...")));
    app.publish_snapshot();

    app.streaming_text.clear();

    // Stream chunks — next_chunk is non-blocking (buffer was collected at init)
    while let Some(chunk) = stream.next_chunk() {
        // Cooperative cancellation check
        if app.cancelled {
            app.entries.push(ChatEntry::System("Cancelled.".into()));
            let (events, actions, aborted, transcript, artifacts, turn, _markers) =
                streaming
                    .abort(
                        std::mem::take(&mut app.agent_host.transcript),
                        std::mem::take(&mut app.agent_host.artifacts),
                        app.agent_host.turn_number,
                    )
                    .into_parts();
            app.agent_host.set_runtime(aborted.into_runtime());
            app.agent_host.transcript = transcript;
            app.agent_host.artifacts = artifacts;
            app.agent_host.turn_number = turn;
            let _ = (events, actions);

            app.running = false;
            app.streaming_start = None;
            app.cancelled = false;
            app.streaming_text.clear();
            app.publish_snapshot();
            return Some(vec![]);
        }

        let _core_events = streaming.feed_llm_chunk(chunk.clone());

        match chunk {
            pi_core::LlmChunk::TextDelta { text } => {
                full_text.push_str(&text);
                app.streaming_text = full_text.clone();
                if let Some(ChatEntry::Assistant(_)) = app.entries.last() {
                    let rendered = markdown::render(&full_text);
                    *app.entries.last_mut().unwrap() = ChatEntry::Assistant(rendered);
                }
                app.publish_snapshot();
            }
            pi_core::LlmChunk::Done => break,
            pi_core::LlmChunk::Error { message } => {
                app.entries
                    .push(ChatEntry::System(format!("LLM Error: {message}")));
                break;
            }
            _ => {}
        }
    }

    // Collect stream metadata from AsyncLlmStream
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

    // Build tool call content blocks
    let tool_use_blocks: Vec<Content> = tool_calls
        .iter()
        .map(|tc| {
            Content::ToolCall(pi_core::ToolCall {
                id: ToolCallId::new(&tc.id),
                name: ToolName::new(&tc.name),
                arguments: ToolArguments::new(tc.input.clone()),
            })
        })
        .collect();

    let text_block = if full_text.is_empty() && tool_use_blocks.is_empty() {
        vec![Content::Text(TextContent {
            text: String::new(),
        })]
    } else if full_text.is_empty() {
        vec![]
    } else {
        vec![Content::Text(TextContent {
            text: full_text,
        })]
    };

    let content: Vec<Content> = text_block.into_iter().chain(tool_use_blocks).collect();

    let sr = if stop_reason == "tool_use"
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
            input: usage.map(|(i, _, _)| i).unwrap_or(0),
            output: usage.map(|(_, o, _)| o).unwrap_or(0),
            cache_read: 0,
            cache_write: 0,
            total_tokens: usage.map(|(_, _, t)| t).unwrap_or(0),
        },
    };

    let (events, actions, new_runtime, transcript, artifacts, turn, _markers) =
        streaming
            .finish_llm(
                LlmResult::Ok(assistant_msg),
                std::mem::take(&mut app.agent_host.transcript),
                std::mem::take(&mut app.agent_host.artifacts),
                app.agent_host.turn_number,
                &budget,
            )
            .into_parts();

    let _ = events;
    app.agent_host.set_runtime(new_runtime);
    app.agent_host.transcript = transcript;
    app.agent_host.artifacts = artifacts;
    app.agent_host.turn_number = turn;

    app.cancelled = false;
    app.streaming_start = None;
    app.streaming_text.clear();
    app.publish_snapshot();

    // Log LLM response
    if let Some(ref logger) = app.session_logger {
        let turn = app.agent_host.turn_number;
        let _ = logger.append(&SessionEvent::LlmResponse {
            turn,
            stop_reason: "completed".to_string(),
        });
    }

    tracing::debug!(?actions, "finish_llm");
    let directives = app.actions_to_directives(actions);
    Some(directives)
}


/// Async actor loop — owns App, polls crossterm events on a separate thread.
pub(crate) async fn run_actor_loop(mut app: App) {
    use tokio::time::{sleep as tokio_sleep};

    let (tx, mut rx) = std::sync::mpsc::channel::<crossterm::event::Event>();
    std::thread::spawn(move || {
        loop {
            if crossterm::event::poll(std::time::Duration::from_millis(16)).unwrap_or(false) {
                match crossterm::event::read().ok() {
                    Some(e) => { let _ = tx.send(e); }
                    None => {}
                }
            }
        }
    });

    // Process events from channel — non-blocking recv with timeout
    while !app.should_quit {
        use std::sync::mpsc::TryRecvError;

        match rx.try_recv() {
            Ok(crossterm::event::Event::Key(key)) => {
                if key.kind == KeyEventKind::Press {
                    let handled = app.handle_key(key);
                    // If handle_key returned false for Enter, it means "not running + non-empty input" → submit
                    if !handled && key.code == KeyCode::Enter && !app.running {
                        let text = app.editor.input.trim().to_string();
                        if !text.is_empty() {
                            // Commands like /model are handled inline in App; regular messages set up agent state + pending LLM context.
                            app.submit_text(&text);
                            app.publish_snapshot();
                        }
                    }
                }
            }
            Ok(_) => {}
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => break, // thread died
        }

        app.publish_snapshot();

        // Process pending LLM stream if set up by submit_text.
        // This drains all chunks through the agent, publishing snapshots as text arrives,
        // so rendering stays at 30fps even while streaming blocks key input briefly.
        if let Some(directives) = process_stream_llm_async(&mut app).await {
            for directive in directives {
                match &directive {
                    HostDirective::Finished => {}
                    HostDirective::WaitForInput { .. } => {}
                    _ => {} // other directives handled implicitly
                }
            }
        }

        tokio_sleep(std::time::Duration::from_millis(33)).await;
    }
}

/// Render task — reads ArcSwap snapshot at fixed frame rate and draws.
pub(crate) fn render_task(
    terminal: ratatui::DefaultTerminal,
    snapshot: Arc<ArcSwap<RenderSnapshot>>,
) -> tokio::task::JoinHandle<()> {
    let term = std::sync::Mutex::new(terminal);

    tokio::spawn(async move {
        use tokio::time::{interval, Duration};

        // ~30fps render rate
        let frame_duration = Duration::from_millis(33);
        let mut ticker = interval(frame_duration);

        loop {
            ticker.tick().await;

            let snap = snapshot.load_full();  // lock-free read of Arc<RenderSnapshot>

            if let Ok(mut t) = term.lock() {
                if let Err(e) = t.draw(|f| render_from_snapshot(f, &snap)) {
                    tracing::error!(?e, "render failed");
                }
            }
        }
    })
}

/// Pure render function — takes snapshot (no mutation), produces widgets.
fn render_from_snapshot(frame: &mut Frame, snap: &RenderSnapshot) {
    let [chat_area, input_area, status_area] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    render_chat_from_snapshot(frame, chat_area, snap);
    render_input_from_snapshot(frame, input_area, snap);
    render_status_from_snapshot(frame, status_area, snap);
}

fn render_chat_from_snapshot(_frame: &mut Frame, _chat_area: Rect, _snap: &RenderSnapshot) {
    // Stub — will be fully implemented when UI modules are refactored.
    // For now delegate to existing App::render for backward compat.
}

fn render_input_from_snapshot(_frame: &mut Frame, _input_area: Rect, _snap: &RenderSnapshot) {
    // Stub
}

fn render_status_from_snapshot(_frame: &mut Frame, _status_area: Rect, _snap: &RenderSnapshot) {
    // Stub — show model name and status bar
}

#[cfg(all(test, not(feature = "replay")))]
impl App {
    /// Build a minimal App for render/scroll E2E tests — no agent, no tools, dummy LLM.
    pub(crate) fn with_entries_for_test(entries: Vec<ChatEntry>) -> Self {
        Self {
            agent_host: crate::agent_host::AgentHost::new(pi_core::AgentRuntime::Uninitialized),
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
            pending_quit: false,
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
            pending_llm_context: None,
            wire_format: WireFormat::OpenAI,
            pending_chunks: None,
            pending_stream_usage: None,
            pending_stop_reason: String::new(),
            pending_tool_calls: Vec::new(),
            snapshot: std::sync::Arc::new(ArcSwap::from_pointee(RenderSnapshot {
                entries: Default::default(),
                input_text: String::new(),
                input_cursor_pos: 0,
                show_suggestions: false,
                suggestions: Vec::new(),
                suggestion_selection: None,
                running: false,
                streaming_start: None,
                scroll_offset: 0,
                auto_scroll: true,
                last_chat_area: Rect::ZERO,
                model_name: "test".into(),
                thinking_level: ThinkingLevel::Off,
                show_quit_prompt: false,
            })),
        }
    }
}
