use super::*;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(js_name = "createAgent")]
pub fn create_agent(options: AgentOptions) -> CreateAgentResult {
    console_error_panic_hook::set_once();
    init_tracing();
    info!("createAgent called");
    let core_options: pi_core::AgentOptions = try_conv!(options.try_into());
    let runtime = AgentRuntime::new(core_options);
    let handle = put_runtime(runtime, SessionState::default());
    info!(handle, "agent created");
    ok(CreateAgentOutput { handle })
}

#[wasm_bindgen(js_name = "prompt")]
pub fn prompt(handle: u32, input: PromptInput) -> StepResult {
    console_error_panic_hook::set_once();
    info!(handle, "prompt called");

    let core_prompt: pi_core::AgentMessage = match input.prompt {
        PromptRequest::Message(m) => try_conv!(m.try_into()),
        PromptRequest::Text { text } => pi_core::AgentMessage::user(text),
    };
    let core_tools: Vec<pi_core::ToolDefinition> = try_conv!(input
        .tools
        .into_iter()
        .map(|t| t.try_into())
        .collect::<Result<Vec<_>, _>>());

    let (runtime, session_state) = match take_runtime(handle) {
        Ok(r) => r,
        Err(e) => return err(&e),
    };

    let result = match runtime {
        AgentRuntime::Idle(idle) => {
            let Transition {
                events,
                actions,
                state,
                session_state,
            } = idle.start_turn(core_prompt, core_tools, session_state);
            put_runtime(state.into_runtime(), session_state);
            Ok((events, actions))
        }
        AgentRuntime::Finished(finished) => {
            let (idle, session_state) = finished.into_idle(session_state);
            let Transition {
                events,
                actions,
                state,
                session_state,
            } = idle.start_turn(core_prompt, core_tools, session_state);
            put_runtime(state.into_runtime(), session_state);
            Ok((events, actions))
        }
        AgentRuntime::Aborted(aborted) => {
            let (idle, session_state) = aborted.into_idle(session_state);
            let Transition {
                events,
                actions,
                state,
                session_state,
            } = idle.start_turn(core_prompt, core_tools, session_state);
            put_runtime(state.into_runtime(), session_state);
            Ok((events, actions))
        }
        AgentRuntime::WaitingTools(mut waiting) => {
            let (events, actions) = waiting
                .submit_user_message(core_prompt)
                .into_events_actions();
            put_runtime(waiting.into_runtime(), session_state);
            Ok((events, actions))
        }
        other => {
            let actual = runtime_phase_name(&other);
            put_runtime(other, session_state);
            Err(HostError::WrongPhase {
                expected: "Idle, Finished, Aborted, or WaitingTools",
                actual,
            })
        }
    };

    match result {
        Ok((events, actions)) => ok(StepOutput {
            events: try_conv!(events
                .into_iter()
                .map(|e| e.try_into())
                .collect::<Result<Vec<_>, _>>()),
            actions: try_conv!(actions
                .into_iter()
                .map(|a| a.try_into())
                .collect::<Result<Vec<_>, _>>()),
        }),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "feedLlmChunk")]
pub fn feed_llm_chunk(handle: u32, chunk: LlmChunk) -> EventsResult {
    console_error_panic_hook::set_once();
    info!(handle, "feedLlmChunk called");

    let core_chunk: pi_core::LlmChunk = try_conv!(chunk.try_into());
    let (runtime, session_state) = match take_runtime(handle) {
        Ok(r) => r,
        Err(e) => return err(&e),
    };

    let result = match runtime {
        AgentRuntime::Streaming(mut streaming) => {
            let events = streaming.feed_llm_chunk(core_chunk);
            put_runtime(streaming.into_runtime(), session_state);
            Ok(events)
        }
        other => {
            let actual = runtime_phase_name(&other);
            put_runtime(other, session_state);
            Err(HostError::WrongPhase {
                expected: "Streaming",
                actual,
            })
        }
    };

    match result {
        Ok(events) => ok(EventsOutput {
            events: try_conv!(events
                .into_iter()
                .map(|e| e.try_into())
                .collect::<Result<Vec<_>, _>>()),
        }),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "onLlmDone")]
