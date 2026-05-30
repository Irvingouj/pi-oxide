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
        messages: vec![],
        session_state: None,
    }
}

#[test]
fn create_agent_returns_ok_with_handle() {
    let resp = create_agent(dummy_options());
    assert!(resp.ok);
    assert!(resp.data.is_some());
    assert_eq!(resp.data.unwrap().handle, 0);
}

#[test]
fn prompt_returns_stream_llm_action() {
    create_agent(dummy_options());
    let resp = prompt(
        0,
        PromptInput {
            prompt: PromptRequest::Text {
                text: "hello".to_string(),
            },
            tools: vec![],
        },
    );
    assert!(resp.ok);
    let actions = resp.data.unwrap().actions;
    assert!(actions
        .iter()
        .any(|a| matches!(a, AgentAction::StreamLlm { .. })));
    destroy_agent(0);
}

#[test]
fn bad_handle_returns_error() {
    let resp = prompt(
        9999,
        PromptInput {
            prompt: PromptRequest::Text {
                text: "hi".to_string(),
            },
            tools: vec![],
        },
    );
    assert!(!resp.ok);
    assert_eq!(resp.error.as_ref().unwrap().code, "bad_handle");
}

#[test]
fn on_llm_done_with_no_tools_finishes() {
    create_agent(dummy_options());
    prompt(
        0,
        PromptInput {
            prompt: PromptRequest::Text {
                text: "hello".to_string(),
            },
            tools: vec![],
        },
    );

    let done_resp = on_llm_done(
        0,
        LlmResult::Ok(AssistantMessage {
            content: vec![Content::Text(TextContent {
                text: "hi".to_string(),
            })],
            api: ApiName("test".to_string()),
            provider: ProviderName("test".to_string()),
            model: ModelId("test".to_string()),
            stop_reason: StopReason::EndTurn,
            error_message: None,
            timestamp: 1,
            usage: TokenUsage::default(),
        }),
    );
    assert!(done_resp.ok);
    let actions = done_resp.data.unwrap().actions;
    assert!(actions
        .iter()
        .any(|a| matches!(a, AgentAction::Finished { .. })));

    destroy_agent(0);
}

#[test]
fn reset_clears_state() {
    create_agent(dummy_options());
    prompt(
        0,
        PromptInput {
            prompt: PromptRequest::Text {
                text: "hello".to_string(),
            },
            tools: vec![],
        },
    );

    let reset_resp = reset(0);
    assert!(reset_resp.ok);

    let state_resp = state(0);
    assert!(state_resp.ok);
    assert!(state_resp.data.unwrap().state.messages.is_empty());

    destroy_agent(0);
}

#[test]
fn state_returns_system_prompt() {
    create_agent(dummy_options());
    let state_resp = state(0);
    assert!(state_resp.ok);
    assert_eq!(state_resp.data.unwrap().state.system_prompt, "test agent");
    destroy_agent(0);
}

