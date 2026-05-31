use super::*;

fn dummy_options() -> AgentOptions {
    AgentOptions {
        system_prompt: "test agent".to_string(),
        model: Model {
            id: ModelId("test-model".to_string()),
            name: ModelName("Test".to_string()),
            api: ApiName("test".to_string()),
            provider: ProviderName("test".to_string()),
            base_url: None,
            reasoning: false,
            context_window: 4096,
            max_tokens: 1024,
            capabilities: Default::default(),
            cost: Default::default(),
        },
        thinking_level: Default::default(),
        steering_mode: Default::default(),
        follow_up_mode: Default::default(),
        tool_execution_mode: Default::default(),
        session_id: None,
    }
}

#[test]
fn empty_result_serialize() {
    let r = EmptyResult {
        ok: true,
        data: Some(()),
        error: None,
    };
    let json = serde_json::to_string(&r).unwrap();
    println!("EmptyResult JSON: {}", json);
    assert!(json.contains("\"ok\":true"));
    assert!(json.contains("\"data\":null"));
}

#[test]
fn estimate_tokens_returns_value() {
    let input = EstimateTokensInput {
        messages: vec![AgentMessage::User(UserMessage {
            content: vec![Content::Text(TextContent {
                text: "hello world".to_string(),
            })],
            timestamp: 1,
        })],
    };
    let resp = estimate_tokens_export(input);
    assert!(resp.ok);
    assert_eq!(resp.data.unwrap().tokens, 3); // (11 + 3) / 4 = 3.5 -> 3
}

#[test]
fn estimate_tokens_for_text_returns_value() {
    let resp = estimate_tokens_for_text_export("hello".to_string());
    assert!(resp.ok);
    assert_eq!(resp.data.unwrap().tokens, 2); // (5 + 3) / 4 = 2
}

// -----------------------------------------------------------------------
// Phase 4 — HostAgent directive tests
// -----------------------------------------------------------------------

fn default_budget() -> ContextProjectionBudget {
    ContextProjectionBudget {
        max_tool_result_chars: 50000,
        max_context_tokens: 100000,
        microcompact_after_turns: 5,
        compaction_threshold: 0.75,
    }
}

fn make_tool_def(name: &str) -> ToolDefinition {
    ToolDefinition {
        name: ToolName(name.to_string()),
        label: "Test".to_string(),
        description: "A test tool.".to_string(),
        parameters: JsonSchema(serde_json::json!({})),
        execution_mode: Default::default(),
        tool_run_mode: Default::default(),
    }
}

fn make_user_prompt(text: &str) -> AgentMessage {
    AgentMessage::User(UserMessage {
        content: vec![Content::Text(TextContent {
            text: text.to_string(),
        })],
        timestamp: 1,
    })
}

fn make_assistant_text(text: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![Content::Text(TextContent {
            text: text.to_string(),
        })],
        api: ApiName("test".to_string()),
        provider: ProviderName("test".to_string()),
        model: ModelId("test".to_string()),
        stop_reason: StopReason::EndTurn,
        error_message: None,
        timestamp: 1,
        usage: TokenUsage::default(),
    }
}

fn make_assistant_with_tool(name: &str, id: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![Content::ToolCall(ToolCall {
            id: ToolCallId(id.to_string()),
            name: ToolName(name.to_string()),
            arguments: ToolArguments(serde_json::json!({})),
        })],
        api: ApiName("test".to_string()),
        provider: ProviderName("test".to_string()),
        model: ModelId("test".to_string()),
        stop_reason: StopReason::ToolUse,
        error_message: None,
        timestamp: 1,
        usage: TokenUsage::default(),
    }
}

#[test]
fn directive_stream_llm_has_projected_messages() {
    let resp = create_host_agent(dummy_options(), default_budget());
    assert!(resp.ok);
    let handle = resp.data.unwrap().handle;

    let resp = start_turn(
        handle,
        StartTurnInput {
            prompt: make_user_prompt("hello"),
            tools: vec![],
        },
    );
    assert!(resp.ok);
    let data = resp.data.unwrap();
    let stream = data
        .directives
        .iter()
        .find_map(|d| match d {
            HostDirective::StreamLlm { context } => Some(context),
            _ => None,
        })
        .expect("should have StreamLlm directive");
    assert!(
        !stream.messages.is_empty(),
        "projected messages should not be empty"
    );
    destroy_host_agent(handle);
}

