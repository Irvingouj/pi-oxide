use pi_core::{AgentRuntime, Content, ToolCall, ToolCallId, ToolError, ToolResult};

use crate::app::{App, ChatEntry};
use crate::extension::{ExtensionContext, ExtensionOutcome, ToolEvent};

impl App {
    pub(crate) fn execute_tools(
        &mut self,
        terminal: &mut ratatui::DefaultTerminal,
        calls: Vec<ToolCall>,
    ) {
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
                    ExtensionOutcome::Complete(result) => {
                        self.on_tool_result(terminal, call.id, result);
                    }
                    ExtensionOutcome::Running(stream) => {
                        let _events = self.agent_mut().on_tool_started(call.id.clone());
                        self.running_tasks.push(crate::app::RunningTask {
                            tool_call_id: call.id.clone(),
                            stream,
                        });
                    }
                }
            } else {
                self.on_tool_result(
                    terminal,
                    call.id,
                    Err(ToolError::new(
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

    pub(crate) fn on_tool_result(
        &mut self,
        terminal: &mut ratatui::DefaultTerminal,
        tool_call_id: ToolCallId,
        result: Result<ToolResult, ToolError>,
    ) {
        let output_text = match &result {
            Ok(tool_result) => tool_result
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
            Err(err) => err.message.clone(),
        };

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
            is_error: result.is_err(),
        });
        let _ = terminal.draw(|f| self.render(f));

        let runtime = self.agent.take().unwrap();
        let (_events, actions, new_runtime) = match runtime {
            AgentRuntime::WaitingTools(waiting) => {
                waiting.on_tool_done(tool_call_id, result).into_parts()
            }
            other => (vec![], vec![], other),
        };
        self.agent = Some(new_runtime);
        self.handle_actions(terminal, actions);
    }

    pub(crate) fn poll_running_tasks(&mut self, terminal: &mut ratatui::DefaultTerminal) {
        let mut remaining = Vec::new();
        let mut just_completed: Vec<(ToolCallId, Result<ToolResult, ToolError>)> = Vec::new();

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

        // Apply UI updates and typestate transitions atomically per completed task
        let mut runtime = self.agent.take().unwrap();
        let mut all_actions = Vec::new();
        for (tool_call_id, result) in just_completed {
            let output_text = match &result {
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

            let (events, actions, new_runtime) = match runtime {
                AgentRuntime::WaitingTools(waiting) => {
                    waiting.on_tool_done(tool_call_id, result).into_parts()
                }
                other => (vec![], vec![], other),
            };
            runtime = new_runtime;
            all_actions.extend(actions);
            // Discard core events in async path (TUI reconstructs its own transcript)
            let _ = events;
        }

        self.agent = Some(runtime);
        self.handle_actions(terminal, all_actions);
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
