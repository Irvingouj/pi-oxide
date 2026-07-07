use pi_core::{AgentRuntime, ToolCall, ToolCallId, ToolError, ToolResult};
use tracing::{debug, trace};

use crate::agent_host::TransitionParts;

use crate::app::{App, ChatEntry};
use crate::extension::{ExtensionContext, ExtensionOutcome, ToolEvent};
use crate::session_log::SessionEvent;

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

            // Log tool call
            if let Some(ref logger) = self.session_logger {
                let turn = self.agent_host.as_ref().expect("agent").turn_number;
                let _ = logger.append(&SessionEvent::ToolCall {
                    turn,
                    tool_call_id: call.id.as_str().to_string(),
                    tool_name: call.name.as_str().to_string(),
                });
            }

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
                        self.on_tool_result(terminal, call.id.clone(), call.name.as_str().to_string(), result);
                    }
                    ExtensionOutcome::Running(stream) => {
                        // Notify the agent that the tool started (mutable, non-consuming)
                        if let AgentRuntime::ExecutingTools(exec) = self.agent_host.as_mut().expect("agent").runtime_mut() {
                            let _events = exec.on_tool_started(call.id.clone());
                        }
                        self.running_tasks.push(crate::app::RunningTask {
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
                    call.name.as_str().to_string(),
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
            self.auto_continue(terminal);
        }
    }

    pub(crate) fn on_tool_result(
        &mut self,
        terminal: &mut ratatui::DefaultTerminal,
        tool_call_id: ToolCallId,
        tool_name: String,
        result: Result<ToolResult, ToolError>,
    ) {
        debug!(
            tool_call_id = tool_call_id.as_str(),
            ok = result.is_ok(),
            "sync tool result"
        );
        let output_text = extract_tool_output(&result);

        let truncated = output_text.len() > 500;
        let display = if truncated {
            format!(
                "{}...\n({} chars total)",
                &output_text[..500],
                output_text.len()
            )
        } else {
            output_text
        };

        // Log tool result
        if let Some(ref logger) = self.session_logger {
            let turn = self.agent_host.as_ref().expect("agent").turn_number;
            let _ = logger.append(&SessionEvent::ToolResult {
                turn,
                tool_call_id: tool_call_id.as_str().to_string(),
                truncated,
            });
        }

        self.entries.push(ChatEntry::ToolResult {
            name: tool_name,
            output: display,
            is_error: result.is_err(),
        });
        let _ = terminal.draw(|f| self.render(f));

        let (_events, actions) = self
            .agent_host
            .as_mut()
            .expect("agent")
            .transition(|runtime, transcript, artifacts, turn| {
                match runtime {
                    AgentRuntime::ExecutingTools(exec) => {
                        debug!(tool_call_id = tool_call_id.as_str(), "on_tool_done (sync)");
                        TransitionParts::from(exec.on_tool_done(tool_call_id, result, transcript, artifacts, turn).into_parts())
                    }
                    other => crate::agent_host::AgentHost::abort_compacting_or_pass_through(
                        other, transcript, artifacts, turn,
                    ),
                }
            });
        self.handle_actions(terminal, actions);
    }

    pub(crate) fn poll_running_tasks(&mut self, terminal: &mut ratatui::DefaultTerminal) {
        if self.running_tasks.is_empty() {
            return;
        }
        trace!(running = self.running_tasks.len(), "polling async tasks");
        let mut remaining = Vec::new();
        let mut just_completed: Vec<(ToolCallId, String, Result<ToolResult, ToolError>)> = Vec::new();

        for mut task in std::mem::take(&mut self.running_tasks) {
            let mut done = false;
            while let Some(event) = task.stream.try_recv() {
                match event {
                    ToolEvent::Update(update) => {
                        // Notify the agent of the update (mutable, non-consuming)
                        if let AgentRuntime::ExecutingTools(exec) = self.agent_host.as_mut().expect("agent").runtime_mut() {
                            let _events = exec.on_tool_update(update);
                        }
                    }
                    ToolEvent::Done(result) => {
                        debug!(
                            tool_call_id = task.tool_call_id.as_str(),
                            ok = result.is_ok(),
                            "async tool completed"
                        );
                        just_completed.push((task.tool_call_id.clone(), task.tool_name.clone(), result));
                        done = true;
                        break;
                    }
                }
            }
            if !done {
                trace!(
                    tool_call_id = task.tool_call_id.as_str(),
                    "async tool still running"
                );
                remaining.push(task);
            }
        }
        self.running_tasks = remaining;

        if just_completed.is_empty() {
            return;
        }

        // Apply UI updates and typestate transitions for each completed task.
        let mut all_actions = Vec::new();
        for (tool_call_id, tool_name, result) in just_completed {
            let output_text = extract_tool_output(&result);

            // Log async tool result
            if let Some(ref logger) = self.session_logger {
                let turn = self.agent_host.as_ref().expect("agent").turn_number;
                let _ = logger.append(&SessionEvent::ToolResult {
                    turn,
                    tool_call_id: tool_call_id.as_str().to_string(),
                    truncated: false,
                });
            }
            self.entries.push(ChatEntry::ToolResult {
                name: tool_name,
                output: output_text,
                is_error: result.is_err(),
            });
            let _ = terminal.draw(|f| self.render(f));

            let (_events, actions) = self
                .agent_host
                .as_mut()
                .expect("agent")
                .transition(|runtime, transcript, artifacts, turn| {
                    match runtime {
                        AgentRuntime::ExecutingTools(exec) => {
                            debug!(tool_call_id = tool_call_id.as_str(), "on_tool_done (async)");
                            TransitionParts::from(exec.on_tool_done(tool_call_id, result, transcript, artifacts, turn).into_parts())
                        }
                        other => crate::agent_host::AgentHost::abort_compacting_or_pass_through(
                            other, transcript, artifacts, turn,
                        ),
                    }
                });
            all_actions.extend(actions);
        }

        // If all async tools completed, auto-continue.
        if self.running_tasks.is_empty() && self.agent().state().pending_tool_calls.is_empty() {
            self.auto_continue(terminal);
        }

        let action_names: Vec<String> = all_actions
            .iter()
            .map(|a| match a {
                pi_core::AgentAction::StreamLlm { .. } => "StreamLlm".to_string(),
                pi_core::AgentAction::ExecuteTools { calls } => {
                    format!("ExecuteTools({})", calls.len())
                }
                pi_core::AgentAction::Finished => "Finished".to_string(),
                pi_core::AgentAction::WaitForInput { .. } => "WaitForInput".to_string(),
                pi_core::AgentAction::Summarize { .. } => "Summarize".to_string(),
                _ => "Other".to_string(),
            })
            .collect();
        debug!(?action_names, "async tool batch processed");
        self.handle_actions(terminal, all_actions);
    }

    /// Helper: auto-continue from ReadyToContinue → next directive.
    fn auto_continue(&mut self, terminal: &mut ratatui::DefaultTerminal) {
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
                    AgentRuntime::ReadyToContinue(ready) => TransitionParts::from(ready
                        .continue_turn(
                            transcript,
                            artifacts,
                            turn,
                            &budget,
                            &compaction_prompt,
                        )
                        .into_parts()),
                    other => TransitionParts::from((vec![], vec![], other, transcript, artifacts, turn, vec![])),
                }
            });
        self.handle_actions(terminal, actions);
    }
}

fn extract_tool_output(result: &Result<ToolResult, ToolError>) -> String {
    match result {
        Ok(r) => r
            .content
            .iter()
            .filter_map(|c| {
                if let pi_core::Content::Text(t) = c {
                    Some(t.text.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Err(e) => e.message.clone(),
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