pub fn on_llm_done(handle: u32, result: LlmResult) -> StepResult {
    console_error_panic_hook::set_once();
    info!(handle, "onLlmDone called");

    let core_result: pi_core::LlmResult = try_conv!(result.try_into());
    let (runtime, session_state) = match take_runtime(handle) {
        Ok(r) => r,
        Err(e) => return err(&e),
    };

    let result = match runtime {
        AgentRuntime::Streaming(streaming) => {
            let (events, actions, new_runtime, session_state) = streaming
                .finish_llm(core_result, session_state)
                .into_parts();
            put_runtime(new_runtime, session_state);
            Ok((events, actions))
        }
        other => {
            let actual = runtime_phase_name(&other);
            put_runtime(other, session_state);
            Err(HostError::WrongPhase {
                expected: "Streaming",
                actual,
            })
        }
    };

    match result {
        Ok((events, actions)) => ok(StepOutput {
            events: try_conv!(events
                .into_iter()
                .map(|e| e.try_into())
                .collect::<Result<Vec<_>, _>>()),
            actions: try_conv!(actions
                .into_iter()
                .map(|a| a.try_into())
                .collect::<Result<Vec<_>, _>>()),
        }),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "onToolDone")]
pub fn on_tool_done(handle: u32, tool_call_id: String, payload: ToolDonePayload) -> StepResult {
    console_error_panic_hook::set_once();
    info!(
        handle,
        tool_call_id = tool_call_id.as_str(),
        "onToolDone called"
    );

    let id = pi_core::ToolCallId::new(&tool_call_id);
    let core_result: Result<pi_core::ToolResult, pi_core::ToolError> = match payload {
        ToolDonePayload::Failure { error } => Err(try_conv!(error.try_into())),
        ToolDonePayload::Success { result } => Ok(try_conv!(result.try_into())),
    };

    let (runtime, session_state) = match take_runtime(handle) {
        Ok(r) => r,
        Err(e) => return err(&e),
    };

    let result = match runtime {
        AgentRuntime::WaitingTools(waiting) => {
            let (events, actions, new_runtime, session_state) = waiting
                .on_tool_done(id, core_result, session_state)
                .into_parts();
            put_runtime(new_runtime, session_state);
            Ok((events, actions))
        }
        other => {
            let actual = runtime_phase_name(&other);
            put_runtime(other, session_state);
            Err(HostError::WrongPhase {
                expected: "WaitingTools",
                actual,
            })
        }
    };

    match result {
        Ok((events, actions)) => ok(StepOutput {
            events: try_conv!(events
                .into_iter()
                .map(|e| e.try_into())
                .collect::<Result<Vec<_>, _>>()),
            actions: try_conv!(actions
                .into_iter()
                .map(|a| a.try_into())
                .collect::<Result<Vec<_>, _>>()),
        }),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "onToolStarted")]
pub fn on_tool_started(handle: u32, tool_call_id: String) -> EventsResult {
    console_error_panic_hook::set_once();
    info!(
        handle,
        tool_call_id = tool_call_id.as_str(),
        "onToolStarted called"
    );

    let id = pi_core::ToolCallId::new(&tool_call_id);
    let (runtime, session_state) = match take_runtime(handle) {
        Ok(r) => r,
        Err(e) => return err(&e),
    };

    let result = match runtime {
        AgentRuntime::WaitingTools(mut waiting) => {
            let events = waiting.on_tool_started(id);
            put_runtime(waiting.into_runtime(), session_state);
            Ok(events)
        }
        other => {
            let actual = runtime_phase_name(&other);
            put_runtime(other, session_state);
            Err(HostError::WrongPhase {
                expected: "WaitingTools",
                actual,
            })
        }
    };

    match result {
        Ok(events) => ok(EventsOutput {
            events: try_conv!(events
                .into_iter()
                .map(|e| e.try_into())
                .collect::<Result<Vec<_>, _>>()),
        }),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "onToolUpdate")]