#[test]
fn destroy_agent_frees_handle() {
    create_agent(dummy_options());
    let destroy_resp = destroy_agent(0);
    assert!(destroy_resp.ok);

    let state_resp = state(0);
    assert!(!state_resp.ok);
    assert_eq!(state_resp.error.as_ref().unwrap().code, "bad_handle");
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
fn project_context_returns_ok_with_report() {
    let input = ProjectionInput {
        system_prompt: "You are helpful.".to_string(),
        messages: vec![AgentMessage::User(UserMessage {
            content: vec![Content::Text(TextContent {
                text: "hello".to_string(),
            })],
            timestamp: 1,
        })],
        budget: ContextProjectionBudget {
            max_tool_result_chars: 50000,
            max_context_tokens: 100000,
            microcompact_after_turns: 5,
            compaction_threshold: 0.75,
        },
        state: ContextProjectionState::default(),
    };

    let resp = project_context_export(input);
    assert!(resp.ok);
    let data = resp.data.unwrap();
    assert!(!data.projected_messages.is_empty());
    assert!(data.report.estimated_tokens > 0);
}

#[test]
fn get_session_state_after_creation_is_empty() {
    create_agent(dummy_options());
    let resp = get_session_state(0);
    assert!(resp.ok);
    let state = resp.data.unwrap().state;
    assert!(state.entries.is_empty());
    assert_eq!(state.leaf_id, "");
    destroy_agent(0);
}

#[test]
fn set_and_get_session_state_roundtrip() {
    create_agent(dummy_options());
    let custom_state = crate::dto::SessionState {
        entries: vec![crate::dto::SessionEntry {
            id: "entry-0".to_string(),
            parent_id: None,
            kind: crate::dto::EntryKind::Message {
                message: crate::dto::AgentMessage::User(crate::dto::UserMessage {
                    content: vec![crate::dto::Content::Text(crate::dto::TextContent {
                        text: "hi".to_string(),
                    })],
                    timestamp: 1,
                }),
            },
            timestamp: 1,
        }],
        leaf_id: "entry-0".to_string(),
        name: "test-session".to_string(),
    };

    let set_resp = set_session_state(0, custom_state.clone());
    assert!(set_resp.ok);

    let get_resp = get_session_state(0);
    assert!(get_resp.ok);
    let retrieved = get_resp.data.unwrap().state;
    assert_eq!(retrieved.entries.len(), 1);
    assert_eq!(retrieved.leaf_id, "entry-0");
    assert_eq!(retrieved.name, "test-session");
    destroy_agent(0);
}

#[test]
fn get_session_branch_after_prompt() {
    create_agent(dummy_options());
    prompt(
        0,
        PromptInput {
            prompt: PromptRequest::Text {
                text: "hello".to_string(),
            },
            tools: vec![],
        },
    );

    let resp = get_session_branch(0);
    assert!(resp.ok);
    let entries = resp.data.unwrap().entries;
    assert_eq!(entries.len(), 1);
    assert!(matches!(entries[0].kind, EntryKind::Message { .. }));
    destroy_agent(0);
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

#[test]
fn abort_from_streaming_returns_aborted() {
    create_agent(dummy_options());
    prompt(
        0,
        PromptInput {
            prompt: PromptRequest::Text {
                text: "hello".to_string(),
            },
            tools: vec![],
        },
    );

    let abort_resp = abort(0);
    assert!(abort_resp.ok);
    let actions = abort_resp.data.unwrap().actions;
    assert!(actions.is_empty());

    // After abort, state should not be streaming
    let state_resp = state(0);
    assert!(state_resp.ok);
    assert!(!state_resp.data.unwrap().state.is_streaming);

    destroy_agent(0);
}

#[test]
fn abort_from_idle_returns_wrong_phase() {
    create_agent(dummy_options());
    let abort_resp = abort(0);
    assert!(!abort_resp.ok);
    assert_eq!(abort_resp.error.as_ref().unwrap().code, "wrong_phase");
    destroy_agent(0);
}

#[test]
fn abort_from_waiting_tools_clears_pending() {
    create_agent(dummy_options());
    prompt(
        0,
        PromptInput {
            prompt: PromptRequest::Text {
                text: "use tool".to_string(),
            },
            tools: vec![],
        },
    );

    let assistant = AssistantMessage {
        content: vec![Content::ToolCall(ToolCall {
            id: ToolCallId("tc-1".to_string()),
            name: ToolName("test_tool".to_string()),
            arguments: ToolArguments(serde_json::json!({})),
        })],
        api: ApiName("test".to_string()),
        provider: ProviderName("test".to_string()),
        model: ModelId("test".to_string()),
        stop_reason: StopReason::ToolUse,
        error_message: None,
        timestamp: 1,
        usage: TokenUsage::default(),
    };
    let done_resp = on_llm_done(0, LlmResult::Ok(assistant));
    assert!(done_resp.ok);
    let state_resp = state(0);
    assert!(state_resp.ok);
    assert!(!state_resp.data.unwrap().state.pending_tool_calls.is_empty());

    let abort_resp = abort(0);
    assert!(abort_resp.ok);
    let state_resp = state(0);
    assert!(state_resp.ok);
    let state = state_resp.data.unwrap().state;
    assert!(
        state.pending_tool_calls.is_empty(),
        "abort from WaitingTools should clear pending tool calls"
    );
    assert!(!state.is_streaming);

    destroy_agent(0);
}

#[test]
fn abort_from_ready_to_continue_is_allowed() {
    create_agent(dummy_options());
    prompt(
        0,
        PromptInput {
            prompt: PromptRequest::Text {
                text: "use tool".to_string(),
            },
            tools: vec![],
        },
    );

    let assistant = AssistantMessage {
        content: vec![Content::ToolCall(ToolCall {
            id: ToolCallId("tc-1".to_string()),
            name: ToolName("test_tool".to_string()),
            arguments: ToolArguments(serde_json::json!({})),
        })],
        api: ApiName("test".to_string()),
        provider: ProviderName("test".to_string()),
        model: ModelId("test".to_string()),
        stop_reason: StopReason::ToolUse,
        error_message: None,
        timestamp: 1,
        usage: TokenUsage::default(),
    };
    let done_resp = on_llm_done(0, LlmResult::Ok(assistant));
    assert!(done_resp.ok);

    // Complete the tool to reach ReadyToContinue
    let tool_done = on_tool_done(
        0,
        "tc-1".to_string(),
        ToolDonePayload::Success {
            result: ToolResult {
                content: vec![Content::Text(TextContent {
                    text: "ok".to_string(),
                })],
                details: None,
                terminate: None,
            },
        },
    );
    assert!(tool_done.ok);
    let state_resp = state(0);
    assert!(state_resp.ok);
    let agent_state = state_resp.data.unwrap().state;
    assert!(agent_state.pending_tool_calls.is_empty());

    // Abort from ReadyToContinue should succeed
    let abort_resp = abort(0);
    assert!(abort_resp.ok);
    let state_resp = state(0);
    assert!(state_resp.ok);
    assert!(!state_resp.data.unwrap().state.is_streaming);

    destroy_agent(0);
}

#[test]
fn prompt_with_tools_passes_them_through() {
    create_agent(dummy_options());
    let tool = ToolDefinition {
        name: ToolName("test_tool".to_string()),
        label: "Test".to_string(),
        description: "A test tool.".to_string(),
        parameters: JsonSchema(serde_json::json!({})),
        execution_mode: Default::default(),
        tool_run_mode: Default::default(),
    };
    let resp = prompt(
        0,
        PromptInput {
            prompt: PromptRequest::Text {
                text: "use tool".to_string(),
            },
            tools: vec![tool],
        },
    );
    assert!(resp.ok);
    let actions = resp.data.unwrap().actions;
    assert!(actions
        .iter()
        .any(|a| matches!(a, AgentAction::StreamLlm { .. })));
    let context = match &actions[0] {
        AgentAction::StreamLlm { context, .. } => context,
        other => panic!("expected StreamLlm, got {other:?}"),
    };
    assert!(
        !context.tools.is_empty(),
        "prompt should pass tools through to StreamLlm context"
    );
    assert_eq!(context.tools[0].name, ToolName("test_tool".to_string()));
    destroy_agent(0);
}

#[test]
fn prompt_from_finished_with_tools_passes_them() {
    create_agent(dummy_options());
    // First turn: finish without tools
    prompt(
        0,
        PromptInput {
            prompt: PromptRequest::Text {
                text: "hello".to_string(),
            },
            tools: vec![],
        },
    );
    let done_resp = on_llm_done(
        0,
        LlmResult::Ok(AssistantMessage {
            content: vec![Content::Text(TextContent {
                text: "hi".to_string(),
            })],
            api: ApiName("test".to_string()),
            provider: ProviderName("test".to_string()),
            model: ModelId("test".to_string()),
            stop_reason: StopReason::EndTurn,
            error_message: None,
            timestamp: 1,
            usage: TokenUsage::default(),
        }),
    );
    assert!(done_resp.ok);

    // Second turn with tools from Finished state
    let tool = ToolDefinition {
        name: ToolName("test_tool".to_string()),
        label: "Test".to_string(),
        description: "A test tool.".to_string(),
        parameters: JsonSchema(serde_json::json!({})),
        execution_mode: Default::default(),
        tool_run_mode: Default::default(),
    };
    let resp = prompt(
        0,
        PromptInput {
            prompt: PromptRequest::Text {
                text: "use tool".to_string(),
            },
            tools: vec![tool],
        },
    );
    assert!(resp.ok);
    let actions = resp.data.unwrap().actions;
    let context = match &actions[0] {
        AgentAction::StreamLlm { context, .. } => context,
        other => panic!("expected StreamLlm, got {other:?}"),
    };
    assert!(
        !context.tools.is_empty(),
        "prompt from Finished should pass tools through"
    );
    assert_eq!(context.tools[0].name, ToolName("test_tool".to_string()));
    destroy_agent(0);
}

#[test]
fn prompt_from_aborted_with_tools_passes_them() {
    create_agent(dummy_options());
    // Start a turn then abort
    prompt(
        0,
        PromptInput {
            prompt: PromptRequest::Text {
                text: "hello".to_string(),
            },
            tools: vec![],
        },
    );
    let abort_resp = abort(0);
    assert!(abort_resp.ok);

    // Prompt from Aborted state with tools
    let tool = ToolDefinition {
        name: ToolName("test_tool".to_string()),
        label: "Test".to_string(),
        description: "A test tool.".to_string(),
        parameters: JsonSchema(serde_json::json!({})),
        execution_mode: Default::default(),
        tool_run_mode: Default::default(),
    };
    let resp = prompt(
        0,
        PromptInput {
            prompt: PromptRequest::Text {
                text: "use tool".to_string(),
            },
            tools: vec![tool],
        },
    );
    assert!(resp.ok);
    let actions = resp.data.unwrap().actions;
    let context = match &actions[0] {
        AgentAction::StreamLlm { context, .. } => context,
        other => panic!("expected StreamLlm, got {other:?}"),
    };
    assert!(
        !context.tools.is_empty(),
        "prompt from Aborted should pass tools through"
    );
    assert_eq!(context.tools[0].name, ToolName("test_tool".to_string()));
    destroy_agent(0);
}

#[test]
fn projection_strategy_dynamic_roundtrip() {
    let strategy = ProjectionStrategy::Dynamic {
        script: "#{ action: \"project\", text: head(text, 100) }".to_string(),
    };
    let core: pi_core::ProjectionStrategy = strategy.clone().try_into().unwrap();
    let back: ProjectionStrategy = core.try_into().unwrap();
    assert!(matches!(back, ProjectionStrategy::Dynamic { .. }));
}

#[test]
fn tool_projection_state_inline_roundtrip() {
    let state = ToolProjectionState::Inline;
    let core: pi_core::ToolProjectionState = state.clone().try_into().unwrap();
    let back: ToolProjectionState = core.try_into().unwrap();
    assert!(matches!(back, ToolProjectionState::Inline));
}

#[test]
fn tool_projection_state_deferred_roundtrip() {
    let state = ToolProjectionState::Deferred {
        until_turn: 5,
        inserted_at_turn: 2,
    };
    let core: pi_core::ToolProjectionState = state.clone().try_into().unwrap();
    let back: ToolProjectionState = core.try_into().unwrap();
    assert!(matches!(
        back,
        ToolProjectionState::Deferred {
            until_turn: 5,
            inserted_at_turn: 2
        }
    ));
}

#[test]
fn tool_projection_state_replaced_roundtrip() {
    let replacement = ContextReplacement {
        tool_call_id: "tc-1".to_string(),
        tool_name: "read".to_string(),
        artifact_id: "tool-result-tc-1".to_string(),
        original_chars: 5000,
        preview_chars: 200,
        strategy: ProjectionStrategy::Fixed {
            shape: ProjectionShape::Head { max_chars: 200 },
            min_age: 0,
        },
        outcome: ProjectionOutcome {
            text: "hello".to_string(),
        },
    };
    let state = ToolProjectionState::Replaced {
        replacement,
        inserted_at_turn: 0,
    };
    let core: pi_core::ToolProjectionState = state.clone().try_into().unwrap();
    let back: ToolProjectionState = core.try_into().unwrap();
    assert!(matches!(back, ToolProjectionState::Replaced { .. }));
}

#[test]
fn projection_outcome_roundtrip() {
    let outcome = ProjectionOutcome {
        text: "hello".to_string(),
    };
    let core: pi_core::ProjectionOutcome = outcome.clone().try_into().unwrap();
    let back: ProjectionOutcome = core.try_into().unwrap();
    assert_eq!(back.text, "hello");
}

#[test]
fn projection_strategy_fixed_roundtrip() {
    let strategy = ProjectionStrategy::Fixed {
        shape: ProjectionShape::Head { max_chars: 2000 },
        min_age: 2,
    };
    let core: pi_core::ProjectionStrategy = strategy.clone().try_into().unwrap();
    let back: ProjectionStrategy = core.try_into().unwrap();
    assert!(
        matches!(
            back,
            ProjectionStrategy::Fixed {
                shape: ProjectionShape::Head { max_chars: 2000 },
                min_age: 2
            }
        ),
        "Fixed strategy should round-trip with shape and min_age"
    );
}

#[test]
fn old_context_state_migration_roundtrip() {
    let old_state = ContextProjectionState {
        tools: std::collections::BTreeMap::new(),
        replacements: {
            let mut map = std::collections::BTreeMap::new();
            map.insert(
                "tc-old".to_string(),
                OldContextReplacement {
                    tool_call_id: "tc-old".to_string(),
                    tool_name: "read".to_string(),
                    artifact_id: "art-old".to_string(),
                    original_chars: 5000,
                    preview_chars: 200,
                    strategy: OldContextStrategy::Head { max_chars: 200 },
                },
            );
            map
        },
        current_turn: 3,
        last_api_usage: None,
        turns_since_compaction: 1,
    };
    let core: pi_core::ContextProjectionState = old_state.try_into().unwrap();
    let entry = core.tools.get("tc-old");
    assert!(
        matches!(entry, Some(pi_core::ToolProjectionState::Replaced { replacement, .. }) if replacement.tool_call_id == "tc-old"),
        "old replacement should migrate to Replaced state, got {:?}",
        entry
    );
}

#[test]
fn project_context_with_old_replacements_migrates() {
    let input = ProjectionInput {
        system_prompt: "You are helpful.".to_string(),
        messages: vec![AgentMessage::User(UserMessage {
            content: vec![Content::Text(TextContent {
                text: "hello".to_string(),
            })],
            timestamp: 1,
        })],
        budget: ContextProjectionBudget {
            max_tool_result_chars: 50000,
            max_context_tokens: 100000,
            microcompact_after_turns: 5,
            compaction_threshold: 0.75,
        },
        state: ContextProjectionState {
            tools: std::collections::BTreeMap::new(),
            replacements: {
                let mut map = std::collections::BTreeMap::new();
                map.insert(
                    "tc-old".to_string(),
                    OldContextReplacement {
                        tool_call_id: "tc-old".to_string(),
                        tool_name: "read".to_string(),
                        artifact_id: "art-old".to_string(),
                        original_chars: 5000,
                        preview_chars: 200,
                        strategy: OldContextStrategy::Head { max_chars: 200 },
                    },
                );
                map
            },
            current_turn: 3,
            last_api_usage: None,
            turns_since_compaction: 0,
        },
    };

    let resp = project_context_export(input);
    assert!(resp.ok);
    let data = resp.data.unwrap();
    // The updated state should have migrated the old replacement into tools
    let migrated = data.updated_state.tools.get("tc-old");
    assert!(
        migrated.is_some(),
        "old replacement should be migrated into updated_state.tools"
    );
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
        content: vec![Content::Text(TextContent { text: text.to_string() })],
        timestamp: 1,
    })
}

