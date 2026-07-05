use std::path::Path;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Text;
use ratatui::widgets::ListState;
use ratatui::Frame;

use pi_core::{
    AgentAction, AgentMessage, AgentOptions, AgentRuntime, ApiName, Artifacts,
    ContextProjectionBudget, ExecutionMode, Model, ModelId, ModelName, ProviderName, QueueMode,
    SessionId, ThinkingLevel, ToolCallId, ToolCallPermission, ToolCallPreparation,
    ToolCallTransform, ToolDefinition, TrimmedMessage, WaitMode,
};

use crate::config::ResolvedConfig;
use crate::extension::{BashExtension, BuiltinExtension, Extension};
use crate::host_state::{HostDirective, HostState};
#[allow(unused_imports)]
use crate::llm::LlmProvider;
use crate::llm::{LlmClient, WireFormat};
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
    "/config",
];

// ---------------------------------------------------------------------------
// Chat line types
// ---------------------------------------------------------------------------

#[allow(dead_code)]
pub(crate) enum ChatEntry {
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

fn wrapped_lines(text: &str, width: usize) -> usize {
    if text.is_empty() {
        return 1;
    }
    let width = width.max(1);
    let display_len = text.chars().count();
    (display_len.saturating_add(width.saturating_sub(1))) / width
}

impl ChatEntry {
    /// Approximate wrapped line count at the given width.
    pub(crate) fn line_count(&self, width: usize) -> u16 {
        match self {
            ChatEntry::User(text) => {
                let mut count: u16 = 2; // header "You: " + blank
                for line in text.lines() {
                    count += wrapped_lines(line, width) as u16;
                }
                count
            }
            ChatEntry::Assistant(text) => {
                let mut count: u16 = 0;
                for line in &text.lines {
                    let s: String = line
                        .spans
                        .iter()
                        .map(|s| (*s.content).to_string())
                        .collect();
                    count += wrapped_lines(&s, width).max(1) as u16;
                }
                count + 1 // blank
            }
            ChatEntry::ToolStart { name, args_summary } => {
                let full = format!(" ┌─ {} {}", name, args_summary);
                wrapped_lines(&full, width) as u16
            }
            ChatEntry::ToolResult {
                output, is_error, ..
            } => {
                let mut count: u16 = 0;
                for line in output.lines() {
                    let full = format!("{}{}", if *is_error { " ┃ " } else { " │ " }, line);
                    count += wrapped_lines(&full, width) as u16;
                }
                count + 2 // footer + blank
            }
            ChatEntry::System(text) => {
                let full = format!("  {}", text);
                wrapped_lines(&full, width) as u16 + 1 // + blank
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Scroll key handling
// ---------------------------------------------------------------------------

/// Scroll intent derived from a keypress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScrollIntent {
    Up,       // one row
    Down,     // one row
    PageUp,   // one viewport
    PageDown, // one viewport
    Top,      // to start
    Bottom,   // to end / re-arm auto_scroll
}

/// Map a `KeyEvent` to a `ScrollIntent`. Returns `None` when the key is not
/// a scroll key.
pub(crate) fn derive_scroll_intent(key: &KeyEvent) -> Option<ScrollIntent> {
    match key.code {
        KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => Some(ScrollIntent::Up),
        KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => Some(ScrollIntent::Down),
        KeyCode::PageUp => Some(ScrollIntent::PageUp),
        KeyCode::PageDown => Some(ScrollIntent::PageDown),
        KeyCode::Home => Some(ScrollIntent::Top),
        KeyCode::End => Some(ScrollIntent::Bottom),
        KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(ScrollIntent::PageUp)
        }
        KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(ScrollIntent::PageDown)
        }
        _ => None,
    }
}

/// Pure state transition. Returns the new `(scroll_offset, auto_scroll)`.
pub(crate) fn apply_scroll(
    intent: ScrollIntent,
    total_lines: u16,
    visible: u16,
    scroll_offset: u16,
    auto_scroll: bool,
) -> (u16, bool) {
    // Everything fits — no scrolling needed
    if total_lines <= visible {
        let (off, auto) = match intent {
            ScrollIntent::Bottom => (scroll_offset, true),
            _ => (scroll_offset, auto_scroll),
        };
        tracing::debug!(
            ?intent,
            total_lines,
            visible,
            scroll_offset,
            auto_scroll,
            offset = off,
            auto_scroll = auto,
            "apply_scroll: fits"
        );
        return (off, auto);
    }

    let max_offset = total_lines - visible;

    let (off, auto) = match intent {
        ScrollIntent::Up => {
            if auto_scroll {
                (max_offset.saturating_sub(1), false)
            } else {
                (scroll_offset.saturating_sub(1), false)
            }
        }
        ScrollIntent::Down => {
            if auto_scroll {
                (scroll_offset, true)
            } else {
                let new_offset = (scroll_offset + 1).min(max_offset);
                if new_offset >= max_offset {
                    (max_offset, true)
                } else {
                    (new_offset, false)
                }
            }
        }
        ScrollIntent::PageUp => {
            if auto_scroll {
                (max_offset.saturating_sub(visible), false)
            } else {
                (scroll_offset.saturating_sub(visible), false)
            }
        }
        ScrollIntent::PageDown => {
            if auto_scroll {
                (scroll_offset, true)
            } else {
                let new_offset = (scroll_offset + visible).min(max_offset);
                if new_offset >= max_offset {
                    (max_offset, true)
                } else {
                    (new_offset, false)
                }
            }
        }
        ScrollIntent::Top => (0, false),
        ScrollIntent::Bottom => (scroll_offset, true),
    };

    tracing::debug!(
        ?intent,
        total_lines,
        visible,
        scroll_offset,
        auto_scroll,
        offset = off,
        auto_scroll = auto,
        "apply_scroll"
    );
    (off, auto)
}

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

pub struct App {
    pub(crate) agent: Option<AgentRuntime>,
    pub(crate) entries: Vec<ChatEntry>,
    pub(crate) input: String,
    pub(crate) cursor_pos: usize,
    pub(crate) scroll_offset: u16,
    pub(crate) auto_scroll: bool,
    pub(crate) should_quit: bool,
    pub(crate) running: bool,
    pub(crate) streaming_text: String,
    #[allow(dead_code)]
    pub(crate) current_tools: Vec<(String, String)>,
    pub(crate) tool_definitions: Vec<ToolDefinition>,
    pub(crate) llm_client: crate::llm::LlmBackend,
    pub(crate) host_state: Option<HostState>,
    pub(crate) last_usage: Option<(u32, u32, u32)>,
    pub(crate) session_id: Option<String>,
    pub(crate) session_backend: FileSystemSessionBackend,
    pub(crate) cwd: std::path::PathBuf,

