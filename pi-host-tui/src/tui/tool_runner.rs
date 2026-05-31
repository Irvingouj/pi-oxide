use pi_core::{AgentRuntime, ToolCall, ToolCallId, ToolError, ToolResult};
use tracing::{debug, trace, warn};

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
            let compaction_prompt = self
                .host_state
                .as_ref()
                .map(|h| h.compaction_prompt.clone())
                .unwrap_or_default();
            let budget = self.budget.clone();
            let transcript = std::mem::take(&mut self.transcript);
            let artifacts = std::mem::take(&mut self.artifacts);
            let (_events, actions, new_runtime, transcript, artifacts, turn_number, _markers) =
                match runtime {
                    AgentRuntime::ReadyToContinue(ready) => ready
                        .continue_turn(
                            transcript,
                            artifacts,
                            self.turn_number,
                            &budget,
                            &compaction_prompt,
                        )
                        .into_parts(),
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
            self.handle_actions(terminal, actions);
        }
    }

    pub(crate) fn on_tool_result(
        &mut self,
        terminal: &mut ratatui::DefaultTerminal,
        tool_call_id: ToolCallId,
        result: Result<ToolResult, ToolError>,
    ) {
        debug!(
            tool_call_id = tool_call_id.as_str(),
            ok = result.is_ok(),
            "sync tool result"
        );
        let output_text = match &result {
            Ok(tool_result) => tool_result
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
        let transcript = std::mem::take(&mut self.transcript);
        let artifacts = std::mem::take(&mut self.artifacts);
        let (_events, actions, new_runtime, transcript, artifacts, turn_number, _markers) =
            match runtime {
                AgentRuntime::WaitingTools(waiting) => {
                    debug!(tool_call_id = tool_call_id.as_str(), "on_tool_done (sync)");
                    waiting
                        .on_tool_done(
                            tool_call_id,
                            result,
                            transcript,
                            artifacts,
                            self.turn_number,
                        )
                        .into_parts()
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
        self.handle_actions(terminal, actions);
    }

    pub(crate) fn poll_running_tasks(&mut self, terminal: &mut ratatui::DefaultTerminal) {
        if self.running_tasks.is_empty() {
            return;
        }
        trace!(running = self.running_tasks.len(), "polling async tasks");
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
                        debug!(
                            tool_call_id = task.tool_call_id.as_str(),
                            ok = result.is_ok(),
                            "async tool completed"
                        );
                        just_completed.push((task.tool_call_id.clone(), result));
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

        // Apply UI updates and typestate transitions atomically per completed task
        let mut transcript = std::mem::take(&mut self.transcript);
        let mut artifacts = std::mem::take(&mut self.artifacts);
        let mut turn_number = self.turn_number;
        let mut runtime = self.agent.take().unwrap();
        let mut all_actions = Vec::new();
        for (tool_call_id, result) in just_completed {
            let output_text = match &result {
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
            };
            self.entries.push(ChatEntry::ToolResult {
                name: tool_call_id.as_str().to_string(),
                output: output_text,
                is_error: result.is_err(),
            });
            let _ = terminal.draw(|f| self.render(f));

            let (
                events,
                actions,
                new_runtime,
                new_transcript,
                new_artifacts,
                new_turn_number,
                _markers,
            ) = match runtime {
                AgentRuntime::WaitingTools(waiting) => {
                    debug!(tool_call_id = tool_call_id.as_str(), "on_tool_done (async)");
                    waiting
                        .on_tool_done(tool_call_id, result, transcript, artifacts, turn_number)
                        .into_parts()
                }
                AgentRuntime::Compacting(compacting) => {
                    let (ev, act, state, transcript, artifacts, tn, m) = compacting
                        .abort(transcript, artifacts, turn_number)
                        .into_parts();
                    (ev, act, state.into_runtime(), transcript, artifacts, tn, m)
                }
                other => {
                    warn!("runtime not WaitingTools when async tool completed");
                    (
                        vec![],
                        vec![],
                        other,
                        transcript,
                        artifacts,
                        turn_number,
                        vec![],
                    )
                }
            };
            runtime = new_runtime;
            transcript = new_transcript;
            artifacts = new_artifacts;
            turn_number = new_turn_number;
            all_actions.extend(actions);
            let _ = events;
        }

        self.transcript = transcript;
        self.artifacts = artifacts;
        self.turn_number = turn_number;
        self.agent = Some(runtime);

        // If all async tools completed, the runtime is ReadyToContinue.
        // Auto-continue to get the next StreamLlm action (send tool results back to LLM).
        if self.running_tasks.is_empty() && self.agent().state().pending_tool_calls.is_empty() {
            let runtime = self.agent.take().unwrap();
            let compaction_prompt = self
                .host_state
                .as_ref()
                .map(|h| h.compaction_prompt.clone())
                .unwrap_or_default();
            let budget = self.budget.clone();
            let transcript = std::mem::take(&mut self.transcript);
            let artifacts = std::mem::take(&mut self.artifacts);
            let (
                _events,
                continue_actions,
                new_runtime,
                transcript,
                artifacts,
                turn_number,
                _markers,
            ) = match runtime {
                AgentRuntime::ReadyToContinue(ready) => ready
                    .continue_turn(
                        transcript,
                        artifacts,
                        self.turn_number,
                        &budget,
                        &compaction_prompt,
                    )
                    .into_parts(),
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
            all_actions.extend(continue_actions);
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