pub fn on_tool_update(handle: u32, update: ToolExecutionUpdate) -> EventsResult {
    console_error_panic_hook::set_once();

    let core_update: pi_core::ToolExecutionUpdate = try_conv!(update.try_into());
    let (runtime, session_state) = match take_runtime(handle) {
        Ok(r) => r,
        Err(e) => return err(&e),
    };

    let result = match runtime {
        AgentRuntime::WaitingTools(mut waiting) => {
            let events = waiting.on_tool_update(core_update);
            put_runtime(waiting.into_runtime(), session_state);
            Ok(events)
        }
        other => {
            let actual = runtime_phase_name(&other);
            put_runtime(other, session_state);
            Err(HostError::WrongPhase {
                expected: "WaitingTools",
                actual,
            })
        }
    };

    match result {
        Ok(events) => ok(EventsOutput {
            events: try_conv!(events
                .into_iter()
                .map(|e| e.try_into())
                .collect::<Result<Vec<_>, _>>()),
        }),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "onToolCancelled")]
pub fn on_tool_cancelled(handle: u32, tool_call_id: String, reason: CancelReason) -> StepResult {
    console_error_panic_hook::set_once();

    let id = pi_core::ToolCallId::new(&tool_call_id);
    let core_reason: pi_core::CancelReason = try_conv!(reason.try_into());
    let (runtime, session_state) = match take_runtime(handle) {
        Ok(r) => r,
        Err(e) => return err(&e),
    };

    let result = match runtime {
        AgentRuntime::WaitingTools(waiting) => {
            let (events, actions, new_runtime, session_state) = waiting
                .cancel_tool(id, core_reason, session_state)
                .into_parts();
            put_runtime(new_runtime, session_state);
            Ok((events, actions))
        }
        other => {
            let actual = runtime_phase_name(&other);
            put_runtime(other, session_state);
            Err(HostError::WrongPhase {
                expected: "WaitingTools",
                actual,
            })
        }
    };

    match result {
        Ok((events, actions)) => ok(StepOutput {
            events: try_conv!(events
                .into_iter()
                .map(|e| e.try_into())
                .collect::<Result<Vec<_>, _>>()),
            actions: try_conv!(actions
                .into_iter()
                .map(|a| a.try_into())
                .collect::<Result<Vec<_>, _>>()),
        }),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "abort")]
pub fn abort(handle: u32) -> StepResult {
    console_error_panic_hook::set_once();
    info!(handle, "abort called");

    let (runtime, session_state) = match take_runtime(handle) {
        Ok(r) => r,
        Err(e) => return err(&e),
    };

    let result = match runtime {
        AgentRuntime::Streaming(streaming) => {
            let Transition {
                events,
                actions,
                state,
                session_state,
            } = streaming.abort(session_state);
            put_runtime(state.into_runtime(), session_state);
            Ok((events, actions))
        }
        AgentRuntime::WaitingTools(waiting) => {
            let Transition {
                events,
                actions,
                state,
                session_state,
            } = waiting.abort(session_state);
            put_runtime(state.into_runtime(), session_state);
            Ok((events, actions))
        }
        AgentRuntime::ReadyToContinue(ready) => {
            let Transition {
                events,
                actions,
                state,
                session_state,
            } = ready.abort(session_state);
            put_runtime(state.into_runtime(), session_state);
            Ok((events, actions))
        }
        other => {
            let actual = runtime_phase_name(&other);
            put_runtime(other, session_state);
            Err(HostError::WrongPhase {
                expected: "Streaming, WaitingTools, or ReadyToContinue",
                actual,
            })
        }
    };

    match result {
        Ok((events, actions)) => ok(StepOutput {
            events: try_conv!(events
                .into_iter()
                .map(|e| e.try_into())
                .collect::<Result<Vec<_>, _>>()),
            actions: try_conv!(actions
                .into_iter()
                .map(|a| a.try_into())
                .collect::<Result<Vec<_>, _>>()),
        }),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "continueTurn")]