    // New: cancellation
    pub(crate) cancelled: bool,

    // New: history recall
    pub(crate) history: Vec<String>,
    pub(crate) history_index: Option<usize>,
    pub(crate) original_input: String,

    // New: command autocomplete
    pub(crate) suggestions: Vec<String>,
    pub(crate) show_suggestions: bool,
    pub(crate) suggestion_state: ListState,

    // Extension-based tool execution
    pub(crate) extensions: Vec<Box<dyn Extension>>,
    pub(crate) running_tasks: Vec<RunningTask>,

    // New transcript/artifacts model
    pub(crate) transcript: Vec<TrimmedMessage>,
    pub(crate) artifacts: Artifacts,
    pub(crate) turn_number: u32,
    pub(crate) budget: ContextProjectionBudget,
    /// Cached chat area from the last render frame.
    pub(crate) last_chat_area: Rect,
    pub(crate) resolved_config: ResolvedConfig,
}

pub(crate) struct RunningTask {
    pub(crate) tool_call_id: ToolCallId,
    pub(crate) stream: Box<dyn crate::extension::ToolEventStream>,
}

impl App {
    pub(crate) fn agent(&self) -> &AgentRuntime {
        self.agent.as_ref().unwrap()
    }

    pub(crate) fn agent_mut(&mut self) -> &mut AgentRuntime {
        self.agent.as_mut().unwrap()
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
    ) -> Result<Self, Box<dyn std::error::Error>> {
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

        Ok(Self {
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
            tool_definitions: tool_defs,
            llm_client,
            host_state: Some(host_state.unwrap_or_else(|| HostState::new(system_prompt.to_string(), "Summarize the following conversation into a concise summary that preserves the key information, decisions, and context.".to_string()))),
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
            transcript: Vec::new(),
            artifacts: Artifacts::new(),
            turn_number: 0,
            budget: ContextProjectionBudget::default(),
            last_chat_area: ratatui::layout::Rect::ZERO,
            resolved_config,
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
    ) -> Result<crate::llm::LlmBackend, Box<dyn std::error::Error>> {
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
    ) -> Result<(), Box<dyn std::error::Error>> {
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
        // Ctrl+C: cancel running LLM
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            if self.running {
                self.cancelled = true;
            }
            return true;
        }

        // Scroll keys — handled before the main match
        if let Some(intent) = derive_scroll_intent(&key) {
            let visible = self.last_chat_area.height.saturating_sub(2);
            let total_lines = self.wrapped_line_count(self.last_chat_area.width as usize);
            let (off, auto) = apply_scroll(
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

        match key.code {
            KeyCode::Enter => {
                if self.show_suggestions {
                    if let Some(idx) = self.suggestion_state.selected() {
                        if let Some(cmd) = self.suggestions.get(idx).cloned() {
                            self.input = cmd;
                            self.cursor_pos = self.input.len();
                            self.show_suggestions = false;
                            return true;
                        }
                    }
                }
                if !self.running && !self.input.trim().is_empty() {
                    // Defer to handle_terminal_key for submit (needs terminal)
                    return false;
                }
                true
            }
            KeyCode::Tab => {
                if self.input.starts_with('/') {
                    self.update_suggestions();
                } else if self.show_suggestions {
                    self.suggestion_state.select_next();
                }
                true
            }
            KeyCode::Up => {
                if self.show_suggestions {
                    self.suggestion_state.select_previous();
                } else {
                    self.history_recall_previous();
                }
                true
            }
            KeyCode::Down => {
                if self.show_suggestions {
                    self.suggestion_state.select_next();
                } else {
                    self.history_recall_next();
                }
                true
            }
            KeyCode::Char(_) => {
                // Handled below
                false
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
                true
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
                true
            }
            KeyCode::Right => {
                if self.cursor_pos < self.input.len() {
                    self.cursor_pos = self.input[self.cursor_pos..]
                        .chars()
                        .next()
                        .map(|c| self.cursor_pos + c.len_utf8())
                        .unwrap_or(self.input.len());
                }
                true
            }
            KeyCode::Esc => {
                if self.show_suggestions {
                    self.show_suggestions = false;
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
                if !self.running && !self.input.trim().is_empty() {
                    let text = self.input.clone();
                    self.input.clear();
                    self.cursor_pos = 0;
                    self.show_suggestions = false;
                    self.submit_prompt(terminal, &text);
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
        let tool_defs = self.tool_definitions.clone();
        let compaction_prompt = self
            .host_state
            .as_ref()
            .map(|h| h.compaction_prompt.clone())
            .unwrap_or_default();
        let transcript = std::mem::take(&mut self.transcript);
        let artifacts = std::mem::take(&mut self.artifacts);
        let turn_number = self.turn_number;
        let budget = self.budget.clone();
        let (_events, actions, new_runtime, transcript, artifacts, turn_number, _markers) =
            match runtime {
                AgentRuntime::Idle(idle) => idle
                    .start_turn(
                        AgentMessage::user(text),
                        tool_defs,
                        transcript,
                        artifacts,
                        turn_number,
                        &budget,
                        &compaction_prompt,
                    )
                    .into_parts(),
                AgentRuntime::ReadyToContinue(ready) => {
                    let (_ev, _act, idle, transcript, artifacts, turn_number, _m) = ready
                        .wait_for_input(transcript, artifacts, turn_number)
                        .into_parts();
                    idle.start_turn(
                        AgentMessage::user(text),
                        tool_defs,
                        transcript,
                        artifacts,
                        turn_number,
                        &budget,
                        &compaction_prompt,
                    )
                    .into_parts()
                }
                AgentRuntime::Finished(finished) => {
                    let (idle, transcript, artifacts, turn_number) =
                        finished.into_idle(transcript, artifacts, turn_number);
                    idle.start_turn(
                        AgentMessage::user(text),
                        tool_defs,
                        transcript,
                        artifacts,
                        turn_number,
                        &budget,
                        &compaction_prompt,
                    )
                    .into_parts()
                }
                AgentRuntime::Aborted(aborted) => {
                    let (idle, transcript, artifacts, turn_number) =
                        aborted.into_idle(transcript, artifacts, turn_number);
                    idle.start_turn(
                        AgentMessage::user(text),
                        tool_defs,
                        transcript,
                        artifacts,
                        turn_number,
                        &budget,
                        &compaction_prompt,
                    )
                    .into_parts()
                }
                AgentRuntime::PreToolCall(mut pre) => {
                    let disposition = pre.submit_user_message(AgentMessage::user(text));
                    let (events, actions) = disposition.into_events_actions();
                    (
                        events,
                        actions,
                        pre.into_runtime(),
                        transcript,
                        artifacts,
                        turn_number,
                        vec![],
                    )
                }
                AgentRuntime::ExecutingTools(mut exec) => {
                    let disposition = exec.submit_user_message(AgentMessage::user(text));
                    let (events, actions) = disposition.into_events_actions();
                    (
                        events,
                        actions,
                        exec.into_runtime(),
                        transcript,
                        artifacts,
                        turn_number,
                        vec![],
                    )
                }
                AgentRuntime::Compacting(compacting) => (
                    vec![],
                    vec![AgentAction::WaitForInput {
                        mode: WaitMode::Any,
                    }],
                    compacting.into_runtime(),
                    transcript,
                    artifacts,
                    turn_number,
                    vec![],
                ),
                other => (
                    vec![],
                    vec![AgentAction::WaitForInput {
                        mode: WaitMode::Any,
                    }],
                    other,
                    transcript,
                    artifacts,
                    turn_number,
                    vec![],
                ),
            };
        self.transcript = transcript;
        self.artifacts = artifacts;
        self.turn_number = turn_number;
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
                self.transcript.clear();
                self.artifacts.clear();
                self.turn_number = 0;
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
                    self.agent_mut().state_mut().model.id = ModelId::new(model_id);
                    self.agent_mut().state_mut().model.name = ModelName::new(model_id);
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
                            if let Some(data) = self.session_backend.load(id) {
                                let host_state = HostState::restore(data.clone());
                                let agent = self.agent.take().unwrap().reset();
                                self.agent = Some(agent);
                                self.transcript = data.transcript;
                                self.artifacts = data.artifacts;
                                self.turn_number = data.turn_number;
                                self.host_state = Some(host_state);
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
                        self.transcript.clear();
                        self.artifacts.clear();
                        self.turn_number = 0;
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
                    let budget = &self.budget;
                    let ctx_pct = if budget.max_context_tokens > 0 {
                        (input as f64 / budget.max_context_tokens as f64 * 100.0) as u16
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
                if let Some(last_user_idx) = self
                    .transcript
                    .iter()
                    .rposition(|m| matches!(m, TrimmedMessage::User(_)))
                {
                    self.transcript.truncate(last_user_idx);
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
            "/config" => {
                let cfg = &self.resolved_config;
                let path = cfg
                    .config_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "(none — using defaults)".into());
                let masked_key = if cfg.api_key.is_empty() {
                    "(not set)".into()
                } else if cfg.api_key.len() > 8 {
                    format!("{}...", &cfg.api_key[..4])
                } else {
                    "****".into()
                };
                let system_prompt = self
                    .host_state
                    .as_ref()
                    .map(|h| h.system_prompt.as_str())
                    .unwrap_or("(unknown)");
                let sp_display: String = system_prompt.chars().take(80).collect();
                let sp_display = if system_prompt.chars().count() > 80 {
                    sp_display + "..."
                } else {
                    sp_display
                };
                self.entries.push(ChatEntry::System(format!(
                    "Config file: {path}\n\
                     Model: {}\n\
                     Provider: {}\n\
                     API key: {}\n\
                     Base URL: {}\n\
                     Session: {}\n\
                     System prompt: {}",
                    cfg.model,
                    cfg.provider,
                    masked_key,
                    cfg.base_url,
                    self.session_id.as_deref().unwrap_or("(none)"),
                    sp_display,
                )));
            }

            _ => {
                self.entries.push(ChatEntry::System(format!(
                    "Unknown command: {cmd}. Type /help for list."
                )));
            }
        }

        let _ = terminal.draw(|f| self.render(f));
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
                    let runtime = self.agent.take().unwrap();
                    if let AgentRuntime::PreToolCall(pre) = runtime {
                        let preps = calls
                            .iter()
                            .map(|c| ToolCallPreparation {
                                tool_call_id: c.id.clone(),
                                transform: ToolCallTransform::None,
                                permission: ToolCallPermission::Allow,
                            })
                            .collect();
                        let transcript = std::mem::take(&mut self.transcript);
                        let artifacts = std::mem::take(&mut self.artifacts);
                        let (
                            events,
                            new_actions,
                            runtime,
                            transcript,
                            artifacts,
                            turn_number,
                            _markers,
                        ) = pre
                            .prepare_tool_calls(preps, transcript, artifacts, self.turn_number)
                            .into_parts();
                        self.transcript = transcript;
                        self.artifacts = artifacts;
                        self.turn_number = turn_number;
                        self.agent = Some(runtime);
                        for action in new_actions {
                            if let AgentAction::ExecuteTools { calls } = action {
                                directives.push(HostDirective::ExecuteTools { calls });
                            }
                        }
                        let _ = events;
                    } else {
                        self.agent = Some(runtime);
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
                let runtime = self.agent.take().unwrap();
                let AgentRuntime::Compacting(compacting) = runtime else {
                    self.agent = Some(runtime);
                    return;
                };
                let transcript = std::mem::take(&mut self.transcript);
                let artifacts = std::mem::take(&mut self.artifacts);
                let transition = compacting.accept_summary(
                    summary_text,
                    transcript,
                    artifacts,
                    self.turn_number,
                    &self.budget,
                );
                let (_events, actions, runtime, transcript, artifacts, turn_number, _markers) =
                    transition.into_parts();
                self.transcript = transcript;
                self.artifacts = artifacts;
                self.turn_number = turn_number;
                self.agent = Some(runtime.into_runtime());
                self.handle_actions(terminal, actions);
            }
            Err(e) => {
                self.entries
                    .push(ChatEntry::System(format!("Summary LLM Error: {e}")));
                let runtime = self.agent.take().unwrap();
                if let AgentRuntime::Compacting(compacting) = runtime {
                    let transcript = std::mem::take(&mut self.transcript);
                    let artifacts = std::mem::take(&mut self.artifacts);
                    let transition = compacting.abort(transcript, artifacts, self.turn_number);
                    let (_events, _actions, runtime, transcript, artifacts, turn_number, _markers) =
                        transition.into_parts();
                    self.transcript = transcript;
                    self.artifacts = artifacts;
                    self.turn_number = turn_number;
                    self.agent = Some(runtime.into_runtime());
                } else {
                    self.agent = Some(runtime);
                }
                self.running = false;
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
        for directive in directives {
            if self.cancelled {
                let runtime = self.agent.take().unwrap();
                let transcript = std::mem::take(&mut self.transcript);
                let artifacts = std::mem::take(&mut self.artifacts);
                let (_events, _actions, new_runtime, transcript, artifacts, turn_number, _markers) =
                    match runtime {
                        AgentRuntime::Streaming(streaming) => {
                            let (ev, act, state, transcript, artifacts, tn, m) = streaming
                                .abort(transcript, artifacts, self.turn_number)
                                .into_parts();
                            (ev, act, state.into_runtime(), transcript, artifacts, tn, m)
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
                self.running = false;
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
                    let _ = terminal.draw(|f| self.render(f));
                }
                HostDirective::WaitForInput { .. } => {
                    self.running = false;
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
    fn with_entries_for_test(entries: Vec<ChatEntry>) -> Self {
        Self {
            agent: None,
            entries,
            input: String::new(),
            cursor_pos: 0,
            scroll_offset: 0,
            auto_scroll: true,
            should_quit: false,
            running: false,
            streaming_text: String::new(),
            current_tools: Vec::new(),
            tool_definitions: Vec::new(),
            llm_client: LlmClient::new("x", "x", "test", WireFormat::OpenAI),
            host_state: None,
            last_usage: None,
            session_id: None,
            session_backend: FileSystemSessionBackend::new(),
            cwd: std::path::PathBuf::from("."),
            cancelled: false,
            history: Vec::new(),
            history_index: None,
            original_input: String::new(),
            suggestions: Vec::new(),
            show_suggestions: false,
            suggestion_state: ListState::default(),
            extensions: Vec::new(),
            running_tasks: Vec::new(),
            transcript: Vec::new(),
            artifacts: Artifacts::new(),
            turn_number: 0,
            budget: ContextProjectionBudget::default(),
            last_chat_area: Rect::ZERO,
            resolved_config: crate::config::ResolvedConfig {
                model: "test".into(),
                provider: "openai".into(),
                api_key: "***".into(),
                base_url: "x".into(),
                config_path: None,
            },
        }
    }
}

#[cfg(test)]
mod scroll_tests {
    use super::*;
    use crossterm::event::KeyEventKind;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn make_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }
    }

    #[test]
    fn derive_scroll_shift_up() {
        let key = make_key(KeyCode::Up, KeyModifiers::SHIFT);
        assert_eq!(derive_scroll_intent(&key), Some(ScrollIntent::Up));
    }

    #[test]
    fn derive_scroll_shift_down() {
        let key = make_key(KeyCode::Down, KeyModifiers::SHIFT);
        assert_eq!(derive_scroll_intent(&key), Some(ScrollIntent::Down));
    }

    #[test]
    fn derive_scroll_page_up() {
        let key = make_key(KeyCode::PageUp, KeyModifiers::NONE);
        assert_eq!(derive_scroll_intent(&key), Some(ScrollIntent::PageUp));
    }

    #[test]
    fn derive_scroll_page_down() {
        let key = make_key(KeyCode::PageDown, KeyModifiers::NONE);
        assert_eq!(derive_scroll_intent(&key), Some(ScrollIntent::PageDown));
    }

    #[test]
    fn derive_scroll_home() {
        let key = make_key(KeyCode::Home, KeyModifiers::NONE);
        assert_eq!(derive_scroll_intent(&key), Some(ScrollIntent::Top));
    }

    #[test]
    fn derive_scroll_end() {
        let key = make_key(KeyCode::End, KeyModifiers::NONE);
        assert_eq!(derive_scroll_intent(&key), Some(ScrollIntent::Bottom));
    }

    #[test]
    fn derive_scroll_ctrl_b() {
        let key = make_key(KeyCode::Char('b'), KeyModifiers::CONTROL);
        assert_eq!(derive_scroll_intent(&key), Some(ScrollIntent::PageUp));
    }

    #[test]
    fn derive_scroll_ctrl_f() {
        let key = make_key(KeyCode::Char('f'), KeyModifiers::CONTROL);
        assert_eq!(derive_scroll_intent(&key), Some(ScrollIntent::PageDown));
    }

    #[test]
    fn derive_scroll_plain_up_not_scroll() {
        let key = make_key(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(derive_scroll_intent(&key), None);
    }

    #[test]
    fn derive_scroll_plain_down_not_scroll() {
        let key = make_key(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(derive_scroll_intent(&key), None);
    }

    #[test]
    fn apply_scroll_top() {
        let (off, auto) = apply_scroll(ScrollIntent::Top, 100, 10, 50, false);
        assert_eq!(off, 0);
        assert!(!auto);
    }

    #[test]
    fn apply_scroll_bottom() {
        let (off, auto) = apply_scroll(ScrollIntent::Bottom, 100, 10, 50, false);
        assert!(!auto || off == 50); // offset unchanged, auto=true
        assert!(auto);
    }

    #[test]
    fn apply_scroll_up_from_auto() {
        let (off, auto) = apply_scroll(ScrollIntent::Up, 100, 10, 90, true);
        assert_eq!(off, 89);
        assert!(!auto);
    }

    #[test]
    fn apply_scroll_down_from_auto_noop() {
        let (off, auto) = apply_scroll(ScrollIntent::Down, 100, 10, 90, true);
        assert_eq!(off, 90);
        assert!(auto);
    }

    #[test]
    fn apply_scroll_down_reaches_bottom() {
        let (off, auto) = apply_scroll(ScrollIntent::Down, 100, 10, 89, false);
        assert_eq!(off, 90);
        assert!(auto);
    }

    #[test]
    fn apply_scroll_fits_all_noop() {
        let (off, auto) = apply_scroll(ScrollIntent::Up, 5, 10, 0, true);
        assert_eq!(off, 0);
        assert!(auto);
    }

    #[test]
    fn handle_key_scroll_shift_up_disengages_auto() {
        // Simulate what handle_key does for scroll keys:
        // 1. derive_scroll_intent maps Shift+Up → ScrollIntent::Up
        // 2. apply_scroll computes new state
        // 3. handle_key writes back to self
        let key = make_key(KeyCode::Up, KeyModifiers::SHIFT);
        let intent = derive_scroll_intent(&key).expect("should be scroll key");
        // 100 lines, 10 visible, auto_scroll=true, offset=0 (at bottom = 90)
        let (off, auto) = apply_scroll(intent, 100, 10, 0, true);
        assert!(!auto, "auto_scroll should be disengaged");
        assert_eq!(off, 89, "should scroll up one row from bottom");
    }

    #[test]
    fn handle_key_scroll_home_jumps_to_top() {
        let key = make_key(KeyCode::Home, KeyModifiers::NONE);
        let intent = derive_scroll_intent(&key).expect("should be scroll key");
        let (off, auto) = apply_scroll(intent, 100, 10, 50, false);
        assert_eq!(off, 0);
        assert!(!auto);
    }

    #[test]
    fn handle_key_scroll_end_rearms_auto() {
        let key = make_key(KeyCode::End, KeyModifiers::NONE);
        let intent = derive_scroll_intent(&key).expect("should be scroll key");
        let (off, auto) = apply_scroll(intent, 100, 10, 0, false);
        assert!(auto, "auto_scroll should be re-armed");
    }

    #[test]
    fn handle_key_plain_up_not_consumed_as_scroll() {
        // Plain Up (no modifier) should NOT be treated as a scroll key
        let key = make_key(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(derive_scroll_intent(&key), None);
    }

    // -----------------------------------------------------------------------
    // E2E: render → scroll key → re-render → assert buffer content changed
    // -----------------------------------------------------------------------

    fn build_scroll_entries() -> Vec<ChatEntry> {
        (0..30)
            .map(|i| ChatEntry::System(format!("Line {i:02}")))
            .collect()
    }

    fn get_backend_render(app: &mut App, terminal: &mut Terminal<TestBackend>) -> String {
        terminal.draw(|f| app.render(f)).unwrap();
        terminal.backend().to_string()
    }

    #[test]
    fn e2e_home_end_scroll() {
        let entries = build_scroll_entries();
        let mut app = App::with_entries_for_test(entries);
        let backend = ratatui::backend::TestBackend::new(40, 12);
        let mut terminal = Terminal::new(backend).unwrap();

        // Initial render — auto-scroll should show bottom
        let rendered = get_backend_render(&mut app, &mut terminal);
        assert!(
            rendered.contains("Line 29"),
            "auto-scroll should show bottom; got: {rendered}"
        );
        assert!(
            !rendered.contains("Line 00"),
            "top should not be visible; got: {rendered}"
        );

        // Press Home → jump to top
        let consumed = app.handle_key(make_key(KeyCode::Home, KeyModifiers::NONE));
        assert!(consumed, "Home should be consumed as scroll key");
        let rendered = get_backend_render(&mut app, &mut terminal);
        assert!(
            rendered.contains("Line 00"),
            "after Home, top should be visible; got: {rendered}"
        );
        assert!(
            !rendered.contains("Line 29"),
            "after Home, bottom should not be visible; got: {rendered}"
        );

        // Press End → jump to bottom
        let consumed = app.handle_key(make_key(KeyCode::End, KeyModifiers::NONE));
        assert!(consumed, "End should be consumed as scroll key");
        let rendered = get_backend_render(&mut app, &mut terminal);
        assert!(
            rendered.contains("Line 29"),
            "after End, bottom should be visible; got: {rendered}"
        );
        assert!(
            !rendered.contains("Line 00"),
            "after End, top should not be visible; got: {rendered}"
        );
    }

    #[test]
    fn e2e_page_scroll() {
        let entries = build_scroll_entries();
        let mut app = App::with_entries_for_test(entries);
        let backend = ratatui::backend::TestBackend::new(40, 12);
        let mut terminal = Terminal::new(backend).unwrap();

        // Initial render at bottom
        let rendered = get_backend_render(&mut app, &mut terminal);
        assert!(
            rendered.contains("Line 29"),
            "start at bottom; got: {rendered}"
        );

        // PageUp (Ctrl+B) → shift up one viewport
        app.handle_key(make_key(KeyCode::Char('b'), KeyModifiers::CONTROL));
        let rendered = get_backend_render(&mut app, &mut terminal);
        assert!(
            !rendered.contains("Line 29"),
            "after PageUp, bottom should not be visible; got: {rendered}"
        );
        assert!(!app.auto_scroll, "PageUp should disengage auto_scroll");

        // PageDown (Ctrl+F) twice → back to bottom
        app.handle_key(make_key(KeyCode::Char('f'), KeyModifiers::CONTROL));
        app.handle_key(make_key(KeyCode::Char('f'), KeyModifiers::CONTROL));
        let rendered = get_backend_render(&mut app, &mut terminal);
        assert!(
            rendered.contains("Line 29"),
            "after 2x PageDown, bottom should be visible; got: {rendered}"
        );
    }

    #[test]
    fn e2e_partial_entry_overlap() {
        // Build entries where one entry is very long (wraps to many lines)
        // so it may only partially overlap the visible range.
        // Short entries first to fill up, then one long entry at the end.
        let mut entries: Vec<ChatEntry> = (0..10)
            .map(|i| ChatEntry::System(format!("Short {i:02}")))
            .collect();
        // One long user message that wraps to ~15 lines at width 40
        let long_text = (0..15)
            .map(|i| format!("Long line {i:02} of the big message"))
            .collect::<Vec<_>>()
            .join("\n");
        entries.push(ChatEntry::User(long_text));

        let mut app = App::with_entries_for_test(entries);
        let backend = ratatui::backend::TestBackend::new(40, 12);
        let mut terminal = Terminal::new(backend).unwrap();

        // Initial render at bottom — should show end of long message
        let rendered = get_backend_render(&mut app, &mut terminal);
        assert!(
            rendered.contains("Long line 14"),
            "auto-scroll should show end of long message; got: {rendered}"
        );
        assert!(
            !rendered.contains("Short 00"),
            "top short entries should not be visible; got: {rendered}"
        );

        // Scroll to top
        app.handle_key(make_key(KeyCode::Home, KeyModifiers::NONE));
        let rendered = get_backend_render(&mut app, &mut terminal);
        assert!(
            rendered.contains("Short 00"),
            "after Home, first short entry should be visible; got: {rendered}"
        );

        // Scroll line by line down until we hit the long message
        for _ in 0..20 {
            app.handle_key(make_key(KeyCode::Down, KeyModifiers::SHIFT));
        }
        let rendered = get_backend_render(&mut app, &mut terminal);
        // We should be somewhere in the long message now
        assert!(
            rendered.contains("Long line"),
            "after scrolling down, long message should be visible; got: {rendered}"
        );
    }
}