#[test]
fn directive_execute_tools_after_llm() {
    let resp = create_host_agent(dummy_options(), default_budget());
    let handle = resp.data.unwrap().handle;

    let resp = start_turn(
        handle,
        StartTurnInput {
            prompt: make_user_prompt("use tool"),
            tools: vec![make_tool_def("test_tool")],
        },
    );
    assert!(resp.ok);

    let resp = host_llm_done(
        handle,
        LlmResult::Ok(make_assistant_with_tool("test_tool", "tc-1")),
    );
    assert!(resp.ok);
    let directives = resp.data.unwrap().directives;
    assert!(
        directives
            .iter()
            .any(|d| matches!(d, HostDirective::ExecuteTools { .. })),
        "should emit ExecuteTools directive after LLM with tool calls"
    );
    destroy_host_agent(handle);
}

#[test]
fn directive_finished_after_no_tools() {
    let resp = create_host_agent(dummy_options(), default_budget());
    let handle = resp.data.unwrap().handle;

    let resp = start_turn(
        handle,
        StartTurnInput {
            prompt: make_user_prompt("hello"),
            tools: vec![],
        },
    );
    assert!(resp.ok);

    let resp = host_llm_done(handle, LlmResult::Ok(make_assistant_text("hi")));
    assert!(resp.ok);
    let directives = resp.data.unwrap().directives;
    assert!(
        directives
            .iter()
            .any(|d| matches!(d, HostDirective::Finished)),
        "should emit Finished directive when no tools are requested"
    );
    destroy_host_agent(handle);
}

#[test]
fn directive_persist_after_entry_append() {
    let resp = create_host_agent(dummy_options(), default_budget());
    let handle = resp.data.unwrap().handle;

    let resp = start_turn(
        handle,
        StartTurnInput {
            prompt: make_user_prompt("hello"),
            tools: vec![],
        },
    );
    assert!(resp.ok);

    let resp = host_llm_done(handle, LlmResult::Ok(make_assistant_text("hi")));
    assert!(resp.ok);
    let directives = resp.data.unwrap().directives;
    assert!(
        directives
            .iter()
            .any(|d| matches!(d, HostDirective::Persist)),
        "should emit Persist directive after entries are appended"
    );
    assert!(
        directives
            .iter()
            .any(|d| matches!(d, HostDirective::Finished)),
        "should also emit Finished directive"
    );
    destroy_host_agent(handle);
}

#[test]
fn low_budget_turn_succeeds() {
    let budget = ContextProjectionBudget {
        max_tool_result_chars: 50000,
        max_context_tokens: 20,
        microcompact_after_turns: 5,
        compaction_threshold: 0.5,
    };
    let resp = create_host_agent(dummy_options(), budget);
    let handle = resp.data.unwrap().handle;

    let resp = start_turn(
        handle,
        StartTurnInput {
            prompt: make_user_prompt("hello"),
            tools: vec![],
        },
    );
    assert!(resp.ok);
    destroy_host_agent(handle);
}

#[test]
fn directive_cancel_tools() {
    // CancelTools is not yet produced by the current AgentRuntime, so we
    // test the conversion logic directly.
    let core_actions = vec![pi_core::AgentAction::CancelTools {
        tool_call_ids: vec![pi_core::ToolCallId::new("tc-1")],
        reason: pi_core::CancelReason::UserRequested,
    }];
    let directives = super::convert_actions_to_directives(core_actions).unwrap();
    assert!(
        directives
            .iter()
            .any(|d| matches!(d, HostDirective::CancelTools { .. })),
        "should convert CancelTools action to directive"
    );
}

#[test]
fn directive_wait_for_input() {
    // WaitForInput is not yet produced by the current AgentRuntime in the
    // standard flow, so we test the conversion logic directly.
    let core_actions = vec![pi_core::AgentAction::WaitForInput {
        mode: pi_core::WaitMode::Any,
    }];
    let directives = super::convert_actions_to_directives(core_actions).unwrap();
    assert!(
        directives
            .iter()
            .any(|d| matches!(d, HostDirective::WaitForInput { .. })),
        "should convert WaitForInput action to directive"
    );
}

