use wasm_bindgen::prelude::*;
use super::*;

#[wasm_bindgen(js_name = "createHostAgent")]
pub fn create_host_agent(options: AgentOptions, budget: ContextProjectionBudget) -> CreateHostAgentResult {
    console_error_panic_hook::set_once();
    init_tracing();
    info!("createHostAgent called");
    let core_options: pi_core::AgentOptions = try_conv!(options.try_into());
    let core_budget: pi_core::ContextProjectionBudget = try_conv!(budget.try_into());
    let runtime = AgentRuntime::new(core_options.clone());
    let mut host_state = HostState::new(core_options.system_prompt.clone(), core_budget);
    let core_session = runtime.session_state().clone();
    host_state.entries = core_session.entries;
    host_state.leaf_id = core_session.leaf_id;
    host_state.name = core_options
        .session_id
        .as_ref()
        .map(|s| s.0.clone())
        .unwrap_or_default();
    let agent = HostAgent { runtime, host_state };
    let handle = put_host_agent(agent);
    info!(handle, "host agent created");
    ok(CreateHostAgentOutput { handle })
}

#[wasm_bindgen(js_name = "destroyHostAgent")]
pub fn destroy_host_agent(handle: u32) -> EmptyResult {
    console_error_panic_hook::set_once();
    match take_host_agent(handle) {
        Ok(_) => ok(()),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "startTurn")]
pub fn start_turn(handle: u32, input: StartTurnInput) -> TurnResultResult {
    console_error_panic_hook::set_once();
    info!(handle, "startTurn called");

    let core_prompt: pi_core::AgentMessage = try_conv!(input.prompt.try_into());
    let core_tools: Vec<pi_core::ToolDefinition> = try_conv!(input
        .tools
        .into_iter()
        .map(|t| t.try_into())
        .collect::<Result<Vec<_>, _>>());

    let mut host_agent = match take_host_agent(handle) {
        Ok(a) => a,
        Err(e) => return err(&e),
    };

    // Sync host_state -> runtime before starting turn
    let state = pi_core::SessionState {
        entries: host_agent.host_state.entries.clone(),
        leaf_id: host_agent.host_state.leaf_id.clone(),
        name: host_agent.host_state.name.clone(),
    };
    host_agent.runtime.set_session_state(state);

    let result = match host_agent.runtime {
        AgentRuntime::Idle(idle) => {
            let Transition {
                events,
                actions,
                state,
            } = idle.start_turn(core_prompt, core_tools);
            host_agent.runtime = state.into_runtime();
            Ok((events, actions))
        }
        AgentRuntime::Finished(finished) => {
            let idle = finished.into_idle();
            let Transition {
                events,
                actions,
                state,
            } = idle.start_turn(core_prompt, core_tools);
            host_agent.runtime = state.into_runtime();
            Ok((events, actions))
        }
        AgentRuntime::Aborted(aborted) => {
            let idle = aborted.into_idle();
            let Transition {
                events,
                actions,
                state,
            } = idle.start_turn(core_prompt, core_tools);
            host_agent.runtime = state.into_runtime();
            Ok((events, actions))
        }
        AgentRuntime::WaitingTools(mut waiting) => {
            let (events, actions) = waiting
                .submit_user_message(core_prompt)
                .into_events_actions();
            host_agent.runtime = waiting.into_runtime();
            Ok((events, actions))
        }
        other => {
            let actual = runtime_phase_name(&other);
            host_agent.runtime = other;
            Err(HostError::WrongPhase {
                expected: "Idle, Finished, Aborted, or WaitingTools",
                actual,
            })
        }
    };

    match result {
        Ok((events, actions)) => {
            // Sync runtime -> host_state before converting directives
            let core_state = host_agent.runtime.session_state().clone();
            host_agent.host_state.entries = core_state.entries;
            host_agent.host_state.leaf_id = core_state.leaf_id;
            let directives = try_conv!(convert_actions_to_directives(
                &mut host_agent.host_state,
                actions
            ));
            let dto_events = try_conv!(convert_events(events));
            put_host_agent(host_agent);
            ok(TurnResultOutput {
                events: dto_events,
                directives,
            })
        }
        Err(e) => {
            put_host_agent(host_agent);
            err(&e)
        }
    }
}

