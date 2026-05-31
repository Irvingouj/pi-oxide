use super::*;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(js_name = "createHostAgent")]
pub fn create_host_agent(
    options: AgentOptions,
    budget: ContextProjectionBudget,
) -> CreateHostAgentResult {
    console_error_panic_hook::set_once();
    init_tracing();
    info!("createHostAgent called");
    let core_options: pi_core::AgentOptions = try_conv!(options.try_into());
    let core_budget: pi_core::ContextProjectionBudget = try_conv!(budget.try_into());
    let runtime = AgentRuntime::new(core_options.clone());
    let host_state = HostState::new(core_options.system_prompt.clone(), "Summarize the following conversation into a concise summary that preserves the key information, decisions, and context.".to_string());
    let agent = HostAgent {
        runtime,
        host_state,
        transcript: vec![],
        artifacts: std::collections::BTreeMap::new(),
        turn_number: 0,
        budget: core_budget,
    };
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
    let budget = host_agent.budget.clone();
    let compaction_prompt = host_agent.host_state.compaction_prompt.clone();

    let result = match host_agent.runtime {
        AgentRuntime::Idle(idle) => {
            let transition = idle.start_turn(
                core_prompt,
                core_tools,
                host_agent.transcript,
                host_agent.artifacts,
                host_agent.turn_number,
                &budget,
                &compaction_prompt,
            );
            let (events, actions, runtime, transcript, artifacts, turn_number, markers) =
                transition.into_parts();
            host_agent.runtime = runtime;
            host_agent.transcript = transcript;
            host_agent.artifacts = artifacts;
            host_agent.turn_number = turn_number;
            for marker in &markers {
                if let pi_core::ChangeMarker::NewArtifacts { entry_ids } = marker {
                    host_agent
                        .host_state
                        .sync_artifacts_from_core(&host_agent.artifacts, entry_ids);
                }
            }
            Ok((events, actions, markers))
        }
        AgentRuntime::Finished(finished) => {
            let (idle, transcript, artifacts, turn_number) = finished.into_idle(
                host_agent.transcript,
                host_agent.artifacts,
                host_agent.turn_number,
            );
            let transition = idle.start_turn(
                core_prompt,
                core_tools,
                transcript,
                artifacts,
                turn_number,
                &budget,
                &compaction_prompt,
            );
            let (events, actions, runtime, transcript, artifacts, turn_number, markers) =
                transition.into_parts();
            host_agent.runtime = runtime;
            host_agent.transcript = transcript;
            host_agent.artifacts = artifacts;
            host_agent.turn_number = turn_number;
            for marker in &markers {
                if let pi_core::ChangeMarker::NewArtifacts { entry_ids } = marker {
                    host_agent
                        .host_state
                        .sync_artifacts_from_core(&host_agent.artifacts, entry_ids);
                }
            }
            Ok((events, actions, markers))
        }
        AgentRuntime::Aborted(aborted) => {
            let (idle, transcript, artifacts, turn_number) = aborted.into_idle(
                host_agent.transcript,
                host_agent.artifacts,
                host_agent.turn_number,
            );
            let transition = idle.start_turn(
                core_prompt,
                core_tools,
                transcript,
                artifacts,
                turn_number,
                &budget,
                &compaction_prompt,
            );
            let (events, actions, runtime, transcript, artifacts, turn_number, markers) =
                transition.into_parts();
            host_agent.runtime = runtime;
            host_agent.transcript = transcript;
            host_agent.artifacts = artifacts;
            host_agent.turn_number = turn_number;
            for marker in &markers {
                if let pi_core::ChangeMarker::NewArtifacts { entry_ids } = marker {
                    host_agent
                        .host_state
                        .sync_artifacts_from_core(&host_agent.artifacts, entry_ids);
                }
            }
            Ok((events, actions, markers))
        }
        AgentRuntime::WaitingTools(mut waiting) => {
            let (events, actions) = waiting
                .submit_user_message(core_prompt)
                .into_events_actions();
            host_agent.runtime = waiting.into_runtime();
            Ok((events, actions, vec![]))
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
        Ok((events, actions, markers)) => {
            let directives = try_conv!(convert_actions_to_directives(actions));
            let dto_events = try_conv!(convert_events(events));
            let dto_markers = try_conv!(markers
                .into_iter()
                .map(|m| ChangeMarkerDto::try_from(m))
                .collect::<Result<Vec<_>, _>>());
            put_host_agent(host_agent);
            ok(TurnResultOutput {
                events: dto_events,
                directives,
                markers: dto_markers,
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
            markers: vec![],
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

    let budget = host_agent.budget.clone();

    let result = match host_agent.runtime {
        AgentRuntime::Streaming(streaming) => {
            let (events, actions, new_runtime, transcript, artifacts, turn_number, markers) =
                streaming
                    .finish_llm(
                        core_result,
                        host_agent.transcript,
                        host_agent.artifacts,
                        host_agent.turn_number,
                        &budget,
                    )
                    .into_parts();
            host_agent.runtime = new_runtime;
            host_agent.transcript = transcript;
            host_agent.artifacts = artifacts;
            host_agent.turn_number = turn_number;
            for marker in &markers {
                if let pi_core::ChangeMarker::NewArtifacts { entry_ids } = marker {
                    host_agent
                        .host_state
                        .sync_artifacts_from_core(&host_agent.artifacts, entry_ids);
                }
            }
            Ok((events, actions, markers))
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
        Ok((events, actions, markers)) => {
            let directives = try_conv!(convert_actions_to_directives(actions));
            let dto_events = try_conv!(convert_events(events));
            let dto_markers = try_conv!(markers
                .into_iter()
                .map(|m| ChangeMarkerDto::try_from(m))
                .collect::<Result<Vec<_>, _>>());
            put_host_agent(host_agent);
            ok(TurnResultOutput {
                events: dto_events,
                directives,
                markers: dto_markers,
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

    let mut host_agent = match take_host_agent(handle) {
        Ok(a) => a,
        Err(e) => return err(&e),
    };

    let result = match host_agent.runtime {
        AgentRuntime::WaitingTools(waiting) => {
            let (events, actions, new_runtime, transcript, artifacts, turn_number, markers) =
                waiting
                    .on_tool_done(
                        core_id,
                        Ok(core_result),
                        host_agent.transcript,
                        host_agent.artifacts,
                        host_agent.turn_number,
                    )
                    .into_parts();
            host_agent.runtime = new_runtime;
            host_agent.transcript = transcript;
            host_agent.artifacts = artifacts;
            host_agent.turn_number = turn_number;
            for marker in &markers {
                if let pi_core::ChangeMarker::NewArtifacts { entry_ids } = marker {
                    host_agent
                        .host_state
                        .sync_artifacts_from_core(&host_agent.artifacts, entry_ids);
                }
            }
            Ok((events, actions, markers))
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
        Ok((events, actions, markers)) => {
            let mut directives = try_conv!(convert_actions_to_directives(actions));
            // If the agent is ready to continue but didn't emit any directive,
            // tell the host to wait for input (it should then call hostContinueTurn).
            if matches!(host_agent.runtime, AgentRuntime::ReadyToContinue(_)) {
                directives.push(HostDirective::WaitForInput {
                    mode: WaitMode::Any,
                });
            }
            let dto_events = try_conv!(convert_events(events));
            let dto_markers = try_conv!(markers
                .into_iter()
                .map(|m| ChangeMarkerDto::try_from(m))
                .collect::<Result<Vec<_>, _>>());
            put_host_agent(host_agent);
            ok(TurnResultOutput {
                events: dto_events,
                directives,
                markers: dto_markers,
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

    let result = match host_agent.runtime {
        AgentRuntime::Compacting(compacting) => {
            let transition = compacting.accept_summary(
                summary,
                host_agent.transcript,
                host_agent.artifacts,
                host_agent.turn_number,
                &host_agent.budget,
            );
            host_agent.runtime = transition.state.into_runtime();
            host_agent.transcript = transition.transcript;
            host_agent.artifacts = transition.artifacts;
            host_agent.turn_number = transition.turn_number;
            for marker in &transition.markers {
                if let pi_core::ChangeMarker::NewArtifacts { entry_ids } = marker {
                    host_agent
                        .host_state
                        .sync_artifacts_from_core(&host_agent.artifacts, entry_ids);
                }
            }
            Ok((transition.events, transition.actions, transition.markers))
        }
        other => {
            let actual = runtime_phase_name(&other);
            host_agent.runtime = other;
            Err(HostError::WrongPhase {
                expected: "Compacting",
                actual,
            })
        }
    };

    match result {
        Ok((events, actions, markers)) => {
            let directives = if actions.is_empty() {
                vec![HostDirective::Persist]
            } else {
                try_conv!(convert_actions_to_directives(actions))
            };
            let dto_events = try_conv!(convert_events(events));
            let dto_markers = try_conv!(markers
                .into_iter()
                .map(|m| ChangeMarkerDto::try_from(m))
                .collect::<Result<Vec<_>, _>>());
            put_host_agent(host_agent);
            ok(TurnResultOutput {
                events: dto_events,
                directives,
                markers: dto_markers,
            })
        }
        Err(e) => {
            put_host_agent(host_agent);
            err(&e)
        }
    }
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
            markers: vec![],
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

    let budget = host_agent.budget.clone();
    let compaction_prompt = host_agent.host_state.compaction_prompt.clone();

    let result = match host_agent.runtime {
        AgentRuntime::ReadyToContinue(ready) => {
            let transition = ready.continue_turn(
                host_agent.transcript,
                host_agent.artifacts,
                host_agent.turn_number,
                &budget,
                &compaction_prompt,
            );
            let (events, actions, runtime, transcript, artifacts, turn_number, markers) =
                transition.into_parts();
            host_agent.runtime = runtime;
            host_agent.transcript = transcript;
            host_agent.artifacts = artifacts;
            host_agent.turn_number = turn_number;
            for marker in &markers {
                if let pi_core::ChangeMarker::NewArtifacts { entry_ids } = marker {
                    host_agent
                        .host_state
                        .sync_artifacts_from_core(&host_agent.artifacts, entry_ids);
                }
            }
            Ok((events, actions, markers))
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
        Ok((events, actions, markers)) => {
            let directives = try_conv!(convert_actions_to_directives(actions));
            let dto_events = try_conv!(convert_events(events));
            let dto_markers = try_conv!(markers
                .into_iter()
                .map(|m| ChangeMarkerDto::try_from(m))
                .collect::<Result<Vec<_>, _>>());
            put_host_agent(host_agent);
            ok(TurnResultOutput {
                events: dto_events,
                directives,
                markers: dto_markers,
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
    let compaction_prompt = host_agent.host_state.compaction_prompt.clone();
    host_agent.runtime = host_agent.runtime.reset();
    host_agent.transcript = vec![];
    host_agent.artifacts = std::collections::BTreeMap::new();
    host_agent.turn_number = 0;
    host_agent.host_state = HostState::new(system_prompt, compaction_prompt);
    put_host_agent(host_agent);
    ok(())
}

#[wasm_bindgen(js_name = "hostToolCancelled")]
pub fn host_tool_cancelled(
    handle: u32,
    tool_call_id: String,
    reason: CancelReason,
) -> TurnResultResult {
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
            let (events, actions, new_runtime, transcript, artifacts, turn_number, markers) =
                waiting
                    .cancel_tool(
                        id,
                        core_reason,
                        host_agent.transcript,
                        host_agent.artifacts,
                        host_agent.turn_number,
                    )
                    .into_parts();
            host_agent.runtime = new_runtime;
            host_agent.transcript = transcript;
            host_agent.artifacts = artifacts;
            host_agent.turn_number = turn_number;
            for marker in &markers {
                if let pi_core::ChangeMarker::NewArtifacts { entry_ids } = marker {
                    host_agent
                        .host_state
                        .sync_artifacts_from_core(&host_agent.artifacts, entry_ids);
                }
            }
            Ok((events, actions, markers))
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
        Ok((events, actions, markers)) => {
            let directives = try_conv!(convert_actions_to_directives(actions));
            let dto_events = try_conv!(convert_events(events));
            let dto_markers = try_conv!(markers
                .into_iter()
                .map(|m| ChangeMarkerDto::try_from(m))
                .collect::<Result<Vec<_>, _>>());
            put_host_agent(host_agent);
            ok(TurnResultOutput {
                events: dto_events,
                directives,
                markers: dto_markers,
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
            let transition = streaming.abort(
                host_agent.transcript,
                host_agent.artifacts,
                host_agent.turn_number,
            );
            host_agent.runtime = transition.state.into_runtime();
            host_agent.transcript = transition.transcript;
            host_agent.artifacts = transition.artifacts;
            host_agent.turn_number = transition.turn_number;
            Ok((transition.events, transition.actions))
        }
        AgentRuntime::WaitingTools(waiting) => {
            let transition = waiting.abort(
                host_agent.transcript,
                host_agent.artifacts,
                host_agent.turn_number,
            );
            host_agent.runtime = transition.state.into_runtime();
            host_agent.transcript = transition.transcript;
            host_agent.artifacts = transition.artifacts;
            host_agent.turn_number = transition.turn_number;
            Ok((transition.events, transition.actions))
        }
        AgentRuntime::ReadyToContinue(ready) => {
            let transition = ready.abort(
                host_agent.transcript,
                host_agent.artifacts,
                host_agent.turn_number,
            );
            host_agent.runtime = transition.state.into_runtime();
            host_agent.transcript = transition.transcript;
            host_agent.artifacts = transition.artifacts;
            host_agent.turn_number = transition.turn_number;
            Ok((transition.events, transition.actions))
        }
        AgentRuntime::Compacting(compacting) => {
            let transition = compacting.abort(
                host_agent.transcript,
                host_agent.artifacts,
                host_agent.turn_number,
            );
            host_agent.runtime = transition.state.into_runtime();
            host_agent.transcript = transition.transcript;
            host_agent.artifacts = transition.artifacts;
            host_agent.turn_number = transition.turn_number;
            Ok((transition.events, transition.actions))
        }
        other => {
            let actual = runtime_phase_name(&other);
            host_agent.runtime = other;
            Err(HostError::WrongPhase {
                expected: "Streaming, WaitingTools, ReadyToContinue, or Compacting",
                actual,
            })
        }
    };

    match result {
        Ok((events, actions)) => {
            let directives = try_conv!(convert_actions_to_directives(actions));
            let dto_events = try_conv!(convert_events(events));
            put_host_agent(host_agent);
            ok(TurnResultOutput {
                events: dto_events,
                directives,
                markers: vec![],
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
    let result = with_host_agent(handle, |host_agent| {
        host_agent.host_state.get_persist_data(
            &host_agent.transcript,
            &host_agent.artifacts,
            host_agent.turn_number,
            &host_agent.budget,
        )
    });
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
    let mut host_state = HostState::restore(core_data.clone());
    host_state.sync_missing_artifacts_from_core(&core_data.artifacts);
    let agent = HostAgent {
        runtime,
        host_state,
        transcript: core_data.transcript,
        artifacts: core_data.artifacts,
        turn_number: core_data.turn_number,
        budget: core_data.budget.clone(),
    };
    let handle = put_host_agent(agent);
    info!(handle, "host agent restored");
    ok(CreateHostAgentOutput { handle })
}