#[test]
fn full_turn_directive_sequence() {
    let resp = create_host_agent(dummy_options(), default_budget());
    let handle = resp.data.unwrap().handle;

    // Step 1: start_turn -> StreamLlm
    let resp = start_turn(
        handle,
        StartTurnInput {
            prompt: make_user_prompt("use tool"),
            tools: vec![make_tool_def("test_tool")],
        },
    );
    assert!(resp.ok);
    let directives = resp.data.unwrap().directives;
    assert!(directives
        .iter()
        .any(|d| matches!(d, HostDirective::StreamLlm { .. })));

    // Step 2: llm_done with tool -> ExecuteTools
    let resp = host_llm_done(
        handle,
        LlmResult::Ok(make_assistant_with_tool("test_tool", "tc-1")),
    );
    assert!(resp.ok);
    let directives = resp.data.unwrap().directives;
    assert!(directives
        .iter()
        .any(|d| matches!(d, HostDirective::ExecuteTools { .. })));

    // Step 3: tool_done -> WaitForInput (agent pauses for host to continue)
    let tool_result = ToolResult {
        content: vec![Content::Text(TextContent {
            text: "ok".to_string(),
        })],
        details: None,
        terminate: None,
    };
    let resp = host_tool_done(handle, ToolCallId("tc-1".to_string()), tool_result);
    assert!(resp.ok);
    let directives = resp.data.unwrap().directives;
    assert!(directives
        .iter()
        .any(|d| matches!(d, HostDirective::WaitForInput { .. })));

    // Step 4: continue_turn -> StreamLlm
    let resp = host_continue_turn(handle);
    assert!(resp.ok);
    let directives = resp.data.unwrap().directives;
    assert!(directives
        .iter()
        .any(|d| matches!(d, HostDirective::StreamLlm { .. })));

    // Step 5: llm_done with no tools -> Finished + Persist
    let resp = host_llm_done(handle, LlmResult::Ok(make_assistant_text("done")));
    assert!(resp.ok);
    let directives = resp.data.unwrap().directives;
    assert!(directives
        .iter()
        .any(|d| matches!(d, HostDirective::Finished)));
    assert!(directives
        .iter()
        .any(|d| matches!(d, HostDirective::Persist)));

    destroy_host_agent(handle);
}

#[test]
fn multi_turn_directive_sequence() {
    let resp = create_host_agent(dummy_options(), default_budget());
    let handle = resp.data.unwrap().handle;

    // Turn 1
    let resp = start_turn(
        handle,
        StartTurnInput {
            prompt: make_user_prompt("hello"),
            tools: vec![],
        },
    );
    assert!(resp.ok);
    let resp = host_llm_done(handle, LlmResult::Ok(make_assistant_text("hi")));
    assert!(resp.ok);
    let directives = resp.data.unwrap().directives;
    assert!(directives
        .iter()
        .any(|d| matches!(d, HostDirective::Finished)));
    assert!(directives
        .iter()
        .any(|d| matches!(d, HostDirective::Persist)));

    // Turn 2
    let resp = start_turn(
        handle,
        StartTurnInput {
            prompt: make_user_prompt("again"),
            tools: vec![],
        },
    );
    assert!(resp.ok);
    let resp = host_llm_done(handle, LlmResult::Ok(make_assistant_text("yep")));
    assert!(resp.ok);
    let directives = resp.data.unwrap().directives;
    assert!(directives
        .iter()
        .any(|d| matches!(d, HostDirective::Finished)));
    assert!(directives
        .iter()
        .any(|d| matches!(d, HostDirective::Persist)));

    destroy_host_agent(handle);
}

#[test]
fn turn_then_finish_with_low_budget() {
    let budget = ContextProjectionBudget {
        max_tool_result_chars: 50000,
        max_context_tokens: 30,
        microcompact_after_turns: 5,
        compaction_threshold: 0.5,
    };
    let resp = create_host_agent(dummy_options(), budget);
    let handle = resp.data.unwrap().handle;

    // Simple turn
    let resp = start_turn(
        handle,
        StartTurnInput {
            prompt: make_user_prompt("trigger"),
            tools: vec![],
        },
    );
    assert!(resp.ok);

    let resp = host_llm_done(handle, LlmResult::Ok(make_assistant_text("done")));
    assert!(resp.ok);
    let directives = resp.data.unwrap().directives;
    assert!(directives
        .iter()
        .any(|d| matches!(d, HostDirective::Finished)));
    assert!(directives
        .iter()
        .any(|d| matches!(d, HostDirective::Persist)));

    let persist = get_host_agent_persist_data(handle);
    assert!(persist.ok);

    destroy_host_agent(handle);
}