#[wasm_bindgen(js_name = "hostFeedLlmChunk")]
pub fn host_feed_llm_chunk(handle: u32, chunk: LlmChunk) -> TurnResultResult {
    console_error_panic_hook::set_once();
    info!(handle, "hostFeedLlmChunk called");

    let core_chunk: pi_core::LlmChunk = try_conv!(chunk.try_into());
    let mut host_agent = match take_host_agent(handle) {
        Ok(a) => a,
        Err(e) => return err(&e),
    };

    let result = match host_agent.runtime {
        AgentRuntime::Streaming(mut streaming) => {
            let events = streaming.feed_llm_chunk(core_chunk);
            host_agent.runtime = streaming.into_runtime();
            Ok(events)
        }
        other => {
            let actual = runtime_phase_name(&other);
            host_agent.runtime = other;
            Err(HostError::WrongPhase {
                expected: "Streaming",
                actual,
            })
        }
    };

    put_host_agent(host_agent);
    match result {
        Ok(events) => ok(TurnResultOutput {
            events: try_conv!(convert_events(events)),
            directives: vec![],
        }),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "hostLlmDone")]
pub fn host_llm_done(handle: u32, result: LlmResult) -> TurnResultResult {
    console_error_panic_hook::set_once();
    info!(handle, "hostLlmDone called");

    let core_result: pi_core::LlmResult = try_conv!(result.try_into());
    let mut host_agent = match take_host_agent(handle) {
        Ok(a) => a,
        Err(e) => return err(&e),
    };

    let result = match host_agent.runtime {
        AgentRuntime::Streaming(streaming) => {
            let (events, actions, new_runtime) = streaming.finish_llm(core_result).into_parts();
            host_agent.runtime = new_runtime;
            Ok((events, actions))
        }
        other => {
            let actual = runtime_phase_name(&other);
            host_agent.runtime = other;
            Err(HostError::WrongPhase {
                expected: "Streaming",
                actual,
            })
        }
    };

    match result {
        Ok((events, actions)) => {
            // Sync runtime -> host_state before converting directives
            let core_state = host_agent.runtime.session_state().clone();
            host_agent.host_state.entries = core_state.entries;
            host_agent.host_state.leaf_id = core_state.leaf_id;
            let directives = try_conv!(convert_actions_to_directives(
                &mut host_agent.host_state,
                actions
            ));
            let dto_events = try_conv!(convert_events(events));
            put_host_agent(host_agent);
            ok(TurnResultOutput {
                events: dto_events,
                directives,
            })
        }
        Err(e) => {
            put_host_agent(host_agent);
            err(&e)
        }
    }
}