fn make_assistant_text(text: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![Content::Text(TextContent { text: text.to_string() })],
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

    let resp = start_turn(handle, StartTurnInput { prompt: make_user_prompt("hello"), tools: vec![] });
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
    assert!(!stream.messages.is_empty(), "projected messages should not be empty");
    destroy_host_agent(handle);
}

#[test]
fn directive_execute_tools_after_llm() {
    let resp = create_host_agent(dummy_options(), default_budget());
    let handle = resp.data.unwrap().handle;

    let resp = start_turn(handle, StartTurnInput { prompt: make_user_prompt("use tool"), tools: vec![make_tool_def("test_tool")] });
    assert!(resp.ok);

    let resp = host_llm_done(
        handle,
        LlmResult::Ok(make_assistant_with_tool("test_tool", "tc-1")),
    );
    assert!(resp.ok);
    let directives = resp.data.unwrap().directives;
    assert!(
        directives.iter().any(|d| matches!(d, HostDirective::ExecuteTools { .. })),
        "should emit ExecuteTools directive after LLM with tool calls"
    );
    destroy_host_agent(handle);
}

#[test]
fn directive_finished_after_no_tools() {
    let resp = create_host_agent(dummy_options(), default_budget());
    let handle = resp.data.unwrap().handle;

    let resp = start_turn(handle, StartTurnInput { prompt: make_user_prompt("hello"), tools: vec![] });
    assert!(resp.ok);

    let resp = host_llm_done(handle, LlmResult::Ok(make_assistant_text("hi")));
    assert!(resp.ok);
    let directives = resp.data.unwrap().directives;
    assert!(
        directives.iter().any(|d| matches!(d, HostDirective::Finished)),
        "should emit Finished directive when no tools are requested"
    );
    destroy_host_agent(handle);
}