pub fn continue_turn(handle: u32) -> StepResult {
    console_error_panic_hook::set_once();
    info!(handle, "continueTurn called");

    let (runtime, session_state) = match take_runtime(handle) {
        Ok(r) => r,
        Err(e) => return err(&e),
    };

    let result = match runtime {
        AgentRuntime::ReadyToContinue(ready) => {
            let Transition {
                events,
                actions,
                state,
                session_state,
            } = ready.continue_turn(session_state);
            put_runtime(state.into_runtime(), session_state);
            Ok((events, actions))
        }
        other => {
            let actual = runtime_phase_name(&other);
            put_runtime(other, session_state);
            Err(HostError::WrongPhase {
                expected: "ReadyToContinue",
                actual,
            })
        }
    };

    match result {
        Ok((events, actions)) => ok(StepOutput {
            events: try_conv!(events
                .into_iter()
                .map(|e| e.try_into())
                .collect::<Result<Vec<_>, _>>()),
            actions: try_conv!(actions
                .into_iter()
                .map(|a| a.try_into())
                .collect::<Result<Vec<_>, _>>()),
        }),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "steer")]
pub fn steer(handle: u32, message: AgentMessage) -> EventsResult {
    console_error_panic_hook::set_once();

    let core_message: pi_core::AgentMessage = try_conv!(message.try_into());
    let (runtime, session_state) = match take_runtime(handle) {
        Ok(r) => r,
        Err(e) => return err(&e),
    };

    let result = match runtime {
        AgentRuntime::Idle(mut idle) => {
            let events = idle.steer(core_message);
            put_runtime(idle.into_runtime(), session_state);
            Ok(events)
        }
        AgentRuntime::ReadyToContinue(mut ready) => {
            let events = ready.steer(core_message);
            put_runtime(ready.into_runtime(), session_state);
            Ok(events)
        }
        other => {
            let actual = runtime_phase_name(&other);
            put_runtime(other, session_state);
            Err(HostError::WrongPhase {
                expected: "Idle or ReadyToContinue",
                actual,
            })
        }
    };

    match result {
        Ok(events) => ok(EventsOutput {
            events: try_conv!(events
                .into_iter()
                .map(|e| e.try_into())
                .collect::<Result<Vec<_>, _>>()),
        }),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "followUp")]
pub fn follow_up(handle: u32, message: AgentMessage) -> EmptyResult {
    console_error_panic_hook::set_once();

    let core_message: pi_core::AgentMessage = try_conv!(message.try_into());
    let (runtime, session_state) = match take_runtime(handle) {
        Ok(r) => r,
        Err(e) => return err(&e),
    };

    let result = match runtime {
        AgentRuntime::Idle(mut idle) => {
            idle.follow_up(core_message);
            put_runtime(idle.into_runtime(), session_state);
            Ok(())
        }
        AgentRuntime::ReadyToContinue(mut ready) => {
            ready.follow_up(core_message);
            put_runtime(ready.into_runtime(), session_state);
            Ok(())
        }
        AgentRuntime::WaitingTools(mut waiting) => {
            waiting.submit_user_message(core_message);
            put_runtime(waiting.into_runtime(), session_state);
            Ok(())
        }
        other => {
            let actual = runtime_phase_name(&other);
            put_runtime(other, session_state);
            Err(HostError::WrongPhase {
                expected: "Idle, ReadyToContinue, or WaitingTools",
                actual,
            })
        }
    };

    match result {
        Ok(()) => ok(()),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "state")]
pub fn state(handle: u32) -> StateResult {
    console_error_panic_hook::set_once();

    let result = with_runtime(handle, |runtime, _session_state| runtime.state().clone());
    match result {
        Ok(core_state) => ok(StateOutput {
            state: try_conv!(core_state.try_into()),
        }),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "reset")]
pub fn reset(handle: u32) -> EmptyResult {
    console_error_panic_hook::set_once();

    let (runtime, _session_state) = match take_runtime(handle) {
        Ok(r) => r,
        Err(e) => return err(&e),
    };
    let runtime = runtime.reset();
    put_runtime(runtime, SessionState::default());
    ok(())
}

#[wasm_bindgen(js_name = "destroyAgent")]
pub fn destroy_agent(handle: u32) -> EmptyResult {
    console_error_panic_hook::set_once();

    match take_runtime(handle) {
        Ok(_) => ok(()),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "projectContext")]