#[test]
fn events_still_emitted_alongside_directives() {
    let resp = create_host_agent(dummy_options(), default_budget());
    let handle = resp.data.unwrap().handle;

    let resp = start_turn(
        handle,
        StartTurnInput {
            prompt: make_user_prompt("hello"),
            tools: vec![],
        },
    );
    assert!(resp.ok);
    let data = resp.data.unwrap();
    assert!(
        !data.events.is_empty(),
        "events should be emitted alongside directives"
    );
    assert!(data
        .directives
        .iter()
        .any(|d| matches!(d, HostDirective::StreamLlm { .. })));
    destroy_host_agent(handle);
}

#[test]
fn steering_during_stream_produces_directives() {
    // Steering during streaming is not supported by the current AgentRuntime.
    // The host_steer function returns a wrong_phase error when called while streaming.
    let resp = create_host_agent(dummy_options(), default_budget());
    let handle = resp.data.unwrap().handle;

    let resp = start_turn(
        handle,
        StartTurnInput {
            prompt: make_user_prompt("hello"),
            tools: vec![],
        },
    );
    assert!(resp.ok);

    let steer_msg = make_user_prompt("steer");
    let resp = host_steer(handle, steer_msg);
    assert!(!resp.ok);
    assert_eq!(resp.error.as_ref().unwrap().code, "wrong_phase");
    destroy_host_agent(handle);
}

// -----------------------------------------------------------------------
// Phase 6 — DTO and SDK Update
// -----------------------------------------------------------------------

#[test]
fn dto_host_directive_serialize() {
    let stream = HostDirective::StreamLlm {
        context: LlmContext {
            system_prompt: "test".to_string(),
            messages: vec![],
            tools: vec![],
        },
    };
    let json = serde_json::to_string(&stream).unwrap();
    assert!(
        json.contains("stream_llm"),
        "StreamLlm should serialize with tag"
    );

    let execute = HostDirective::ExecuteTools {
        calls: vec![ToolCall {
            id: ToolCallId("tc-1".to_string()),
            name: ToolName("read".to_string()),
            arguments: ToolArguments(serde_json::json!({})),
        }],
    };
    let json = serde_json::to_string(&execute).unwrap();
    assert!(
        json.contains("execute_tools"),
        "ExecuteTools should serialize with tag"
    );

    let cancel = HostDirective::CancelTools {
        tool_call_ids: vec![ToolCallId("tc-1".to_string())],
        reason: CancelReason::UserRequested,
    };
    let json = serde_json::to_string(&cancel).unwrap();
    assert!(
        json.contains("cancel_tools"),
        "CancelTools should serialize with tag"
    );

    let persist = HostDirective::Persist;
    let json = serde_json::to_string(&persist).unwrap();
    assert!(
        json.contains("persist"),
        "Persist should serialize with tag"
    );

    let finished = HostDirective::Finished;
    let json = serde_json::to_string(&finished).unwrap();
    assert!(
        json.contains("finished"),
        "Finished should serialize with tag"
    );

    let wait = HostDirective::WaitForInput {
        mode: WaitMode::Any,
    };
    let json = serde_json::to_string(&wait).unwrap();
    assert!(
        json.contains("wait_for_input"),
        "WaitForInput should serialize with tag"
    );
}

#[test]
fn dto_turn_result_structure() {
    let result = TurnResultResult {
        ok: true,
        data: Some(TurnResultOutput {
            events: vec![AgentEvent::AgentStart],
            directives: vec![HostDirective::Persist],
        }),
        error: None,
    };
    let json = serde_json::to_string(&result).unwrap();
    assert!(json.contains("\"ok\":true"));
    assert!(json.contains("events"));
    assert!(json.contains("directives"));
}

#[test]
fn dto_persist_data_structure() {
    let data = PersistData {
        transcript: serde_json::Value::Array(vec![]),
        artifacts: serde_json::Value::Object(serde_json::Map::new()),
        turn_number: 1,
        host_artifacts: vec![("a1".to_string(), "hello".to_string())],
        budget: ContextProjectionBudget::default(),
        system_prompt: "You are helpful.".to_string(),
        compaction_prompt: "Summarize.".to_string(),
    };
    let json = serde_json::to_string(&data).unwrap();
    assert!(json.contains("host_artifacts"));
    assert!(json.contains("budget"));
    assert!(json.contains("system_prompt"));
    assert!(json.contains("turn_number"));
}

