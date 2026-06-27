use super::*;

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
        AgentRuntime::PreToolCall(mut pre) => {
            let (events, actions) = pre.submit_user_message(core_prompt).into_events_actions();
            host_agent.runtime = pre.into_runtime();
            Ok((events, actions, vec![]))
        }
        AgentRuntime::ExecutingTools(mut exec) => {
            let (events, actions) = exec.submit_user_message(core_prompt).into_events_actions();
            host_agent.runtime = exec.into_runtime();
            Ok((events, actions, vec![]))
        }
        other => {
            let actual = runtime_phase_name(&other);
            host_agent.runtime = other;
            Err(HostError::WrongPhase {
                expected: "Idle, Finished, Aborted, PreToolCall, or ExecutingTools",
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
                .map(ChangeMarkerDto::try_from)
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
                .map(ChangeMarkerDto::try_from)
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

#[wasm_bindgen(js_name = "hostPrepareToolCalls")]
pub fn host_prepare_tool_calls(handle: u32, preparations_json: String) -> TurnResultResult {
    console_error_panic_hook::set_once();
    info!(handle, "hostPrepareToolCalls called");

    let dto_preps: Vec<ToolCallPreparation> = try_conv!(serde_json::from_str(&preparations_json));
    let core_preps: Vec<pi_core::ToolCallPreparation> = try_conv!(dto_preps
        .into_iter()
        .map(|p| p.try_into())
        .collect::<Result<Vec<_>, _>>());

    let mut host_agent = match take_host_agent(handle) {
        Ok(a) => a,
        Err(e) => return err(&e),
    };

    let result = match host_agent.runtime {
        AgentRuntime::PreToolCall(pre) => {
            let (events, actions, new_runtime, transcript, artifacts, turn_number, markers) = pre
                .prepare_tool_calls(
                    core_preps,
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
                expected: "PreToolCall",
                actual,
            })
        }
    };

    match result {
        Ok((events, actions, markers)) => {
            let mut directives = try_conv!(convert_actions_to_directives(actions));
            if matches!(host_agent.runtime, AgentRuntime::ReadyToContinue(_)) {
                directives.push(HostDirective::WaitForInput {
                    mode: WaitMode::Any,
                });
            }
            let dto_events = try_conv!(convert_events(events));
            let dto_markers = try_conv!(markers
                .into_iter()
                .map(ChangeMarkerDto::try_from)
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
        AgentRuntime::ExecutingTools(exec) => {
            let (events, actions, new_runtime, transcript, artifacts, turn_number, markers) = exec
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
                expected: "ExecutingTools",
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
                .map(ChangeMarkerDto::try_from)
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

#[wasm_bindgen(js_name = "hostToolFailed")]
pub fn host_tool_failed(handle: u32, id: ToolCallId, error: ToolError) -> TurnResultResult {
    console_error_panic_hook::set_once();
    info!(handle, "hostToolFailed called");

    let core_id: pi_core::ToolCallId = try_conv!(id.try_into());
    let core_error: pi_core::ToolError = try_conv!(error.try_into());

    let mut host_agent = match take_host_agent(handle) {
        Ok(a) => a,
        Err(e) => return err(&e),
    };

    let result = match host_agent.runtime {
        AgentRuntime::ExecutingTools(exec) => {
            let (events, actions, new_runtime, transcript, artifacts, turn_number, markers) = exec
                .on_tool_done(
                    core_id,
                    Err(core_error),
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
                expected: "ExecutingTools",
                actual,
            })
        }
    };

    match result {
        Ok((events, actions, markers)) => {
            let mut directives = try_conv!(convert_actions_to_directives(actions));
            if matches!(host_agent.runtime, AgentRuntime::ReadyToContinue(_)) {
                directives.push(HostDirective::WaitForInput {
                    mode: WaitMode::Any,
                });
            }
            let dto_events = try_conv!(convert_events(events));
            let dto_markers = try_conv!(markers
                .into_iter()
                .map(ChangeMarkerDto::try_from)
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
                .map(ChangeMarkerDto::try_from)
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

    // Steering is a pure queue op (pi-core AgentRuntime::steer pushes to
    // steering_queue unconditionally). It never interrupts an in-flight LLM
    // stream or tool batch; the message drains into the transcript at the
    // next continue_turn. Accept it in any phase so a host can queue
    // environmental input (e.g. a navigation event) mid-turn.
    let events = host_agent.runtime.steer(core_message);

    put_host_agent(host_agent);
    ok(TurnResultOutput {
        events: try_conv!(convert_events(events)),
        directives: vec![],
        markers: vec![],
    })
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
                .map(ChangeMarkerDto::try_from)
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
        AgentRuntime::PreToolCall(pre) => {
            let (events, actions, new_runtime, transcript, artifacts, turn_number, markers) = pre
                .cancel_tool(
                    id,
                    core_reason.clone(),
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
        AgentRuntime::ExecutingTools(exec) => {
            let (events, actions, new_runtime, transcript, artifacts, turn_number, markers) = exec
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
                expected: "PreToolCall or ExecutingTools",
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
                .map(ChangeMarkerDto::try_from)
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
        AgentRuntime::PreToolCall(pre) => {
            let transition = pre.abort(
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
        AgentRuntime::ExecutingTools(exec) => {
            let transition = exec.abort(
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
                expected: "Streaming, PreToolCall, ExecutingTools, ReadyToContinue, or Compacting",
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