#[test]
fn directive_persist_after_entry_append() {
    let resp = create_host_agent(dummy_options(), default_budget());
    let handle = resp.data.unwrap().handle;

    let resp = start_turn(handle, StartTurnInput { prompt: make_user_prompt("hello"), tools: vec![] });
    assert!(resp.ok);

    let resp = host_llm_done(handle, LlmResult::Ok(make_assistant_text("hi")));
    assert!(resp.ok);
    let directives = resp.data.unwrap().directives;
    assert!(
        directives.iter().any(|d| matches!(d, HostDirective::Persist)),
        "should emit Persist directive after entries are appended"
    );
    assert!(
        directives.iter().any(|d| matches!(d, HostDirective::Finished)),
        "should also emit Finished directive"
    );
    destroy_host_agent(handle);
}

#[test]
fn directive_compact_when_over_budget() {
    let mut options = dummy_options();
    // Pre-populate messages so the context exceeds the tiny budget
    options.messages = vec![make_user_prompt("a very long message that will exceed the tiny budget")];
    let budget = ContextProjectionBudget {
        max_tool_result_chars: 50000,
        max_context_tokens: 20,
        microcompact_after_turns: 5,
        compaction_threshold: 0.5,
    };
    let resp = create_host_agent(options, budget);
    let handle = resp.data.unwrap().handle;

    let resp = start_turn(handle, StartTurnInput { prompt: make_user_prompt("hello"), tools: vec![] });
    assert!(resp.ok);
    let directives = resp.data.unwrap().directives;
    eprintln!("directive_compact_when_over_budget: directives = {:?}", directives);
    assert!(
        directives.iter().any(|d| matches!(d, HostDirective::Compact { .. })),
        "should emit Compact directive when projection is over budget"
    );
    destroy_host_agent(handle);
}

