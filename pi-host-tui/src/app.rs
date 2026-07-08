use std::path::Path;
use std::sync::Arc;

use arc_swap::ArcSwap;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::text::Text;
use thiserror::Error;

use pi_core::{
    AgentAction, AgentMessage, AgentOptions, AgentRuntime, ApiName, ContextProjectionBudget,
    ExecutionMode, Model, ModelId, ModelName, ProviderName, QueueMode, SessionId, ThinkingLevel,
    ToolCallPermission, ToolCallPreparation, ToolCallTransform, ToolDefinition, WaitMode,
};

use crate::agent_host::TransitionParts;
use crate::config::ResolvedConfig;
use crate::directives::HostDirective;
use crate::extension::{BashExtension, BuiltinExtension, Extension};
use crate::host_state::{HostState, SessionContext};
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
            ChatEntry::System(text) => {
                // Must match emit_entry: "◇ {text}"
                let full = format!("◇ {}", text);
                wrapped_lines(&full, width) as u16 + 1 // + blank
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Render snapshot
// ---------------------------------------------------------------------------

/// Lock-free render snapshot — published by actor, read by render loop at 30fps.
pub(crate) struct RenderSnapshot {
    pub entries: Arc<[ChatEntry]>,
    pub input_text: String,
    pub input_cursor_pos: usize,
    pub running: bool,
    pub streaming_start: Option<std::time::Instant>,
    pub model_name: String,
    pub show_quit_prompt: bool,
    // Suggestions
    pub show_suggestions: bool,
    pub suggestions: Vec<String>,
    pub suggestion_selected: Option<usize>,
    // Model picker
    pub show_model_picker: bool,
    pub model_picker_items: Vec<String>,
    pub model_picker_selected: usize,
    pub model_picker_filter: String,
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

    pub(crate) tool_definitions: Vec<ToolDefinition>,
    pub(crate) llm_client: crate::llm::LlmBackend,
    pub(crate) host_state: Option<HostState>,
    pub(crate) last_usage: Option<(u32, u32, u32)>,
    pub(crate) session_id: Option<String>,
    pub(crate) session_backend: FileSystemSessionBackend,

    // Cancellation
    pub(crate) cancelled: bool,

    // Double-press Ctrl+C protection
    pub(crate) pending_quit: bool,

    // Model picker
    pub(crate) model_picker: Option<crate::model_picker::ModelPicker>,

    // Session event logger
    pub(crate) session_logger: Option<SessionEventLogger>,

    // Context budget and window
    pub(crate) budget: ContextProjectionBudget,
    pub(crate) context_window: u32,
    /// Cached chat area from the last render frame.
    pub(crate) last_chat_area: Rect,
    pub(crate) resolved_config: ResolvedConfig,

    /// Lock-free render snapshot — updated by actor, read at 30fps by render loop.
    pub(crate) snapshot: std::sync::Arc<ArcSwap<RenderSnapshot>>,

    /// Wire format derived from provider (Anthropic or OpenAI).
    pub(crate) wire_format: WireFormat,

    /// Pending LLM context to be streamed by the async actor loop.
    pub(crate) pending_llm_context: Option<pi_core::LlmContext>,

    /// Pre-collected chunks from stream_sync (replay/record compatible).
    /// When set, process_stream_llm_async drains these instead of creating a new client.
    pub(crate) pending_chunks: Option<Vec<pi_core::LlmChunk>>,

    /// Stream metadata collected alongside pending_chunks.
    pub(crate) pending_stream_usage: Option<(u32, u32, u32)>,
    pub(crate) pending_stop_reason: String,
    pub(crate) pending_tool_calls: Vec<crate::agent_host::CollectedToolCall>,
}

impl App {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        system_prompt: &str,
        model_id: &str,
        api_key: &str,
        base_url: &str,
        session_id: Option<String>,
        host_state: Option<HostState>,
        session_ctx: Option<SessionContext>,
        _cwd: &Path,
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

        let mut agent_host = crate::agent_host::AgentHost::new(agent);
        let budget = if let Some(ref ctx) = session_ctx {
            agent_host.transcript = ctx.transcript.clone();
            agent_host.artifacts = ctx.artifacts.clone();
            agent_host.turn_number = ctx.turn_number;
            ctx.budget.clone()
        } else {
            ContextProjectionBudget::default()
        };

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

            tool_definitions: tool_defs,
            llm_client,
            host_state: Some(host_state.unwrap_or_else(|| HostState::new(system_prompt.to_string(), "Summarize the following conversation into a concise summary that preserves the key information, decisions, and context.".to_string()))),
            last_usage: None,
            session_logger: session_id
                .as_ref()
                .and_then(|id| SessionEventLogger::new(id).ok()),
            session_id,
            session_backend: FileSystemSessionBackend::new(),
            cancelled: false,
            pending_quit: false,
            model_picker: None,
            budget,
            context_window,
            last_chat_area: ratatui::layout::Rect::ZERO,
            resolved_config,
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
                running: false,
                streaming_start: None,
                model_name: String::from(model_id),
                show_quit_prompt: false,
                show_suggestions: false,
                suggestions: Vec::new(),
                suggestion_selected: None,
                show_model_picker: false,
                model_picker_items: Vec::new(),
                model_picker_selected: 0,
                model_picker_filter: String::new(),
            })),
        })
    }

    /// Publish current state as a render snapshot — updates ArcSwap for 30fps reads.
    pub(crate) fn publish_snapshot(&self) {
        let picker = &self.model_picker;
        let snap = RenderSnapshot {
            entries: Arc::from(self.entries.clone()),
            input_text: self.editor.input.clone(),
            input_cursor_pos: self.editor.cursor_pos,
            running: self.running,
            streaming_start: self.streaming_start,
            model_name: self.llm_client.model_id().to_string(),
            show_quit_prompt: self.pending_quit,
            show_suggestions: self.editor.show_suggestions,
            suggestions: self.editor.suggestions.clone(),
            suggestion_selected: self.editor.suggestion_state.selected(),
            show_model_picker: picker.is_some(),
            model_picker_items: picker
                .as_ref()
                .map(|p| p.filtered().into_iter().map(|s| s.to_string()).collect())
                .unwrap_or_default(),
            model_picker_selected: picker.as_ref().map(|p| p.selected_index).unwrap_or(0),
            model_picker_filter: picker
                .as_ref()
                .map(|p| p.filter_text().to_string())
                .unwrap_or_default(),
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

    /// Sum of wrapped line counts for all entries at the given width.
    fn wrapped_line_count(&self, width: usize) -> u16 {
        self.entries.iter().map(|e| e.line_count(width)).sum()
    }

    /// Persist the current session to disk.
    pub(crate) fn save_session(&self) {
        let id = self.session_id.as_deref();
        let host = self.host_state.as_ref();
        let (id, host) = match (id, host) {
            (Some(id), Some(h)) => (id, h),
            _ => return,
        };
        let session_ctx = SessionContext {
            transcript: self.agent_host.transcript.clone(),
            artifacts: self.agent_host.artifacts.clone(),
            turn_number: self.agent_host.turn_number,
            budget: self.budget.clone(),
        };
        let data = host.get_persist_data(&session_ctx);
        if let Err(e) = self.session_backend.save(id, &data) {
            tracing::warn!(session_id = id, error = ?e, "failed to save session");
        }
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
}

// ---------------------------------------------------------------------------
// Async actor loop + render task (replaces blocking run() in tokio model)
// ---------------------------------------------------------------------------

/// Process pre-collected chunks through the streaming agent (replay/record path).
fn process_stream_llm_from_chunks(
    app: &mut App,
    mut chunks: Vec<pi_core::LlmChunk>,
) -> Option<Vec<HostDirective>> {
    use crate::agent_host::CollectedToolCall;
    use crate::markdown;
    use pi_core::{
        message::TokenUsage, timestamp, AssistantMessage, Content, LlmResult, StopReason,
        TextContent, ToolArguments, ToolCallId, ToolName,
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
            let (events, actions, aborted, transcript, artifacts, turn, _markers) = streaming
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
            pi_core::LlmChunk::Done => continue,
            pi_core::LlmChunk::Error { message } => {
                app.entries
                    .push(ChatEntry::System(format!("LLM Error: {message}")));
                break;
            }
            _ => {}
        }
    }

    // Use pre-collected metadata from App fields (set by submit_text)
    let usage = std::mem::take(&mut app.pending_stream_usage);
    let mut stop_reason = std::mem::take(&mut app.pending_stop_reason);
    if stop_reason.is_empty() {
        stop_reason.push_str("end_turn");
    }
    let tool_calls: Vec<CollectedToolCall> = std::mem::take(&mut app.pending_tool_calls);

    tracing::debug!(
        text_len = full_text.len(),
        tool_calls = tool_calls.len(),
        %stop_reason,
        "LLM stream completed (replay/record)"
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
        vec![Content::Text(TextContent { text: full_text })]
    };

    let content: Vec<Content> = text_block.into_iter().chain(tool_use_blocks).collect();

    let sr =
        if stop_reason == "tool_use" || content.iter().any(|c| matches!(c, Content::ToolCall(_))) {
            StopReason::ToolUse
        } else {
            StopReason::EndTurn
        };

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

    let (events, actions, new_runtime, transcript, artifacts, turn, _markers) = streaming
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
    use crate::agent_host::CollectedToolCall;
    use crate::markdown;
    use pi_core::{
        message::TokenUsage, timestamp, AssistantMessage, Content, LlmResult, StopReason,
        TextContent, ToolArguments, ToolCallId, ToolName,
    };

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
            let (_events, actions) =
                app.agent_host
                    .transition(|runtime, transcript, artifacts, turn| match runtime {
                        AgentRuntime::Streaming(streaming) => TransitionParts::from(
                            streaming
                                .finish_llm(err_result, transcript, artifacts, turn, &budget)
                                .into_parts(),
                        ),
                        other => crate::agent_host::AgentHost::abort_compacting_or_pass_through(
                            other, transcript, artifacts, turn,
                        ),
                    });
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
            let (events, actions, aborted, transcript, artifacts, turn, _markers) = streaming
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
        vec![Content::Text(TextContent { text: full_text })]
    };

    let content: Vec<Content> = text_block.into_iter().chain(tool_use_blocks).collect();

    let sr =
        if stop_reason == "tool_use" || content.iter().any(|c| matches!(c, Content::ToolCall(_))) {
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

    let (events, actions, new_runtime, transcript, artifacts, turn, _markers) = streaming
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
    use tokio::time::sleep as tokio_sleep;

    let (tx, rx) = std::sync::mpsc::channel::<crossterm::event::Event>();
    std::thread::spawn(move || loop {
        if crossterm::event::poll(std::time::Duration::from_millis(16)).unwrap_or(false) {
            if let Ok(e) = crossterm::event::read() {
                let _ = tx.send(e);
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
                match directive {
                    HostDirective::Finished => {
                        app.entries.push(ChatEntry::System("Done.".into()));
                        app.running = false;
                        app.streaming_start = None;
                        app.publish_snapshot();
                    }
                    HostDirective::WaitForInput { .. } => {
                        app.running = false;
                        app.streaming_start = None;
                        app.save_session();
                        app.publish_snapshot();
                    }
                    HostDirective::Persist => {
                        app.save_session();
                    }
                    _ => {}
                }
            }
        }

        tokio_sleep(std::time::Duration::from_millis(33)).await;
    }
}

#[cfg(all(test, not(feature = "replay")))]
impl App {
    /// Build a minimal App for unit tests — no agent, no tools, dummy LLM.
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
            tool_definitions: Vec::new(),
            llm_client: LlmClient::new("x", "x", "test", WireFormat::OpenAI),
            host_state: None,
            last_usage: None,
            session_id: None,
            session_backend: FileSystemSessionBackend::new(),
            cancelled: false,
            pending_quit: false,
            model_picker: None,
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
                running: false,
                streaming_start: None,
                model_name: "test".into(),
                show_quit_prompt: false,
                show_suggestions: false,
                suggestions: Vec::new(),
                suggestion_selected: None,
                show_model_picker: false,
                model_picker_items: Vec::new(),
                model_picker_selected: 0,
                model_picker_filter: String::new(),
            })),
        }
    }
}
