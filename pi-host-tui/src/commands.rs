use crossterm::event::{KeyCode, KeyEvent};
use ratatui::DefaultTerminal;

use crate::app::App;

impl App {
    // -----------------------------------------------------------------------
    // Model picker
    // -----------------------------------------------------------------------

    fn open_model_picker(&mut self) {
        use crate::llm::ModelDiscovery;
        let agent_host = &self.agent_host;
        let models = self
            .llm_client
            .list_models()
            .map(|m| m.into_iter().map(|m| m.id).collect())
            .unwrap_or_else(|_| {
                // Fallback: just the current model
                vec![agent_host.runtime().state().model.id.as_str().to_string()]
            });
        let current = agent_host.runtime().state().model.id.as_str().to_string();
        if models.is_empty() {
            self.entries
                .push(crate::app::ChatEntry::System("No available models".into()));
            return;
        }
        self.model_picker = Some(crate::model_picker::ModelPicker::new(models, current));
    }

    fn switch_model(&mut self, model_id: &str) {
        self.llm_client.set_model(model_id);
        self.agent_mut().state_mut().model.id = pi_core::ModelId::new(model_id);
        self.agent_mut().state_mut().model.name = pi_core::ModelName::new(model_id);
        self.entries.push(crate::app::ChatEntry::System(format!(
            "Model switched to {model_id}"
        )));
    }

    pub(crate) fn handle_model_picker_key(&mut self, key: KeyEvent) -> bool {
        let picker = self.model_picker.as_mut().expect("model_picker");
        match key.code {
            KeyCode::Enter => {
                if let Some(model_id) = picker.confirm() {
                    self.model_picker = None;
                    self.editor.clear_input();
                    self.switch_model(&model_id);
                }
                true
            }
            KeyCode::Esc => {
                self.model_picker = None;
                self.editor.clear_input();
                true
            }
            KeyCode::Up => {
                picker.select_previous();
                true
            }
            KeyCode::Down => {
                picker.select_next();
                true
            }
            KeyCode::Backspace => {
                picker.backspace();
                true
            }
            KeyCode::Char(c) => {
                picker.append_char(c);
                true
            }
            _ => false,
        }
    }

    // -----------------------------------------------------------------------
    // Slash commands
    // -----------------------------------------------------------------------

    pub(crate) fn handle_command(&mut self, terminal: &mut DefaultTerminal, text: &str) {
        let parts: Vec<&str> = text.split_whitespace().collect();
        let cmd = parts.first().copied().unwrap_or("");

        match cmd {
            "/clear" => {
                self.agent_host.reset();
                self.entries.clear();
                self.entries
                    .push(crate::app::ChatEntry::System("Chat cleared.".into()));
            }
            "/help" => {
                let list = crate::editor::COMMANDS.join("  ");
                self.entries
                    .push(crate::app::ChatEntry::System(format!("Commands: {list}")));
            }
            "/quit" => {
                self.should_quit = true;
            }
            "/model" => {
                if parts.len() >= 2 {
                    let model_id = parts[1];
                    self.switch_model(model_id);
                } else {
                    // Open model picker
                    self.open_model_picker();
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
                        self.entries.push(crate::app::ChatEntry::System(msg));
                    }
                    "load" => {
                        if let Some(id) = parts.get(2) {
                            if let Some(data) = self.session_backend.load(id) {
                                let host_state =
                                    crate::host_state::HostState::restore(data.clone());
                                self.agent_host.reset();
                                self.agent_host.transcript = data.transcript;
                                self.agent_host.artifacts = data.artifacts;
                                self.agent_host.turn_number = data.turn_number;
                                self.host_state = Some(host_state);
                                self.session_id = Some(id.to_string());
                                self.session_logger =
                                    crate::session_log::SessionEventLogger::new(id).ok();
                                self.entries.clear();
                                self.entries.push(crate::app::ChatEntry::System(format!(
                                    "Session '{id}' loaded."
                                )));
                            } else {
                                self.entries.push(crate::app::ChatEntry::System(format!(
                                    "Session '{id}' not found."
                                )));
                            }
                        } else {
                            self.entries.push(crate::app::ChatEntry::System(
                                "Usage: /session load <id>".into(),
                            ));
                        }
                    }
                    "new" => {
                        self.agent_host.reset();
                        self.session_id = None;
                        self.session_logger = None;
                        self.entries.clear();
                        self.entries
                            .push(crate::app::ChatEntry::System("New session started.".into()));
                    }
                    _ => {
                        self.entries.push(crate::app::ChatEntry::System(
                            "Usage: /session list | load <id> | new".into(),
                        ));
                    }
                }
            }
            "/tokens" => {
                if let Some((input, output, total)) = self.last_usage {
                    let ctx_pct = if self.context_window > 0 {
                        (input as f64 / self.context_window as f64 * 100.0) as u16
                    } else {
                        0
                    };
                    self.entries.push(crate::app::ChatEntry::System(format!(
                        "Tokens: in={input} out={output} total={total} ctx={ctx_pct}%"
                    )));
                } else {
                    self.entries.push(crate::app::ChatEntry::System(
                        "No token usage recorded yet.".into(),
                    ));
                }
            }
            "/undo" => {
                let host = &mut self.agent_host;
                if let Some(last_user_idx) = host
                    .transcript
                    .iter()
                    .rposition(|m| matches!(m, pi_core::TrimmedMessage::User(_)))
                {
                    host.transcript.truncate(last_user_idx);
                    if let Some(last_user_entry) = self
                        .entries
                        .iter()
                        .rposition(|e| matches!(e, crate::app::ChatEntry::User(_)))
                    {
                        self.entries.truncate(last_user_entry);
                    }
                    self.entries
                        .push(crate::app::ChatEntry::System("Last turn undone.".into()));
                } else {
                    self.entries
                        .push(crate::app::ChatEntry::System("Nothing to undo.".into()));
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
                self.entries.push(crate::app::ChatEntry::System(format!(
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
                self.entries.push(crate::app::ChatEntry::System(format!(
                    "Unknown command: {cmd}. Type /help for list."
                )));
            }
        }

        let _ = terminal.draw(|f| self.render(f));
    }
}