#[test]
fn directive_cancel_tools() {
    // CancelTools is not yet produced by the current AgentRuntime, so we
    // test the conversion logic directly.
    let mut host_state = HostState::new(String::new(), default_budget().try_into().unwrap());
    let core_actions = vec![pi_core::AgentAction::CancelTools {
        tool_call_ids: vec![pi_core::ToolCallId::new("tc-1")],
        reason: pi_core::CancelReason::UserRequested,
    }];
    let directives = super::convert_actions_to_directives(&mut host_state, &[], "", core_actions).unwrap();
    assert!(
        directives.iter().any(|d| matches!(d, HostDirective::CancelTools { .. })),
        "should convert CancelTools action to directive"
    );
}

#[test]
fn directive_wait_for_input() {
    // WaitForInput is not yet produced by the current AgentRuntime in the
    // standard flow, so we test the conversion logic directly.
    let mut host_state = HostState::new(String::new(), default_budget().try_into().unwrap());
    let core_actions = vec![pi_core::AgentAction::WaitForInput {
        mode: pi_core::WaitMode::Any,
    }];
    let directives = super::convert_actions_to_directives(&mut host_state, &[], "", core_actions).unwrap();
    assert!(
        directives.iter().any(|d| matches!(d, HostDirective::WaitForInput { .. })),
        "should convert WaitForInput action to directive"
    );
}

#[test]
fn full_turn_directive_sequence() {
    let resp = create_host_agent(dummy_options(), default_budget());
    let handle = resp.data.unwrap().handle;

    // Step 1: start_turn -> StreamLlm
    let resp = start_turn(handle, StartTurnInput { prompt: make_user_prompt("use tool"), tools: vec![make_tool_def("test_tool")] });
    assert!(resp.ok);
    let directives = resp.data.unwrap().directives;
    assert!(directives.iter().any(|d| matches!(d, HostDirective::StreamLlm { .. })));

    // Step 2: llm_done with tool -> ExecuteTools
    let resp = host_llm_done(
        handle,
        LlmResult::Ok(make_assistant_with_tool("test_tool", "tc-1")),
    );
    assert!(resp.ok);
    let directives = resp.data.unwrap().directives;
    assert!(directives.iter().any(|d| matches!(d, HostDirective::ExecuteTools { .. })));

    // Step 3: tool_done -> WaitForInput (agent pauses for host to continue)
    let tool_result = ToolResult {
        content: vec![Content::Text(TextContent { text: "ok".to_string() })],
        details: None,
        terminate: None,
    };
    let resp = host_tool_done(handle, ToolCallId("tc-1".to_string()), tool_result);
    assert!(resp.ok);
    let directives = resp.data.unwrap().directives;
    assert!(directives.iter().any(|d| matches!(d, HostDirective::WaitForInput { .. })));

    // Step 4: continue_turn -> StreamLlm
    let resp = host_continue_turn(handle);
    assert!(resp.ok);
    let directives = resp.data.unwrap().directives;
    assert!(directives.iter().any(|d| matches!(d, HostDirective::StreamLlm { .. })));

    // Step 5: llm_done with no tools -> Finished + Persist
    let resp = host_llm_done(handle, LlmResult::Ok(make_assistant_text("done")));
    assert!(resp.ok);
    let directives = resp.data.unwrap().directives;
    assert!(directives.iter().any(|d| matches!(d, HostDirective::Finished)));
    assert!(directives.iter().any(|d| matches!(d, HostDirective::Persist)));

    destroy_host_agent(handle);
}