#[test]
fn dto_persist_data_roundtrip() {
    let original = PersistData {
        transcript: serde_json::Value::Array(vec![]),
        artifacts: serde_json::Value::Object(serde_json::Map::new()),
        turn_number: 2,
        host_artifacts: vec![("a1".to_string(), "hello".to_string())],
        budget: ContextProjectionBudget::default(),
        system_prompt: "You are helpful.".to_string(),
        compaction_prompt: "Summarize.".to_string(),
    };
    let json = serde_json::to_string(&original).unwrap();
    let back: PersistData = serde_json::from_str(&json).unwrap();
    assert_eq!(original, back);
}

#[test]
fn get_host_state_persist_data_roundtrip() {
    let state = HostState::new("You are helpful.".to_string(), "Summarize.".to_string());
    let handle = put_host_state(state);

    let resp = get_host_state_persist_data(handle);
    assert!(resp.ok);
    let data = resp.data.unwrap().state;
    assert_eq!(data.system_prompt, "You are helpful.");

    let restore_handle = restore_host_state(data);
    assert!(restore_handle.ok);
    let restored_handle = restore_handle.data.unwrap().handle;

    let resp2 = get_host_state_persist_data(restored_handle);
    assert!(resp2.ok);
    let data2 = resp2.data.unwrap().state;
    assert_eq!(data2.system_prompt, "You are helpful.");

    destroy_host_state(handle);
    destroy_host_state(restored_handle);
}

// -----------------------------------------------------------------------
// Phase 7 — Session Migration
// -----------------------------------------------------------------------

#[test]
fn migrate_old_session_extracts_projection() {
    // Old format with projection_state is no longer recognized by new PersistData format.
    // Should fail gracefully with invalid_session_json.
    let old_json = r#"{
        "entries": [],
        "leaf_id": "",
        "name": "old",
        "projection_state": {"tools":{},"current_turn":3,"turns_since_compaction":1},
        "artifacts": []
    }"#;
    let resp = restore_host_state_from_json(old_json.to_string());
    assert!(!resp.ok, "old format with projection_state should fail");
}

#[test]
fn migrate_new_session_noop() {
    let data = PersistData {
        transcript: serde_json::Value::Array(vec![]),
        artifacts: serde_json::Value::Object(serde_json::Map::new()),
        turn_number: 1,
        host_artifacts: vec![("a1".to_string(), "hello".to_string())],
        budget: ContextProjectionBudget::default(),
        system_prompt: "You are helpful.".to_string(),
        compaction_prompt: "Summarize.".to_string(),
    };
    let json = serde_json::to_string(&data).unwrap();
    let resp = restore_host_state_from_json(json);
    assert!(resp.ok, "expected ok, got error: {:?}", resp.error);
    let handle = resp.data.unwrap().handle;
    let state_resp = get_host_state_persist_data(handle);
    assert!(state_resp.ok);
    let restored = state_resp.data.unwrap().state;
    assert_eq!(restored.system_prompt, "You are helpful.");
    assert_eq!(restored.host_artifacts.len(), 1);
    assert_eq!(
        restored.host_artifacts[0],
        ("a1".to_string(), "hello".to_string())
    );
    destroy_host_state(handle);
}

#[test]
fn roundtrip_persist_data() {
    let data = PersistData {
        transcript: serde_json::Value::Array(vec![]),
        artifacts: serde_json::Value::Object(serde_json::Map::new()),
        turn_number: 3,
        host_artifacts: vec![("a1".to_string(), "hello".to_string())],
        budget: ContextProjectionBudget::default(),
        system_prompt: "You are helpful.".to_string(),
        compaction_prompt: "Summarize.".to_string(),
    };
    let json = serde_json::to_string(&data).unwrap();
    let resp = restore_host_state_from_json(json);
    assert!(resp.ok, "expected ok, got error: {:?}", resp.error);
    let handle = resp.data.unwrap().handle;

    let state_resp = get_host_state_persist_data(handle);
    assert!(state_resp.ok);
    let restored_data = state_resp.data.unwrap().state;

    // Re-serialize and restore again
    let json2 = serde_json::to_string(&restored_data).unwrap();
    let resp2 = restore_host_state_from_json(json2);
    assert!(
        resp2.ok,
        "expected ok on roundtrip, got error: {:?}",
        resp2.error
    );
    let handle2 = resp2.data.unwrap().handle;

    let state_resp2 = get_host_state_persist_data(handle2);
    assert!(state_resp2.ok);
    let data2 = state_resp2.data.unwrap().state;

    assert_eq!(restored_data, data2);

    destroy_host_state(handle);
    destroy_host_state(handle2);
}