pub fn project_context_export(input: ProjectionInput) -> ProjectionResult {
    console_error_panic_hook::set_once();
    info!("projectContext called");

    let core_input: pi_core::ProjectionInput = try_conv!(input.try_into());
    let core_output = project_context(core_input);
    let output: ProjectionOutput = try_conv!(core_output.try_into());
    ok(output)
}

#[wasm_bindgen(js_name = "getSessionState")]
pub fn get_session_state(handle: u32) -> SessionStateResult {
    console_error_panic_hook::set_once();

    let (runtime, session_state) = match take_runtime(handle) {
        Ok(r) => r,
        Err(e) => return err(&e),
    };
    let core_state = session_state.clone();
    put_runtime(runtime, session_state);
    ok(SessionStateOutput {
        state: try_conv!(core_state.try_into()),
    })
}

#[wasm_bindgen(js_name = "setSessionState")]
pub fn set_session_state(handle: u32, state: crate::dto::SessionState) -> EmptyResult {
    console_error_panic_hook::set_once();

    let core_state: pi_core::SessionState = try_conv!(state.try_into());
    let (runtime, _session_state) = match take_runtime(handle) {
        Ok(r) => r,
        Err(e) => return err(&e),
    };
    put_runtime(runtime, core_state);
    ok(())
}

#[wasm_bindgen(js_name = "getSessionBranch")]
pub fn get_session_branch(handle: u32) -> SessionBranchResult {
    console_error_panic_hook::set_once();

    let (runtime, session_state) = match take_runtime(handle) {
        Ok(r) => r,
        Err(e) => return err(&e),
    };
    let agent = runtime.into_agent();
    let entries = agent.session_branch(&session_state);
    put_runtime(AgentRuntime::from_agent(agent), session_state);
    ok(SessionBranchOutput {
        entries: try_conv!(entries
            .into_iter()
            .map(|e| e.try_into())
            .collect::<Result<Vec<_>, _>>()),
    })
}

#[wasm_bindgen(js_name = "moveTo")]
pub fn move_to(handle: u32, target_id: String, summary: Option<BranchSummary>) -> MoveToResult {
    console_error_panic_hook::set_once();

    let core_summary: Option<pi_core::BranchSummary> = summary.map(|s| try_conv!(s.try_into()));
    let (runtime, mut session_state) = match take_runtime(handle) {
        Ok(r) => r,
        Err(e) => return err(&e),
    };
    let mut agent = runtime.into_agent();
    let id = agent.move_to(&mut session_state, &target_id, core_summary);
    put_runtime(AgentRuntime::from_agent(agent), session_state);
    ok(MoveToOutput {
        summary_entry_id: id,
    })
}

#[wasm_bindgen(js_name = "appendSessionEntry")]
pub fn append_session_entry(handle: u32, entry: SessionEntry) -> EmptyResult {
    console_error_panic_hook::set_once();

    let core_entry: pi_core::SessionEntry = try_conv!(entry.try_into());
    let (runtime, mut session_state) = match take_runtime(handle) {
        Ok(r) => r,
        Err(e) => return err(&e),
    };
    let mut agent = runtime.into_agent();
    agent.append_session_entry(&mut session_state, core_entry);
    put_runtime(AgentRuntime::from_agent(agent), session_state);
    ok(())
}

#[wasm_bindgen(js_name = "estimateTokens")]
pub fn estimate_tokens_export(input: EstimateTokensInput) -> EstimateTokensResult {
    console_error_panic_hook::set_once();

    let core_messages: Vec<pi_core::AgentMessage> = try_conv!(input
        .messages
        .into_iter()
        .map(|m| m.try_into())
        .collect::<Result<Vec<_>, _>>());

    let tokens = pi_core::estimate_tokens(&core_messages);
    ok(EstimateTokensOutput { tokens })
}

#[wasm_bindgen(js_name = "estimateTokensForText")]
pub fn estimate_tokens_for_text_export(text: String) -> EstimateTokensResult {
    console_error_panic_hook::set_once();

    let tokens = pi_core::estimate_tokens_for_text(&text);
    ok(EstimateTokensOutput { tokens })
}