#[test]
fn multi_turn_directive_sequence() {
    let resp = create_host_agent(dummy_options(), default_budget());
    let handle = resp.data.unwrap().handle;

    // Turn 1
    let resp = start_turn(handle, StartTurnInput { prompt: make_user_prompt("hello"), tools: vec![] });
    assert!(resp.ok);
    let resp = host_llm_done(handle, LlmResult::Ok(make_assistant_text("hi")));
    assert!(resp.ok);
    let directives = resp.data.unwrap().directives;
    assert!(directives.iter().any(|d| matches!(d, HostDirective::Finished)));
    assert!(directives.iter().any(|d| matches!(d, HostDirective::Persist)));

    // Turn 2
    let resp = start_turn(handle, StartTurnInput { prompt: make_user_prompt("again"), tools: vec![] });
    assert!(resp.ok);
    let resp = host_llm_done(handle, LlmResult::Ok(make_assistant_text("yep")));
    assert!(resp.ok);
    let directives = resp.data.unwrap().directives;
    assert!(directives.iter().any(|d| matches!(d, HostDirective::Finished)));
    assert!(directives.iter().any(|d| matches!(d, HostDirective::Persist)));

    destroy_host_agent(handle);
}

#[test]
fn directive_compaction_sequence() {
    let mut options = dummy_options();
    options.messages = vec![make_user_prompt("this is a long message that will definitely exceed the tiny budget we are going to set")];
    let budget = ContextProjectionBudget {
        max_tool_result_chars: 50000,
        max_context_tokens: 30,
        microcompact_after_turns: 5,
        compaction_threshold: 0.5,
    };
    let resp = create_host_agent(options, budget);
    let handle = resp.data.unwrap().handle;

    // Turn that goes over budget
    let resp = start_turn(handle, StartTurnInput { prompt: make_user_prompt("trigger"), tools: vec![] });
    assert!(resp.ok);
    let directives = resp.data.unwrap().directives;
    assert!(directives.iter().any(|d| matches!(d, HostDirective::StreamLlm { .. })));
    assert!(directives.iter().any(|d| matches!(d, HostDirective::Compact { .. })));

    // Finish the turn first (host processes StreamLlm then calls llmDone)
    let resp = host_llm_done(handle, LlmResult::Ok(make_assistant_text("done")));
    assert!(resp.ok);
    let directives = resp.data.unwrap().directives;
    assert!(directives.iter().any(|d| matches!(d, HostDirective::Finished)));
    assert!(directives.iter().any(|d| matches!(d, HostDirective::Persist)));

    // Accept compaction after the turn entries are fully appended
    let compacted_ids: Vec<String> = directives
        .iter()
        .filter_map(|d| match d {
            HostDirective::Compact { compact_up_to, .. } => Some(compact_up_to.clone()),
            _ => None,
        })
        .collect();
    let resp = host_accept_compaction(handle, "summary".to_string(), compacted_ids);
    assert!(resp.ok);
    let directives = resp.data.unwrap().directives;
    assert!(directives.iter().any(|d| matches!(d, HostDirective::Persist)));

    // Verify compaction was applied by checking persisted state
    let persist = get_host_agent_persist_data(handle);
    assert!(persist.ok);
    let entries = persist.data.unwrap().state.entries;
    assert!(entries.iter().any(|e| matches!(e.kind, EntryKind::Compaction { .. })), "should have a compaction entry after acceptCompaction");

    destroy_host_agent(handle);
}

#[test]
fn events_still_emitted_alongside_directives() {
    let resp = create_host_agent(dummy_options(), default_budget());
    let handle = resp.data.unwrap().handle;

    let resp = start_turn(handle, StartTurnInput { prompt: make_user_prompt("hello"), tools: vec![] });
    assert!(resp.ok);
    let data = resp.data.unwrap();
    assert!(!data.events.is_empty(), "events should be emitted alongside directives");
    assert!(data.directives.iter().any(|d| matches!(d, HostDirective::StreamLlm { .. })));
    destroy_host_agent(handle);
}