#[wasm_bindgen(js_name = "hostToolDone")]
pub fn host_tool_done(handle: u32, id: ToolCallId, result: ToolResult) -> TurnResultResult {
    console_error_panic_hook::set_once();
    info!(handle, "hostToolDone called");

    let core_id: pi_core::ToolCallId = try_conv!(id.try_into());
    let core_result: pi_core::ToolResult = try_conv!(result.try_into());

    // Extract text for artifact storage before consuming the result.
    let artifact_text = core_result
        .content
        .iter()
        .filter_map(|c| {
            if let pi_core::Content::Text(tc) = c {
                Some(tc.text.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    let mut host_agent = match take_host_agent(handle) {
        Ok(a) => a,
        Err(e) => return err(&e),
    };

    let result = match host_agent.runtime {
        AgentRuntime::WaitingTools(waiting) => {
            let (events, actions, new_runtime) =
                waiting.on_tool_done(core_id.clone(), Ok(core_result)).into_parts();
            host_agent.runtime = new_runtime;
            // Store artifact text if this tool call was replaced by projection.
            if let Some(pi_core::ToolProjectionState::Replaced { replacement, .. }) =
                host_agent.host_state.projection_state.tools.get(core_id.as_str())
            {
                host_agent
                    .host_state
                    .store_artifact(replacement.artifact_id.clone(), artifact_text);
            }
            Ok((events, actions))
        }
        other => {
            let actual = runtime_phase_name(&other);
            host_agent.runtime = other;
            Err(HostError::WrongPhase {
                expected: "WaitingTools",
                actual,
            })
        }
    };

    match result {
        Ok((events, actions)) => {
            // Sync runtime -> host_state before converting directives
            let core_state = host_agent.runtime.session_state().clone();
            host_agent.host_state.entries = core_state.entries;
            host_agent.host_state.leaf_id = core_state.leaf_id;
            let mut directives = try_conv!(convert_actions_to_directives(
                &mut host_agent.host_state,
                actions
            ));
            // If the agent is ready to continue but didn't emit any directive,
            // tell the host to wait for input (it should then call hostContinueTurn).
            if matches!(host_agent.runtime, AgentRuntime::ReadyToContinue(_)) {
                directives.push(HostDirective::WaitForInput {
                    mode: WaitMode::Any,
                });
            }
            let dto_events = try_conv!(convert_events(events));
            put_host_agent(host_agent);
            ok(TurnResultOutput {
                events: dto_events,
                directives,
            })
        }
        Err(e) => {
            put_host_agent(host_agent);
            err(&e)
        }
    }
}

#[wasm_bindgen(js_name = "hostAcceptCompaction")]
pub fn host_accept_compaction(
    handle: u32,
    summary: String,
    _compacted_entry_ids: Vec<String>,
) -> TurnResultResult {
    console_error_panic_hook::set_once();
    let mut host_agent = match take_host_agent(handle) {
        Ok(a) => a,
        Err(e) => return err(&e),
    };

    if let Some((_, plan)) = host_agent.host_state.pending_compaction_plans.pop() {
        host_agent.host_state.accept_compaction(plan, summary);
    }

    put_host_agent(host_agent);
    ok(TurnResultOutput {
        events: vec![],
        directives: vec![HostDirective::Persist],
    })
}

#[wasm_bindgen(js_name = "hostSteer")]
pub fn host_steer(handle: u32, message: AgentMessage) -> TurnResultResult {
    console_error_panic_hook::set_once();

    let core_message: pi_core::AgentMessage = try_conv!(message.try_into());
    let mut host_agent = match take_host_agent(handle) {
        Ok(a) => a,
        Err(e) => return err(&e),
    };

    let result = match host_agent.runtime {
        AgentRuntime::Idle(mut idle) => {
            let events = idle.steer(core_message);
            host_agent.runtime = idle.into_runtime();
            Ok(events)
        }
        AgentRuntime::ReadyToContinue(mut ready) => {
            let events = ready.steer(core_message);
            host_agent.runtime = ready.into_runtime();
            Ok(events)
        }
        other => {
            let actual = runtime_phase_name(&other);
            host_agent.runtime = other;
            Err(HostError::WrongPhase {
                expected: "Idle or ReadyToContinue",
                actual,
            })
        }
    };

    put_host_agent(host_agent);
    match result {
        Ok(events) => ok(TurnResultOutput {
            events: try_conv!(convert_events(events)),
            directives: vec![],
        }),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "hostContinueTurn")]
pub fn host_continue_turn(handle: u32) -> TurnResultResult {
    console_error_panic_hook::set_once();
    info!(handle, "hostContinueTurn called");

    let mut host_agent = match take_host_agent(handle) {
        Ok(a) => a,
        Err(e) => return err(&e),
    };

    let result = match host_agent.runtime {
        AgentRuntime::ReadyToContinue(ready) => {
            let Transition {
                events,
                actions,
                state,
            } = ready.continue_turn();
            host_agent.runtime = state.into_runtime();
            Ok((events, actions))
        }
        other => {
            let actual = runtime_phase_name(&other);
            host_agent.runtime = other;
            Err(HostError::WrongPhase {
                expected: "ReadyToContinue",
                actual,
            })
        }
    };

    match result {
        Ok((events, actions)) => {
            // Sync runtime -> host_state before converting directives
            let core_state = host_agent.runtime.session_state().clone();
            host_agent.host_state.entries = core_state.entries;
            host_agent.host_state.leaf_id = core_state.leaf_id;
            let directives = try_conv!(convert_actions_to_directives(
                &mut host_agent.host_state,
                actions
            ));
            let dto_events = try_conv!(convert_events(events));
            put_host_agent(host_agent);
            ok(TurnResultOutput {
                events: dto_events,
                directives,
            })
        }
        Err(e) => {
            put_host_agent(host_agent);
            err(&e)
        }
    }
}

#[wasm_bindgen(js_name = "hostReset")]
pub fn host_reset(handle: u32) -> EmptyResult {
    console_error_panic_hook::set_once();
    let mut host_agent = match take_host_agent(handle) {
        Ok(a) => a,
        Err(e) => return err(&e),
    };
    let system_prompt = host_agent.host_state.system_prompt.clone();
    let budget = host_agent.host_state.budget.clone();
    host_agent.runtime = host_agent.runtime.reset();
    host_agent.host_state = HostState::new(system_prompt, budget);
    put_host_agent(host_agent);
    ok(())
}

#[wasm_bindgen(js_name = "hostToolCancelled")]
pub fn host_tool_cancelled(handle: u32, tool_call_id: String, reason: CancelReason) -> TurnResultResult {
    console_error_panic_hook::set_once();
    info!(handle, "hostToolCancelled called");

    let id = pi_core::ToolCallId::new(&tool_call_id);
    let core_reason: pi_core::CancelReason = try_conv!(reason.try_into());
    let mut host_agent = match take_host_agent(handle) {
        Ok(a) => a,
        Err(e) => return err(&e),
    };

    let result = match host_agent.runtime {
        AgentRuntime::WaitingTools(waiting) => {
            let (events, actions, new_runtime) = waiting.cancel_tool(id, core_reason).into_parts();
            host_agent.runtime = new_runtime;
            Ok((events, actions))
        }
        other => {
            let actual = runtime_phase_name(&other);
            host_agent.runtime = other;
            Err(HostError::WrongPhase {
                expected: "WaitingTools",
                actual,
            })
        }
    };

    match result {
        Ok((events, actions)) => {
            // Sync runtime -> host_state before converting directives
            let core_state = host_agent.runtime.session_state().clone();
            host_agent.host_state.entries = core_state.entries;
            host_agent.host_state.leaf_id = core_state.leaf_id;
            let directives = try_conv!(convert_actions_to_directives(
                &mut host_agent.host_state,
                actions
            ));
            let dto_events = try_conv!(convert_events(events));
            put_host_agent(host_agent);
            ok(TurnResultOutput {
                events: dto_events,
                directives,
            })
        }
        Err(e) => {
            put_host_agent(host_agent);
            err(&e)
        }
    }
}

#[wasm_bindgen(js_name = "hostAbort")]
pub fn host_abort(handle: u32) -> TurnResultResult {
    console_error_panic_hook::set_once();
    info!(handle, "hostAbort called");

    let mut host_agent = match take_host_agent(handle) {
        Ok(a) => a,
        Err(e) => return err(&e),
    };

    let result = match host_agent.runtime {
        AgentRuntime::Streaming(streaming) => {
            let Transition {
                events,
                actions,
                state,
            } = streaming.abort();
            host_agent.runtime = state.into_runtime();
            Ok((events, actions))
        }
        AgentRuntime::WaitingTools(waiting) => {
            let Transition {
                events,
                actions,
                state,
            } = waiting.abort();
            host_agent.runtime = state.into_runtime();
            Ok((events, actions))
        }
        AgentRuntime::ReadyToContinue(ready) => {
            let Transition {
                events,
                actions,
                state,
            } = ready.abort();
            host_agent.runtime = state.into_runtime();
            Ok((events, actions))
        }
        other => {
            let actual = runtime_phase_name(&other);
            host_agent.runtime = other;
            Err(HostError::WrongPhase {
                expected: "Streaming, WaitingTools, or ReadyToContinue",
                actual,
            })
        }
    };

    match result {
        Ok((events, actions)) => {
            // Sync runtime -> host_state before converting directives
            let core_state = host_agent.runtime.session_state().clone();
            host_agent.host_state.entries = core_state.entries;
            host_agent.host_state.leaf_id = core_state.leaf_id;
            let directives = try_conv!(convert_actions_to_directives(
                &mut host_agent.host_state,
                actions
            ));
            let dto_events = try_conv!(convert_events(events));
            put_host_agent(host_agent);
            ok(TurnResultOutput {
                events: dto_events,
                directives,
            })
        }
        Err(e) => {
            put_host_agent(host_agent);
            err(&e)
        }
    }
}

#[wasm_bindgen(js_name = "getHostAgentPersistData")]
pub fn get_host_agent_persist_data(handle: u32) -> HostStatePersistDataResult {
    console_error_panic_hook::set_once();
    let result = with_host_agent(handle, |host_agent| host_agent.host_state.get_persist_data());
    match result {
        Ok(data) => {
            let dto_data: PersistData = try_conv!(data.try_into());
            ok(HostStatePersistDataOutput { state: dto_data })
        }
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "restoreHostAgent")]
pub fn restore_host_agent(options: AgentOptions, data: PersistData) -> CreateHostAgentResult {
    console_error_panic_hook::set_once();
    init_tracing();
    let core_options: pi_core::AgentOptions = try_conv!(options.try_into());
    let core_data: crate::host_state::PersistData = try_conv!(data.try_into());
    let runtime = AgentRuntime::new(core_options);
    let host_state = HostState::restore(core_data);
    let agent = HostAgent { runtime, host_state };
    let handle = put_host_agent(agent);
    info!(handle, "host agent restored");
    ok(CreateHostAgentOutput { handle })
}