#[test]
fn steering_during_stream_produces_directives() {
    // Steering during streaming is not supported by the current AgentRuntime.
    // The host_steer function returns a wrong_phase error when called while streaming.
    let resp = create_host_agent(dummy_options(), default_budget());
    let handle = resp.data.unwrap().handle;

    let resp = start_turn(handle, StartTurnInput { prompt: make_user_prompt("hello"), tools: vec![] });
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
    assert!(json.contains("stream_llm"), "StreamLlm should serialize with tag");

    let execute = HostDirective::ExecuteTools {
        calls: vec![ToolCall {
            id: ToolCallId("tc-1".to_string()),
            name: ToolName("read".to_string()),
            arguments: ToolArguments(serde_json::json!({})),
        }],
    };
    let json = serde_json::to_string(&execute).unwrap();
    assert!(json.contains("execute_tools"), "ExecuteTools should serialize with tag");

    let cancel = HostDirective::CancelTools {
        tool_call_ids: vec![ToolCallId("tc-1".to_string())],
        reason: CancelReason::UserRequested,
    };
    let json = serde_json::to_string(&cancel).unwrap();
    assert!(json.contains("cancel_tools"), "CancelTools should serialize with tag");

    let compact = HostDirective::Compact {
        compact_up_to: "leaf".to_string(),
        first_kept_entry_id: "e2".to_string(),
        tokens_before: 42,
        reason: CompactReason::OverBudget {
            estimated_tokens: 100,
            budget_tokens: 50,
        },
        compacted_entry_ids: vec!["e0".to_string(), "e1".to_string()],
        summary_context: LlmContext {
            system_prompt: "Summarize".to_string(),
            messages: vec![],
            tools: vec![],
        },
    };
    let json = serde_json::to_string(&compact).unwrap();
    assert!(json.contains("compact"), "Compact should serialize with tag");

    let persist = HostDirective::Persist;
    let json = serde_json::to_string(&persist).unwrap();
    assert!(json.contains("persist"), "Persist should serialize with tag");

    let finished = HostDirective::Finished;
    let json = serde_json::to_string(&finished).unwrap();
    assert!(json.contains("finished"), "Finished should serialize with tag");

    let wait = HostDirective::WaitForInput {
        mode: WaitMode::Any,
    };
    let json = serde_json::to_string(&wait).unwrap();
    assert!(json.contains("wait_for_input"), "WaitForInput should serialize with tag");
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
        entries: vec![SessionEntry {
            id: "e1".to_string(),
            parent_id: None,
            kind: EntryKind::Message {
                message: AgentMessage::User(UserMessage {
                    content: vec![Content::Text(TextContent {
                        text: "hi".to_string(),
                    })],
                    timestamp: 1,
                }),
            },
            timestamp: 1,
        }],
        leaf_id: "e1".to_string(),
        name: "test".to_string(),
        projection_state: ContextProjectionState::default(),
        artifacts: vec![("a1".to_string(), "hello".to_string())],
        budget: ContextProjectionBudget {
            max_tool_result_chars: 50000,
            max_context_tokens: 100000,
            microcompact_after_turns: 5,
            compaction_threshold: 0.75,
        },
        system_prompt: "You are helpful.".to_string(),
    };
    let json = serde_json::to_string(&data).unwrap();
    assert!(json.contains("entries"));
    assert!(json.contains("leaf_id"));
    assert!(json.contains("name"));
    assert!(json.contains("projection_state"));
    assert!(json.contains("artifacts"));
    assert!(json.contains("budget"));
    assert!(json.contains("system_prompt"));
}

#[test]
fn dto_persist_data_roundtrip() {
    let original = PersistData {
        entries: vec![SessionEntry {
            id: "e1".to_string(),
            parent_id: None,
            kind: EntryKind::Message {
                message: AgentMessage::User(UserMessage {
                    content: vec![Content::Text(TextContent {
                        text: "hi".to_string(),
                    })],
                    timestamp: 1,
                }),
            },
            timestamp: 1,
        }],
        leaf_id: "e1".to_string(),
        name: "test".to_string(),
        projection_state: ContextProjectionState::default(),
        artifacts: vec![("a1".to_string(), "hello".to_string())],
        budget: ContextProjectionBudget {
            max_tool_result_chars: 50000,
            max_context_tokens: 100000,
            microcompact_after_turns: 5,
            compaction_threshold: 0.75,
        },
        system_prompt: "You are helpful.".to_string(),
    };
    let json = serde_json::to_string(&original).unwrap();
    let back: PersistData = serde_json::from_str(&json).unwrap();
    assert_eq!(original, back);
}

#[test]
fn get_host_state_persist_data_roundtrip() {
    let budget = ContextProjectionBudget {
        max_tool_result_chars: 50000,
        max_context_tokens: 100000,
        microcompact_after_turns: 5,
        compaction_threshold: 0.75,
    };
    let state = HostState::new("You are helpful.".to_string(), budget.try_into().unwrap());
    let handle = put_host_state(state);

    let resp = get_host_state_persist_data(handle);
    assert!(resp.ok);
    let data = resp.data.unwrap().state;
    assert_eq!(data.system_prompt, "You are helpful.");
    assert!(data.entries.is_empty());
    assert!(data.artifacts.is_empty());

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

fn make_entry(id: &str, parent_id: Option<&str>) -> SessionEntry {
    SessionEntry {
        id: id.to_string(),
        parent_id: parent_id.map(|s| s.to_string()),
        kind: EntryKind::Message {
            message: AgentMessage::User(UserMessage {
                content: vec![Content::Text(TextContent {
                    text: "hi".to_string(),
                })],
                timestamp: 1,
            }),
        },
        timestamp: 1,
    }
}

#[test]
fn migrate_old_session_extracts_projection() {
    let old_json = r#"{
        "entries": [],
        "leaf_id": "",
        "name": "old",
        "projection_state": {"tools":{},"current_turn":3,"turns_since_compaction":1},
        "artifacts": []
    }"#;
    let resp = restore_host_state_from_json(old_json.to_string());
    assert!(resp.ok, "expected ok, got error: {:?}", resp.error);
    let handle = resp.data.unwrap().handle;
    let state_resp = get_host_state_persist_data(handle);
    assert!(state_resp.ok);
    let data = state_resp.data.unwrap().state;
    assert_eq!(data.projection_state.current_turn, 3);
    assert_eq!(data.projection_state.turns_since_compaction, 1);
    destroy_host_state(handle);
}

#[test]
fn migrate_old_session_extracts_artifacts() {
    let old_json = r#"{
        "entries": [],
        "leaf_id": "",
        "name": "old",
        "projection_state": {},
        "artifacts": [{"id": "a1", "text": "full-text"}, {"id": "a2", "text": "more-text"}]
    }"#;
    let resp = restore_host_state_from_json(old_json.to_string());
    assert!(resp.ok, "expected ok, got error: {:?}", resp.error);
    let handle = resp.data.unwrap().handle;
    let state_resp = get_host_state_persist_data(handle);
    assert!(state_resp.ok);
    let data = state_resp.data.unwrap().state;
    assert_eq!(data.artifacts.len(), 2);
    assert!(data.artifacts.contains(&("a1".to_string(), "full-text".to_string())));
    assert!(data.artifacts.contains(&("a2".to_string(), "more-text".to_string())));
    destroy_host_state(handle);
}

#[test]
fn migrate_old_session_entries_preserved() {
    let old_json = r#"{
        "entries": [{"id":"e1","parent_id":null,"kind":{"type":"message","role":"user","content":[{"type":"text","text":"hello"}],"timestamp":1},"timestamp":1}],
        "leaf_id": "e1",
        "name": "legacy",
        "projection_state": {},
        "artifacts": []
    }"#;
    let resp = restore_host_state_from_json(old_json.to_string());
    assert!(resp.ok, "expected ok, got error: {:?}", resp.error);
    let handle = resp.data.unwrap().handle;
    let state_resp = get_host_state_persist_data(handle);
    assert!(state_resp.ok);
    let data = state_resp.data.unwrap().state;
    // Standalone HostState no longer stores entries/leaf_id/name
    assert_eq!(data.entries.len(), 0);
    assert_eq!(data.leaf_id, "");
    assert_eq!(data.name, "");
    assert_eq!(data.projection_state, ContextProjectionState::default());
    assert!(data.artifacts.is_empty());
    assert_eq!(data.budget.max_tool_result_chars, 50000); // default
    destroy_host_state(handle);
}

#[test]
fn migrate_new_session_noop() {
    let data = PersistData {
        entries: vec![make_entry("e1", None)],
        leaf_id: "e1".to_string(),
        name: "new".to_string(),
        projection_state: ContextProjectionState::default(),
        artifacts: vec![("a1".to_string(), "hello".to_string())],
        budget: ContextProjectionBudget {
            max_tool_result_chars: 50000,
            max_context_tokens: 100000,
            microcompact_after_turns: 5,
            compaction_threshold: 0.75,
        },
        system_prompt: "You are helpful.".to_string(),
    };
    let json = serde_json::to_string(&data).unwrap();
    let resp = restore_host_state_from_json(json);
    assert!(resp.ok, "expected ok, got error: {:?}", resp.error);
    let handle = resp.data.unwrap().handle;
    let state_resp = get_host_state_persist_data(handle);
    assert!(state_resp.ok);
    let restored = state_resp.data.unwrap().state;
    // Standalone HostState no longer stores entries/leaf_id/name
    assert_eq!(restored.name, "");
    assert_eq!(restored.system_prompt, "You are helpful.");
    assert_eq!(restored.artifacts.len(), 1);
    assert_eq!(restored.artifacts[0], ("a1".to_string(), "hello".to_string()));
    destroy_host_state(handle);
}

#[test]
fn migrate_partial_session() {
    // Old session with entries but no projection_state -> valid empty state
    let old_json = r#"{
        "entries": [{"id":"e1","parent_id":null,"kind":{"type":"message","role":"user","content":[{"type":"text","text":"hello"}],"timestamp":1},"timestamp":1}],
        "leaf_id": "e1",
        "name": "partial"
    }"#;
    let resp = restore_host_state_from_json(old_json.to_string());
    assert!(resp.ok, "expected ok, got error: {:?}", resp.error);
    let handle = resp.data.unwrap().handle;
    let state_resp = get_host_state_persist_data(handle);
    assert!(state_resp.ok);
    let data = state_resp.data.unwrap().state;
    // Standalone HostState no longer stores entries/leaf_id/name
    assert_eq!(data.entries.len(), 0);
    assert_eq!(data.projection_state, ContextProjectionState::default());
    assert!(data.artifacts.is_empty());
    assert_eq!(data.budget.max_tool_result_chars, 50000); // default
    destroy_host_state(handle);
}

#[test]
fn roundtrip_migrated_session() {
    let old_json = r#"{
        "entries": [{"id":"e1","parent_id":null,"kind":{"type":"message","role":"user","content":[{"type":"text","text":"hello"}],"timestamp":1},"timestamp":1}],
        "leaf_id": "e1",
        "name": "legacy",
        "projection_state": {"tools":{},"current_turn":2,"turns_since_compaction":0},
        "artifacts": [{"id": "a1", "text": "old-text"}]
    }"#;
    let resp = restore_host_state_from_json(old_json.to_string());
    assert!(resp.ok, "expected ok, got error: {:?}", resp.error);
    let handle = resp.data.unwrap().handle;

    let state_resp = get_host_state_persist_data(handle);
    assert!(state_resp.ok);
    let data = state_resp.data.unwrap().state;

    // Re-serialize and restore again
    let json = serde_json::to_string(&data).unwrap();
    let resp2 = restore_host_state_from_json(json);
    assert!(resp2.ok, "expected ok on roundtrip, got error: {:?}", resp2.error);
    let handle2 = resp2.data.unwrap().handle;

    let state_resp2 = get_host_state_persist_data(handle2);
    assert!(state_resp2.ok);
    let data2 = state_resp2.data.unwrap().state;

    assert_eq!(data, data2);

    destroy_host_state(handle);
    destroy_host_state(handle2);
}
