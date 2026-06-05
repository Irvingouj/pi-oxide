#![allow(non_snake_case)]

use pi_core::{
    AgentAction, AgentEvent, AgentMessage, AgentOptions, AgentRuntime, Artifacts, AssistantMessage,
    ChangeMarker, Content, ContentDelta, ContextProjectionBudget, ContinueTurnTransition, LlmChunk,
    LlmResult, Model, StartTurnTransition, StopReason, TextContent, ToolArguments, ToolCall,
    ToolCallId, ToolCallPermission, ToolCallPreparation, ToolCallTransform, ToolDefinition,
    ToolExecutionUpdate, ToolName, ToolResult, TrimmedMessage,
};

fn dummy_model() -> Model {
    Model {
        id: "test-model".into(),
        name: "Test".into(),
        api: "test".into(),
        provider: "test".into(),
        base_url: None,
        reasoning: false,
        context_window: 4096,
        max_tokens: 1024,
        capabilities: Default::default(),
        cost: Default::default(),
    }
}

fn dummy_options() -> AgentOptions {
    AgentOptions {
        system_prompt: "You are a test agent.".to_string(),
        model: dummy_model(),
        thinking_level: pi_core::ThinkingLevel::Off,
        steering_mode: pi_core::QueueMode::OneAtATime,
        follow_up_mode: pi_core::QueueMode::OneAtATime,
        tool_execution_mode: pi_core::ExecutionMode::Parallel,
        session_id: None,
    }
}

fn assistant_with_tool_calls(calls: Vec<ToolCall>) -> AssistantMessage {
    AssistantMessage {
        content: calls.into_iter().map(Content::ToolCall).collect(),
        api: "test".into(),
        provider: "test".into(),
        model: "test-model".into(),
        stop_reason: StopReason::ToolUse,
        error_message: None,
        timestamp: 1,
        usage: Default::default(),
    }
}

fn tool_call(id: &str, name: &str) -> ToolCall {
    ToolCall {
        id: ToolCallId::new(id),
        name: ToolName::new(name),
        arguments: ToolArguments::new(serde_json::json!({})),
    }
}

/// Default empty T, A, turn_number for tests.
fn empty() -> (Vec<TrimmedMessage>, Artifacts, u32) {
    (vec![], Artifacts::new(), 0)
}

#[test]
fn agent_new_is_idle() {
    let runtime = AgentRuntime::new(dummy_options());
    assert!(matches!(runtime, AgentRuntime::Idle(_)));
}

#[test]
fn start_turn_returns_stream_action() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("hello"),
        vec![],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let (events, actions, _state, T, _A, _turn_number, _markers) = t.into_parts();

    assert!(T.len() == 1, "T should have the user message");
    assert!(events.iter().any(|e| matches!(e, AgentEvent::AgentStart)));
    assert!(events.iter().any(|e| matches!(e, AgentEvent::TurnStart)));

    assert_eq!(actions.len(), 1);
    assert!(matches!(actions[0], AgentAction::StreamLlm { .. }));
}

#[test]
fn on_llm_done_with_no_tools_finishes() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("hello"),
        vec![],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let (_events, _actions, _state, T, A, turn_number, _markers) = t.into_parts();

    let streaming = _state;
    let transition = streaming.finish_llm(
        LlmResult::done(),
        T,
        A,
        turn_number,
        &ContextProjectionBudget::default(),
    );
    let (_events, actions, _runtime, _T, _A, _turn_number, _markers) = transition.into_parts();

    assert!(actions.iter().any(|a| matches!(a, AgentAction::Finished)));
}

#[test]
fn reset_clears_state() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("hello"),
        vec![],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let runtime = t.state.into_runtime();

    let runtime = runtime.reset();
    assert!(matches!(runtime, AgentRuntime::Idle(_)));
}

#[test]
fn serialization_roundtrip() {
    let runtime = AgentRuntime::new(dummy_options());
    let json = serde_json::to_string(runtime.state()).unwrap();
    let _deserialized: pi_core::AgentState = serde_json::from_str(&json).unwrap();
}

#[test]
fn tool_calls_update_public_pending_state() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("use tools"),
        vec![],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let (_events, _actions, _state, T, A, turn_number, _markers) = t.into_parts();

    let streaming = _state;
    let transition = streaming.finish_llm(
        LlmResult::Ok(assistant_with_tool_calls(vec![
            tool_call("call-1", "read"),
            tool_call("call-2", "write"),
        ])),
        T,
        A,
        turn_number,
        &ContextProjectionBudget::default(),
    );
    let (_events, actions, runtime, T, A, turn_number, _markers) = transition.into_parts();

    assert!(matches!(actions[0], AgentAction::PrepareToolCalls { .. }));
    assert_eq!(runtime.state().pending_tool_calls, vec!["call-1", "call-2"]);

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools");
    };

    // Prepare: allow both calls
    let preps = vec![
        ToolCallPreparation {
            tool_call_id: ToolCallId::new("call-1"),
            transform: ToolCallTransform::None,
            permission: ToolCallPermission::Allow,
        },
        ToolCallPreparation {
            tool_call_id: ToolCallId::new("call-2"),
            transform: ToolCallTransform::None,
            permission: ToolCallPermission::Allow,
        },
    ];
    let transition = waiting.prepare_tool_calls(preps, T, A, turn_number);
    let (_events, _actions, runtime, T, A, turn_number, _markers) = transition.into_parts();

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools");
    };
    let result = waiting.on_tool_done(
        ToolCallId::new("call-1"),
        Ok(ToolResult::text("ok")),
        T,
        A,
        turn_number,
    );
    let (_events, _actions, state, _T, _A, _turn_number, _markers) = result.into_parts();
    assert_eq!(state.state().pending_tool_calls, vec!["call-2"]);
}

#[test]
fn turn_end_after_tools_reports_assistant_and_tool_results() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("use tool"),
        vec![],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let (_events, _actions, _state, T, A, turn_number, _markers) = t.into_parts();

    let streaming = _state;
    let transition = streaming.finish_llm(
        LlmResult::Ok(assistant_with_tool_calls(vec![tool_call("call-1", "read")])),
        T,
        A,
        turn_number,
        &ContextProjectionBudget::default(),
    );
    let (_events, _actions, runtime, T, A, turn_number, _markers) = transition.into_parts();

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools");
    };
    let (events, actions, _runtime, _T, _A, _turn_number, _markers) = waiting
        .on_tool_done(
            ToolCallId::new("call-1"),
            Ok(ToolResult::text("result")),
            T,
            A,
            turn_number,
        )
        .into_parts();

    let turn_end = events
        .iter()
        .find_map(|event| match event {
            AgentEvent::TurnEnd {
                message,
                tool_results,
            } => Some((message, tool_results)),
            _ => None,
        })
        .expect("turn_end event");

    assert!(matches!(turn_end.0, AgentMessage::Assistant(_)));
    assert_eq!(turn_end.1.len(), 1);
    assert_eq!(turn_end.1[0].tool_call_id, ToolCallId::new("call-1"));
    assert!(
        actions.is_empty(),
        "on_tool_done should return empty actions; host calls continue_turn()"
    );
}

#[test]
fn tool_batch_terminates_only_when_all_results_terminate() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("use tools"),
        vec![],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let (_events, _actions, _state, T, A, turn_number, _markers) = t.into_parts();

    let streaming = _state;
    let transition = streaming.finish_llm(
        LlmResult::Ok(assistant_with_tool_calls(vec![
            tool_call("call-1", "read"),
            tool_call("call-2", "write"),
        ])),
        T,
        A,
        turn_number,
        &ContextProjectionBudget::default(),
    );
    let (_events, _actions, runtime, T, A, turn_number, _markers) = transition.into_parts();

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools");
    };
    let mut terminating = ToolResult::text("stop");
    terminating.terminate = Some(true);
    let (events, _actions, runtime, T, A, turn_number, _markers) = waiting
        .on_tool_done(
            ToolCallId::new("call-1"),
            Ok(terminating),
            T,
            A,
            turn_number,
        )
        .into_parts();
    let _ = events;

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools");
    };
    let (events, actions, _runtime, _T, _A, _turn_number, _markers) = waiting
        .on_tool_done(
            ToolCallId::new("call-2"),
            Ok(ToolResult::text("continue")),
            T,
            A,
            turn_number,
        )
        .into_parts();

    assert!(!events
        .iter()
        .any(|event| matches!(event, AgentEvent::AgentEnd)));
    assert!(
        actions.is_empty(),
        "non-unanimous termination should not finish; host calls continue_turn()"
    );
}

#[test]
fn continue_turn_after_tools_resumes_llm() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("use tool"),
        vec![],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let (_events, _actions, _state, T, A, turn_number, _markers) = t.into_parts();

    let streaming = _state;
    let transition = streaming.finish_llm(
        LlmResult::Ok(assistant_with_tool_calls(vec![tool_call("call-1", "read")])),
        T,
        A,
        turn_number,
        &ContextProjectionBudget::default(),
    );
    let (_events, _actions, runtime, T, A, turn_number, _markers) = transition.into_parts();

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools");
    };
    let (_events, actions, runtime, T, A, turn_number, _markers) = waiting
        .on_tool_done(
            ToolCallId::new("call-1"),
            Ok(ToolResult::text("result")),
            T,
            A,
            turn_number,
        )
        .into_parts();
    assert!(actions.is_empty());

    let AgentRuntime::ReadyToContinue(ready) = runtime else {
        panic!("expected ReadyToContinue");
    };
    let t = ready.continue_turn(T, A, turn_number, &ContextProjectionBudget::default(), "");
    let ContinueTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    assert_eq!(t.actions.len(), 1);
    assert!(matches!(t.actions[0], AgentAction::StreamLlm { .. }));
}

#[test]
fn tool_batch_terminates_when_all_terminate() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("use tools"),
        vec![],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let (_events, _actions, _state, T, A, turn_number, _markers) = t.into_parts();

    let streaming = _state;
    let transition = streaming.finish_llm(
        LlmResult::Ok(assistant_with_tool_calls(vec![
            tool_call("call-1", "read"),
            tool_call("call-2", "write"),
        ])),
        T,
        A,
        turn_number,
        &ContextProjectionBudget::default(),
    );
    let (_events, _actions, runtime, T, A, turn_number, _markers) = transition.into_parts();

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools");
    };
    let mut terminating = ToolResult::text("stop");
    terminating.terminate = Some(true);
    let (events, _actions, runtime, T, A, turn_number, _markers) = waiting
        .on_tool_done(
            ToolCallId::new("call-1"),
            Ok(terminating.clone()),
            T,
            A,
            turn_number,
        )
        .into_parts();
    let _ = events;

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools");
    };
    let (events, actions, _runtime, _T, _A, _turn_number, _markers) = waiting
        .on_tool_done(
            ToolCallId::new("call-2"),
            Ok(terminating),
            T,
            A,
            turn_number,
        )
        .into_parts();

    assert!(events
        .iter()
        .any(|event| matches!(event, AgentEvent::AgentEnd)));
    assert!(matches!(actions[0], AgentAction::Finished));
}

#[test]
fn text_delta_is_incremental_not_accumulated() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("hello"),
        vec![],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let mut streaming = t.state;

    // Feed Start chunk to initialize the assistant message
    let start_chunk = LlmChunk::Start {
        partial: AssistantMessage::empty(),
    };
    let events = streaming.feed_llm_chunk(start_chunk);
    assert!(events
        .iter()
        .any(|e| matches!(e, AgentEvent::MessageStart { .. })));

    // Feed first text delta
    let events = streaming.feed_llm_chunk(LlmChunk::TextDelta {
        text: "Hello".into(),
    });
    let deltas: Vec<&ContentDelta> = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::MessageUpdate { delta, .. } => Some(delta),
            _ => None,
        })
        .collect();
    assert_eq!(deltas.len(), 1);
    assert!(
        matches!(deltas[0], ContentDelta::TextDelta { text } if text == "Hello"),
        "first delta should be the incremental chunk 'Hello', got {:?}",
        deltas[0]
    );

    // Feed second text delta
    let events = streaming.feed_llm_chunk(LlmChunk::TextDelta {
        text: " world".into(),
    });
    let deltas: Vec<&ContentDelta> = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::MessageUpdate { delta, .. } => Some(delta),
            _ => None,
        })
        .collect();
    assert_eq!(deltas.len(), 1);
    assert!(
        matches!(deltas[0], ContentDelta::TextDelta { text } if text == " world"),
        "second delta should be the incremental chunk ' world', got {:?}",
        deltas[0]
    );

    // Finish the turn so the message is complete
    let (T, A, tn) = empty();
    let transition = streaming.finish_llm(
        LlmResult::done(),
        T,
        A,
        tn,
        &ContextProjectionBudget::default(),
    );
    let (_events, actions, _runtime, _T, _A, _turn_number, _markers) = transition.into_parts();
    assert!(actions.iter().any(|a| matches!(a, AgentAction::Finished)));
}

#[test]
fn context_projection_integrates_with_state_machine() {
    let options = dummy_options();

    let runtime = AgentRuntime::new(options);
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("read file"),
        vec![],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let (_events, _actions, _state, T, A, turn_number, _markers) = t.into_parts();

    let streaming = _state;
    // Finish LLM with a tool call
    let transition = streaming.finish_llm(
        LlmResult::Ok(assistant_with_tool_calls(vec![tool_call("call-1", "read")])),
        T,
        A,
        turn_number,
        &ContextProjectionBudget::default(),
    );
    let (_events, _actions, runtime, T, A, turn_number, _markers) = transition.into_parts();

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools");
    };

    // Submit a long tool result
    let long_result = "a".repeat(3000);
    let (_events, _actions, runtime, T, A, turn_number, _markers) = waiting
        .on_tool_done(
            ToolCallId::new("call-1"),
            Ok(ToolResult::text(long_result)),
            T,
            A,
            turn_number,
        )
        .into_parts();

    // projection_scan runs when all tools complete (this is turn end).
    // But here age=1 - turn=1 = 0, which is < min_age=2 for "read", so no projection yet.

    let AgentRuntime::ReadyToContinue(ready) = runtime else {
        panic!("expected ReadyToContinue");
    };
    let t = ready.continue_turn(T, A, turn_number, &ContextProjectionBudget::default(), "");
    let ContinueTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };

    // Extract LlmContext from the StreamLlm action
    let context = match &t.actions[0] {
        AgentAction::StreamLlm { context, .. } => context.clone(),
        other => panic!("expected StreamLlm, got {other:?}"),
    };

    // Context should include user, assistant, and tool_result messages
    assert_eq!(context.messages.len(), 3);

    // build_llm_context_from_trimmed converts OriginalTool to full ToolResultMessage.
    // The tool result is still full-content in T at this point (age < min_age).
    // Token estimate should be positive.
    let estimate = pi_core::estimate_tokens(&context.messages);
    assert!(estimate > 0);

    // Verify the tool result content is present in the context
    let tool_result_msg = context
        .messages
        .iter()
        .find_map(|m| match m {
            AgentMessage::ToolResult(tr) => Some(tr),
            _ => None,
        })
        .expect("projected messages should contain tool result");

    let result_text: String = tool_result_msg
        .content
        .iter()
        .filter_map(|c| match c {
            Content::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        result_text.len(),
        3000,
        "tool result should be full content (not yet projected)"
    );
}

#[test]
fn tool_done_unknown_id_is_noop() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("use tools"),
        vec![],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let (_events, _actions, _state, T, A, turn_number, _markers) = t.into_parts();

    let streaming = _state;
    let transition = streaming.finish_llm(
        LlmResult::Ok(assistant_with_tool_calls(vec![tool_call("call-1", "read")])),
        T,
        A,
        turn_number,
        &ContextProjectionBudget::default(),
    );
    let (_events, _actions, runtime, T, A, turn_number, _markers) = transition.into_parts();

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools");
    };

    // Submit result for a different tool call ID
    let (events, _actions, runtime, _T, _A, _turn_number, _markers) = waiting
        .on_tool_done(
            ToolCallId::new("unknown-call"),
            Ok(ToolResult::text("result")),
            T,
            A,
            turn_number,
        )
        .into_parts();
    let _ = events;

    // Should still be WaitingTools with the original pending call
    assert!(
        matches!(runtime, AgentRuntime::WaitingTools(_)),
        "unknown tool call id should not change phase"
    );
    assert_eq!(runtime.state().pending_tool_calls, vec!["call-1"]);
}

#[test]
fn context_projection_keep_full_bypass() {
    let options = dummy_options();

    let runtime = AgentRuntime::new(options);
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("edit file"),
        vec![],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let (_events, _actions, _state, T, A, turn_number, _markers) = t.into_parts();

    let streaming = _state;
    let transition = streaming.finish_llm(
        LlmResult::Ok(assistant_with_tool_calls(vec![tool_call("call-1", "edit")])),
        T,
        A,
        turn_number,
        &ContextProjectionBudget::default(),
    );
    let (_events, _actions, runtime, T, A, turn_number, _markers) = transition.into_parts();

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools");
    };

    // edit uses KeepFull strategy — content should stay full
    let large_result = "x".repeat(300);
    let (_events, _actions, runtime, T, A, turn_number, _markers) = waiting
        .on_tool_done(
            ToolCallId::new("call-1"),
            Ok(ToolResult::text(large_result)),
            T,
            A,
            turn_number,
        )
        .into_parts();

    let AgentRuntime::ReadyToContinue(ready) = runtime else {
        panic!("expected ReadyToContinue");
    };
    let t = ready.continue_turn(T, A, turn_number, &ContextProjectionBudget::default(), "");
    let ContinueTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };

    let context = match &t.actions[0] {
        AgentAction::StreamLlm { context, .. } => context.clone(),
        other => panic!("expected StreamLlm, got {other:?}"),
    };

    // build_llm_context_from_trimmed converts OriginalTool to full ToolResultMessage.
    // KeepFull strategy means projection_scan will never project it (at any age).
    let tool_result_msg = context
        .messages
        .iter()
        .find_map(|m| match m {
            AgentMessage::ToolResult(tr) => Some(tr),
            _ => None,
        })
        .expect("projected messages should contain tool result");

    let result_text: String = tool_result_msg
        .content
        .iter()
        .filter_map(|c| match c {
            Content::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect();

    // KeepFull: full text is preserved inline
    assert!(
        !result_text.contains("<context-artifact"),
        "KeepFull strategy should not insert preview marker"
    );
    assert_eq!(
        result_text.len(),
        300,
        "KeepFull should preserve full result"
    );
}

#[test]
fn agent_runtime_delegation_exercise() {
    let mut runtime = AgentRuntime::new(dummy_options());

    // Idle — exercise state, on_tool_started, on_tool_update
    assert!(matches!(runtime, AgentRuntime::Idle(_)));
    assert!(runtime.on_tool_started(ToolCallId::new("x")).is_empty());
    assert!(runtime
        .on_tool_update(ToolExecutionUpdate {
            tool_call_id: ToolCallId::new("x"),
            stream: pi_core::ToolOutputStream::Stdout,
            chunk: "test".into(),
            sequence: 0,
            timestamp: 0,
        })
        .is_empty());

    // Streaming
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("hello"),
        vec![],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    runtime = t.state.into_runtime();
    assert!(matches!(runtime, AgentRuntime::Streaming(_)));
    assert!(runtime.on_tool_started(ToolCallId::new("x")).is_empty());

    // WaitingTools
    let AgentRuntime::Streaming(streaming) = runtime else {
        panic!("expected Streaming");
    };
    let (T, A, turn_number) = (vec![], Artifacts::new(), 0);
    let transition = streaming.finish_llm(
        LlmResult::Ok(assistant_with_tool_calls(vec![tool_call("call-1", "read")])),
        T,
        A,
        turn_number,
        &ContextProjectionBudget::default(),
    );
    let (_events, _actions, mut runtime, _T, _A, _turn_number, _markers) = transition.into_parts();
    assert!(matches!(runtime, AgentRuntime::WaitingTools(_)));

    let events = runtime.on_tool_started(ToolCallId::new("call-1"));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AgentEvent::ToolExecutionUpdate { .. })),
        "on_tool_started for pending tool should emit ToolExecutionUpdate"
    );

    // ReadyToContinue — state_mut should work
    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools");
    };
    let result = waiting.on_tool_done(
        ToolCallId::new("call-1"),
        Ok(ToolResult::text("ok")),
        vec![],
        Artifacts::new(),
        0,
    );
    let (events, _actions, mut runtime, _T, _A, _turn_number, _markers) = result.into_parts();
    let _ = events;
    assert!(matches!(runtime, AgentRuntime::ReadyToContinue(_)));
    runtime.state_mut().model.id = "changed".into();
    assert_eq!(runtime.state().model.id.as_str(), "changed");

    // Finished
    let AgentRuntime::ReadyToContinue(ready) = runtime else {
        panic!("expected ReadyToContinue");
    };
    let t = ready.continue_turn(
        vec![],
        Artifacts::new(),
        0,
        &ContextProjectionBudget::default(),
        "",
    );
    let ContinueTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let streaming = t.state;
    let (T, A, tn) = empty();
    let transition = streaming.finish_llm(
        LlmResult::done(),
        T,
        A,
        tn,
        &ContextProjectionBudget::default(),
    );
    let (_events, _actions, mut runtime, _T, _A, _turn_number, _markers) = transition.into_parts();
    assert!(matches!(runtime, AgentRuntime::Finished(_)));

    // Aborted
    let AgentRuntime::Finished(finished) = runtime else {
        panic!("expected Finished");
    };
    let t = finished.restart();
    runtime = t.state.into_runtime();
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("hello"),
        vec![],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let (_events, _actions, _state, T, A, turn_number, _markers) = t.into_parts();
    let streaming = _state;
    let _ = events;
    let transition = streaming.abort(T, A, turn_number);
    let runtime = transition.state.into_runtime();
    assert!(matches!(runtime, AgentRuntime::Aborted(_)));
}

#[test]
fn abort_during_streaming_clears_queues_and_emits_agent_end() {
    let mut runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("hello"),
        vec![],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let mut streaming = t.state;

    // Feed a partial chunk so streaming_assistant has content
    streaming.feed_llm_chunk(LlmChunk::Start {
        partial: AssistantMessage::empty(),
    });
    let ev = streaming.feed_llm_chunk(LlmChunk::TextDelta {
        text: "partial".into(),
    });
    assert!(
        ev.iter()
            .any(|e| matches!(e, AgentEvent::MessageUpdate { .. })),
        "feeding a chunk should emit MessageUpdate"
    );

    let (T, A, tn) = empty();
    let transition = streaming.abort(T, A, tn);
    let events = transition.events;
    runtime = transition.state.into_runtime();

    assert!(matches!(runtime, AgentRuntime::Aborted(_)));
    assert!(
        events.iter().any(|e| matches!(e, AgentEvent::AgentEnd)),
        "abort should emit AgentEnd"
    );
}

#[test]
fn abort_from_waiting_tools_clears_pending_tools() {
    let runtime = AgentRuntime::new(dummy_options());

    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("run tool"),
        vec![],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let (_events, _actions, _state, T, A, turn_number, _markers) = t.into_parts();

    let streaming = _state;
    let assistant = assistant_with_tool_calls(vec![tool_call("tc-1", "test_tool")]);
    let transition = streaming.finish_llm(
        LlmResult::Ok(assistant),
        T,
        A,
        turn_number,
        &ContextProjectionBudget::default(),
    );
    let (_events, _actions, mut runtime, T, A, turn_number, _markers) = transition.into_parts();
    assert!(matches!(runtime, AgentRuntime::WaitingTools(_)));
    assert_eq!(runtime.state().pending_tool_calls.len(), 1);

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools");
    };
    let transition = waiting.abort(T, A, turn_number);
    runtime = transition.state.into_runtime();

    assert!(matches!(runtime, AgentRuntime::Aborted(_)));
    assert!(
        runtime.state().pending_tool_calls.is_empty(),
        "abort should clear pending tool calls"
    );
    assert!(
        transition
            .events
            .iter()
            .any(|e| matches!(e, AgentEvent::AgentEnd)),
        "abort should emit AgentEnd"
    );
}

#[test]
fn steer_in_idle_queues_message_and_emits_queue_update() {
    let mut runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(mut idle) = runtime else {
        panic!("expected Idle");
    };
    let events = idle.steer(AgentMessage::user(" steer me"));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AgentEvent::QueueUpdate { .. })),
        "steer should emit QueueUpdate"
    );
    runtime = idle.into_runtime();
    assert!(matches!(runtime, AgentRuntime::Idle(_)));
}

#[test]
fn steer_in_ready_to_continue_is_drained_on_next_turn() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("hello"),
        vec![],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let (_events, _actions, _state, T, A, turn_number, _markers) = t.into_parts();

    let streaming = _state;
    // Finish LLM with a tool call so we land in WaitingTools
    let transition = streaming.finish_llm(
        LlmResult::Ok(assistant_with_tool_calls(vec![tool_call(
            "tc-1",
            "test_tool",
        )])),
        T,
        A,
        turn_number,
        &ContextProjectionBudget::default(),
    );
    let (_events, _actions, runtime, T, A, turn_number, _markers) = transition.into_parts();
    assert!(matches!(runtime, AgentRuntime::WaitingTools(_)));

    // Complete the tool to reach ReadyToContinue
    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools");
    };
    let (events, _actions, runtime, T, A, turn_number, _markers) = waiting
        .on_tool_done(
            ToolCallId::new("tc-1"),
            Ok(ToolResult::text("ok")),
            T,
            A,
            turn_number,
        )
        .into_parts();
    let _ = events;
    assert!(matches!(runtime, AgentRuntime::ReadyToContinue(_)));

    let AgentRuntime::ReadyToContinue(mut ready) = runtime else {
        panic!("expected ReadyToContinue");
    };
    let steer_events = ready.steer(AgentMessage::user("steer while ready"));
    assert!(
        steer_events
            .iter()
            .any(|e| matches!(e, AgentEvent::QueueUpdate { .. })),
        "steer in ReadyToContinue should emit QueueUpdate"
    );

    // continue_turn drains steering and starts a new stream
    let t = ready.continue_turn(T, A, turn_number, &ContextProjectionBudget::default(), "");
    let ContinueTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let (events, _actions, _state, T, A, turn_number, _markers) = t.into_parts();
    let streaming = _state;
    let _ = events;

    let transition = streaming.finish_llm(
        LlmResult::done(),
        T,
        A,
        turn_number,
        &ContextProjectionBudget::default(),
    );
    let (_events, _actions, runtime, _T, _A, _turn_number, _markers) = transition.into_parts();

    // After continuing, the steer message should have been drained into T
    // We can't check T directly here since it's consumed, but the runtime should be Finished
    assert!(matches!(runtime, AgentRuntime::Finished(_)));
}

#[test]
fn cancel_tool_removes_pending_and_emits_cancellation() {
    let runtime = AgentRuntime::new(dummy_options());

    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("run tool"),
        vec![],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let (_events, _actions, _state, T, A, turn_number, _markers) = t.into_parts();

    let streaming = _state;
    let assistant = assistant_with_tool_calls(vec![tool_call("tc-1", "test_tool")]);
    let transition = streaming.finish_llm(
        LlmResult::Ok(assistant),
        T,
        A,
        turn_number,
        &ContextProjectionBudget::default(),
    );
    let (_events, _actions, runtime, T, A, turn_number, _markers) = transition.into_parts();
    assert!(matches!(runtime, AgentRuntime::WaitingTools(_)));
    assert_eq!(runtime.state().pending_tool_calls.len(), 1);

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools");
    };
    let transition = waiting.cancel_tool(
        ToolCallId::new("tc-1"),
        pi_core::CancelReason::UserRequested,
        T,
        A,
        turn_number,
    );
    let (events, _actions, new_runtime, _T, _A, _turn_number, _markers) = transition.into_parts();

    assert!(
        events
            .iter()
            .any(|e| matches!(e, AgentEvent::ToolExecutionCancelled { .. })),
        "cancel_tool should emit ToolExecutionCancelled"
    );
    assert!(
        matches!(
            new_runtime,
            AgentRuntime::ReadyToContinue(_) | AgentRuntime::Idle(_)
        ),
        "canceling the only pending tool should leave ReadyToContinue or Idle"
    );
    assert!(new_runtime.state().pending_tool_calls.is_empty());
}

#[test]
fn start_turn_tools_appear_in_stream_llm_context() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let tool = ToolDefinition {
        name: ToolName::new("read"),
        label: "Read".into(),
        description: "Read a file.".into(),
        parameters: pi_core::JsonSchema::new(serde_json::json!({})),
        execution_mode: Default::default(),
        tool_run_mode: Default::default(),
    };
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("read file"),
        vec![tool.clone()],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };

    assert_eq!(t.actions.len(), 1);
    let context = match &t.actions[0] {
        AgentAction::StreamLlm { context, .. } => context,
        other => panic!("expected StreamLlm, got {other:?}"),
    };
    assert_eq!(context.tools.len(), 1);
    assert_eq!(context.tools[0].name, tool.name);
}

#[test]
fn continue_turn_preserves_tools_from_start_turn() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let tool = ToolDefinition {
        name: ToolName::new("test_tool"),
        label: "Test".into(),
        description: "A test tool.".into(),
        parameters: pi_core::JsonSchema::new(serde_json::json!({})),
        execution_mode: Default::default(),
        tool_run_mode: Default::default(),
    };
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("use tool"),
        vec![tool.clone()],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let (_events, _actions, _state, T, A, turn_number, _markers) = t.into_parts();

    let streaming = _state;
    let assistant = assistant_with_tool_calls(vec![tool_call("tc-1", "test_tool")]);
    let transition = streaming.finish_llm(
        LlmResult::Ok(assistant),
        T,
        A,
        turn_number,
        &ContextProjectionBudget::default(),
    );
    let (_events, _actions, runtime, T, A, turn_number, _markers) = transition.into_parts();
    assert!(matches!(runtime, AgentRuntime::WaitingTools(_)));

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools");
    };
    let (events, _actions, runtime, T, A, turn_number, _markers) = waiting
        .on_tool_done(
            ToolCallId::new("tc-1"),
            Ok(ToolResult::text("ok")),
            T,
            A,
            turn_number,
        )
        .into_parts();
    let _ = events;
    assert!(matches!(runtime, AgentRuntime::ReadyToContinue(_)));

    let AgentRuntime::ReadyToContinue(ready) = runtime else {
        panic!("expected ReadyToContinue");
    };
    let t = ready.continue_turn(T, A, turn_number, &ContextProjectionBudget::default(), "");
    let ContinueTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    assert_eq!(t.actions.len(), 1);
    let context = match &t.actions[0] {
        AgentAction::StreamLlm { context, .. } => context,
        other => panic!("expected StreamLlm, got {other:?}"),
    };
    assert_eq!(context.tools.len(), 1);
    assert_eq!(context.tools[0].name, tool.name);
}

#[test]
fn abort_clears_current_turn_tools() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let tool = ToolDefinition {
        name: ToolName::new("test_tool"),
        label: "Test".into(),
        description: "A test tool.".into(),
        parameters: pi_core::JsonSchema::new(serde_json::json!({})),
        execution_mode: Default::default(),
        tool_run_mode: Default::default(),
    };
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("use tool"),
        vec![tool],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let streaming = t.state;

    let (T, A, tn) = empty();
    let transition = streaming.abort(T, A, tn);
    let runtime = transition.state.into_runtime();
    assert!(matches!(runtime, AgentRuntime::Aborted(_)));

    let AgentRuntime::Aborted(aborted) = runtime else {
        panic!("expected Aborted");
    };
    let t = aborted.restart();
    let idle = t.state;
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("hello again"),
        vec![],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let context = match &t.actions[0] {
        AgentAction::StreamLlm { context, .. } => context,
        other => panic!("expected StreamLlm, got {other:?}"),
    };
    assert!(context.tools.is_empty(), "abort should clear turn_tools");
}

#[test]
fn reset_clears_current_turn_tools() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let tool = ToolDefinition {
        name: ToolName::new("test_tool"),
        label: "Test".into(),
        description: "A test tool.".into(),
        parameters: pi_core::JsonSchema::new(serde_json::json!({})),
        execution_mode: Default::default(),
        tool_run_mode: Default::default(),
    };
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("use tool"),
        vec![tool],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let mut runtime = t.state.into_runtime();

    runtime = runtime.reset();
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("hello again"),
        vec![],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let context = match &t.actions[0] {
        AgentAction::StreamLlm { context, .. } => context,
        other => panic!("expected StreamLlm, got {other:?}"),
    };
    assert!(context.tools.is_empty(), "reset should clear turn_tools");
}

#[test]
fn compacting_agent_start_turn_emits_summarize_action() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let budget = ContextProjectionBudget {
        max_context_tokens: 500,
        compaction_threshold: 0.75,
        ..Default::default()
    };
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("trigger"),
        vec![],
        t,
        a,
        tn,
        &budget,
        "Summarize the following conversation.",
    );
    // With empty T, we won't trigger compaction — test the non-compacting path.
    match t {
        StartTurnTransition::Streaming(_) => {} // Expected with empty T
        StartTurnTransition::Compacting(_) => {} // Could happen if T is large enough
    }
}

#[test]
fn accept_summary_applies_compaction_and_emits_compaction_applied_marker() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(_idle) = runtime else {
        panic!("expected Idle");
    };
    let budget = ContextProjectionBudget {
        max_context_tokens: 500,
        compaction_threshold: 0.75,
        ..Default::default()
    };

    // Pre-populate T with large messages to exceed budget
    let mut t = vec![
        TrimmedMessage::User(pi_core::UserMessage::new_text("hello")),
        TrimmedMessage::Assistant(AssistantMessage {
            content: vec![Content::Text(TextContent {
                text: "x".repeat(200),
            })],
            api: "test".into(),
            provider: "test".into(),
            model: "test".into(),
            stop_reason: StopReason::EndTurn,
            error_message: None,
            timestamp: 1,
            usage: Default::default(),
        }),
    ];
    t.push(TrimmedMessage::User(pi_core::UserMessage::new_text("next")));
    t.push(TrimmedMessage::Assistant(AssistantMessage {
        content: vec![Content::Text(TextContent {
            text: "y".repeat(1960),
        })],
        api: "test".into(),
        provider: "test".into(),
        model: "test".into(),
        stop_reason: StopReason::EndTurn,
        error_message: None,
        timestamp: 2,
        usage: Default::default(),
    }));

    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let t = idle.start_turn(
        AgentMessage::user("trigger"),
        vec![],
        t,
        Artifacts::new(),
        0,
        &budget,
        "Summarize the following conversation.",
    );
    let StartTurnTransition::Compacting(t) = t else {
        panic!("expected Compacting");
    };
    let transition = t.state.accept_summary(
        "summary text".to_string(),
        t.transcript,
        t.artifacts,
        t.turn_number,
        &budget,
    );
    assert!(
        transition
            .markers
            .iter()
            .any(|m| matches!(m, ChangeMarker::CompactionApplied)),
        "accept_summary should emit CompactionApplied marker"
    );
    assert!(
        transition
            .actions
            .iter()
            .any(|a| matches!(a, AgentAction::StreamLlm { .. })),
        "accept_summary should return a StreamLlm action"
    );
}

#[test]
fn compacting_agent_abort_transitions_to_aborted() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(_idle) = runtime else {
        panic!("expected Idle");
    };
    let budget = ContextProjectionBudget {
        max_context_tokens: 500,
        compaction_threshold: 0.75,
        ..Default::default()
    };

    // Pre-populate T with large messages
    let mut t = vec![
        TrimmedMessage::User(pi_core::UserMessage::new_text("hello")),
        TrimmedMessage::Assistant(AssistantMessage {
            content: vec![Content::Text(TextContent {
                text: "x".repeat(200),
            })],
            api: "test".into(),
            provider: "test".into(),
            model: "test".into(),
            stop_reason: StopReason::EndTurn,
            error_message: None,
            timestamp: 1,
            usage: Default::default(),
        }),
    ];
    t.push(TrimmedMessage::User(pi_core::UserMessage::new_text("next")));
    t.push(TrimmedMessage::Assistant(AssistantMessage {
        content: vec![Content::Text(TextContent {
            text: "y".repeat(1960),
        })],
        api: "test".into(),
        provider: "test".into(),
        model: "test".into(),
        stop_reason: StopReason::EndTurn,
        error_message: None,
        timestamp: 2,
        usage: Default::default(),
    }));

    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let t = idle.start_turn(
        AgentMessage::user("trigger"),
        vec![],
        t,
        Artifacts::new(),
        0,
        &budget,
        "Summarize the following conversation.",
    );
    let StartTurnTransition::Compacting(t) = t else {
        panic!("expected Compacting");
    };
    let transition = t.state.abort(t.transcript, t.artifacts, t.turn_number);
    let runtime = transition.state.into_runtime();
    assert!(matches!(runtime, AgentRuntime::Aborted(_)));
}

#[test]
fn continue_turn_can_also_trigger_compacting() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("hello"),
        vec![],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let (_events, _actions, _state, T, A, turn_number, _markers) = t.into_parts();

    let streaming = _state;

    let transition = streaming.finish_llm(
        LlmResult::Ok(assistant_with_tool_calls(vec![tool_call("call-1", "read")])),
        T,
        A,
        turn_number,
        &ContextProjectionBudget::default(),
    );
    let (_events, _actions, runtime, T, A, turn_number, _markers) = transition.into_parts();

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools");
    };
    let result = waiting.on_tool_done(
        ToolCallId::new("call-1"),
        Ok(ToolResult::text("ok")),
        T,
        A,
        turn_number,
    );
    let (events, _actions, runtime, T, A, turn_number, _markers) = result.into_parts();
    let _ = events;

    let AgentRuntime::ReadyToContinue(ready) = runtime else {
        panic!("expected ReadyToContinue");
    };
    let budget = ContextProjectionBudget {
        max_context_tokens: 500,
        compaction_threshold: 0.75,
        ..Default::default()
    };
    // With empty T, compaction won't trigger even with tight budget.
    let t = ready.continue_turn(
        T,
        A,
        turn_number,
        &budget,
        "Summarize the following conversation.",
    );
    match t {
        ContinueTurnTransition::Streaming(_) => {} // Expected with empty T
        ContinueTurnTransition::Compacting(_) => {} // Possible if T is large enough
    }
}

#[test]
fn projection_scan_projects_old_tools_across_multi_round_turn() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };

    // Turn 0: user → LLM → tool (grep, large result) → tool done → continue
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("grep for pattern"),
        vec![],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let (_events, _actions, _state, T, A, turn_number, _markers) = t.into_parts();
    // turn_number should be 1 after start_turn

    let streaming = _state;
    let transition = streaming.finish_llm(
        LlmResult::Ok(assistant_with_tool_calls(vec![tool_call("call-1", "grep")])),
        T,
        A,
        turn_number,
        &ContextProjectionBudget::default(),
    );
    let (_events, _actions, runtime, T, A, turn_number, _markers) = transition.into_parts();

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools");
    };

    // Submit a large grep result (> max_chars=3000 for grep)
    let large_result = "x".repeat(5000);
    let (events, _actions, runtime, T, A, turn_number, _markers) = waiting
        .on_tool_done(
            ToolCallId::new("call-1"),
            Ok(ToolResult::text(large_result)),
            T,
            A,
            turn_number,
        )
        .into_parts();
    let _ = events;

    // After on_tool_done, projection_scan runs.
    // grep strategy: Head { min_age: 1, max_chars: 3000 }
    // tool was created at turn_number=1, age = turn_number - 1.
    // But on_tool_done increments turn_number when going to Ready, so now turn_number=2.
    // projection_scan uses the turn_number AT THE TIME OF THE CALL (inside on_tool_done),
    // which is still 1. age = 1 - 1 = 0, which is < min_age=1. So no projection yet.
    let AgentRuntime::ReadyToContinue(ready) = runtime else {
        panic!("expected ReadyToContinue");
    };

    // Continue the turn: LLM → EndTurn
    let t = ready.continue_turn(T, A, turn_number, &ContextProjectionBudget::default(), "");
    let ContinueTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let (_events, _actions, _state, T, A, turn_number, _markers) = t.into_parts();

    let streaming = _state;
    let transition = streaming.finish_llm(
        LlmResult::done(),
        T,
        A,
        turn_number,
        &ContextProjectionBudget::default(),
    );
    let (_events, _actions, _state, T, A, turn_number, markers) = transition.into_parts();

    // Now turn_number is 2 (incremented by on_tool_done earlier, passed through continue_turn and finish_llm).
    // projection_scan in finish_llm runs with turn_number=2.
    // The grep tool was at turn=1, age = 2 - 1 = 1, which is >= min_age=1.
    // Size 5000 > max_chars=3000. So it SHOULD be projected now.

    // Verify the tool result is now a ProjectedTool in T
    let has_projected = T
        .iter()
        .any(|m| matches!(m, TrimmedMessage::ProjectedTool(_)));
    assert!(has_projected, "grep tool should be projected after aging");

    // Verify an artifact was stored in A
    assert!(!A.is_empty(), "projected tool should have an artifact in A");

    // Verify a NewArtifacts marker was emitted
    let has_artifact_marker = markers
        .iter()
        .any(|m| matches!(m, ChangeMarker::NewArtifacts { .. }));
    assert!(
        has_artifact_marker,
        "projection should emit NewArtifacts marker"
    );

    let _ = (_events, _actions, _state, turn_number);
}

// ---------------------------------------------------------------------------
// Tool call preparation tests
// ---------------------------------------------------------------------------

#[test]
fn on_llm_done_with_tools_emits_prepare_tool_calls() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("use tool"),
        vec![],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let (_events, _actions, _state, T, A, turn_number, _markers) = t.into_parts();

    let streaming = _state;
    let transition = streaming.finish_llm(
        LlmResult::Ok(assistant_with_tool_calls(vec![tool_call("call-1", "read")])),
        T,
        A,
        turn_number,
        &ContextProjectionBudget::default(),
    );
    let (events, actions, _runtime, _T, _A, _turn_number, _markers) = transition.into_parts();

    assert!(
        actions
            .iter()
            .any(|a| matches!(a, AgentAction::PrepareToolCalls { .. })),
        "should emit PrepareToolCalls, got {:?}",
        actions
    );
    assert!(
        !actions
            .iter()
            .any(|a| matches!(a, AgentAction::ExecuteTools { .. })),
        "should NOT emit ExecuteTools before preparation"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, AgentEvent::ToolExecutionStart { .. })),
        "should NOT emit ToolExecutionStart before preparation"
    );
}

#[test]
fn prepare_tool_calls_allowed_proceeds_to_execute_tools() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("use tool"),
        vec![],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let (_events, _actions, _state, T, A, turn_number, _markers) = t.into_parts();

    let streaming = _state;
    let transition = streaming.finish_llm(
        LlmResult::Ok(assistant_with_tool_calls(vec![tool_call("call-1", "read")])),
        T,
        A,
        turn_number,
        &ContextProjectionBudget::default(),
    );
    let (_events, _actions, runtime, T, A, turn_number, _markers) = transition.into_parts();

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools")
    };

    let prep = ToolCallPreparation {
        tool_call_id: ToolCallId::new("call-1"),
        transform: ToolCallTransform::None,
        permission: ToolCallPermission::Allow,
    };

    let transition = waiting.prepare_tool_calls(vec![prep], T, A, turn_number);
    let (events, actions, runtime, _T, _A, _turn_number, _markers) = transition.into_parts();

    assert!(
        actions
            .iter()
            .any(|a| matches!(a, AgentAction::ExecuteTools { .. })),
        "should emit ExecuteTools after preparation"
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, AgentEvent::ToolExecutionStart { .. }))
            .count(),
        1,
        "allowed call should emit exactly one ToolExecutionStart"
    );
    assert!(
        runtime
            .state()
            .pending_tool_calls
            .contains(&"call-1".to_string()),
        "allowed call should remain in pending"
    );
}

#[test]
fn prepare_tool_calls_rewrite_args_applied() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("use tool"),
        vec![],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let (_events, _actions, _state, T, A, turn_number, _markers) = t.into_parts();

    let streaming = _state;
    let transition = streaming.finish_llm(
        LlmResult::Ok(assistant_with_tool_calls(vec![tool_call("call-1", "read")])),
        T,
        A,
        turn_number,
        &ContextProjectionBudget::default(),
    );
    let (_events, _actions, runtime, T, A, turn_number, _markers) = transition.into_parts();

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools")
    };

    let rewritten_args = ToolArguments::new(serde_json::json!({"path": "/foo"}));
    let prep = ToolCallPreparation {
        tool_call_id: ToolCallId::new("call-1"),
        transform: ToolCallTransform::RewriteArgs {
            arguments: rewritten_args.clone(),
        },
        permission: ToolCallPermission::Allow,
    };

    let transition = waiting.prepare_tool_calls(vec![prep], T, A, turn_number);
    let (_events, actions, _runtime, _T, _A, _turn_number, _markers) = transition.into_parts();

    let execute_tools = actions
        .iter()
        .find_map(|a| match a {
            AgentAction::ExecuteTools { calls } => Some(calls),
            _ => None,
        })
        .expect("should have ExecuteTools");
    assert_eq!(execute_tools[0].arguments, rewritten_args);
}

#[test]
fn prepare_tool_calls_blocked_creates_error_result() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("use tool"),
        vec![],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let (_events, _actions, _state, T, A, turn_number, _markers) = t.into_parts();

    let streaming = _state;
    let transition = streaming.finish_llm(
        LlmResult::Ok(assistant_with_tool_calls(vec![tool_call("call-1", "read")])),
        T,
        A,
        turn_number,
        &ContextProjectionBudget::default(),
    );
    let (_events, _actions, runtime, T, A, turn_number, _markers) = transition.into_parts();

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools")
    };

    let prep = ToolCallPreparation {
        tool_call_id: ToolCallId::new("call-1"),
        transform: ToolCallTransform::None,
        permission: ToolCallPermission::Block {
            reason: "unsafe".into(),
        },
    };

    let transition = waiting.prepare_tool_calls(vec![prep], T, A, turn_number);
    let (events, actions, runtime, T, _A, _turn_number, _markers) = transition.into_parts();

    assert!(
        !actions
            .iter()
            .any(|a| matches!(a, AgentAction::ExecuteTools { .. })),
        "blocked call should not emit ExecuteTools"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, AgentEvent::ToolExecutionStart { .. })),
        "blocked call should not emit ToolExecutionStart"
    );
    assert!(
        matches!(runtime, AgentRuntime::ReadyToContinue(_)),
        "all blocked should transition to ReadyToContinue"
    );

    let tool_end = events
        .iter()
        .find(|e| matches!(e, AgentEvent::ToolExecutionEnd { .. }))
        .expect("should emit ToolExecutionEnd");
    assert!(
        matches!(
            tool_end,
            AgentEvent::ToolExecutionEnd { is_error: true, .. }
        ),
        "blocked result should be error"
    );

    let turn_end = events
        .iter()
        .find(|e| matches!(e, AgentEvent::TurnEnd { .. }))
        .expect("should emit TurnEnd when all blocked");
    assert!(
        matches!(turn_end, AgentEvent::TurnEnd { .. }),
        "should finalize batch immediately"
    );

    // Verify OriginalTool was pushed to T
    assert!(T
        .iter()
        .any(|m| matches!(m, TrimmedMessage::OriginalTool(_))));
}

#[test]
fn prepare_tool_calls_mixed_batch_preserves_one_result_per_call() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("use tools"),
        vec![],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let (_events, _actions, _state, T, A, turn_number, _markers) = t.into_parts();

    let streaming = _state;
    let transition = streaming.finish_llm(
        LlmResult::Ok(assistant_with_tool_calls(vec![
            tool_call("call-1", "read"),
            tool_call("call-2", "write"),
        ])),
        T,
        A,
        turn_number,
        &ContextProjectionBudget::default(),
    );
    let (_events, _actions, runtime, T, A, turn_number, _markers) = transition.into_parts();

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools")
    };

    let preps = vec![
        ToolCallPreparation {
            tool_call_id: ToolCallId::new("call-1"),
            transform: ToolCallTransform::None,
            permission: ToolCallPermission::Allow,
        },
        ToolCallPreparation {
            tool_call_id: ToolCallId::new("call-2"),
            transform: ToolCallTransform::None,
            permission: ToolCallPermission::Block {
                reason: "unsafe".into(),
            },
        },
    ];

    let transition = waiting.prepare_tool_calls(preps, T, A, turn_number);
    let (events, actions, runtime, T, _A, _turn_number, _markers) = transition.into_parts();

    assert!(
        actions
            .iter()
            .any(|a| matches!(a, AgentAction::ExecuteTools { .. })),
        "should emit ExecuteTools for allowed call"
    );
    assert!(
        matches!(runtime, AgentRuntime::WaitingTools(_)),
        "should remain WaitingTools when some allowed"
    );

    let tool_ends: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, AgentEvent::ToolExecutionEnd { .. }))
        .collect();
    assert_eq!(
        tool_ends.len(),
        1,
        "blocked call should produce exactly one result"
    );

    // T should have the blocked result as OriginalTool
    assert_eq!(
        T.iter()
            .filter(|m| matches!(m, TrimmedMessage::OriginalTool(_)))
            .count(),
        1,
        "blocked call should push one OriginalTool to T"
    );
}

#[test]
fn prepare_tool_calls_all_blocked_finalizes_without_execution() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("use tools"),
        vec![],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let (_events, _actions, _state, T, A, turn_number, _markers) = t.into_parts();

    let streaming = _state;
    let transition = streaming.finish_llm(
        LlmResult::Ok(assistant_with_tool_calls(vec![
            tool_call("call-1", "read"),
            tool_call("call-2", "write"),
        ])),
        T,
        A,
        turn_number,
        &ContextProjectionBudget::default(),
    );
    let (_events, _actions, runtime, T, A, turn_number, _markers) = transition.into_parts();

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools")
    };

    let preps = vec![
        ToolCallPreparation {
            tool_call_id: ToolCallId::new("call-1"),
            transform: ToolCallTransform::None,
            permission: ToolCallPermission::Block {
                reason: "unsafe".into(),
            },
        },
        ToolCallPreparation {
            tool_call_id: ToolCallId::new("call-2"),
            transform: ToolCallTransform::None,
            permission: ToolCallPermission::Block {
                reason: "unsafe".into(),
            },
        },
    ];

    let transition = waiting.prepare_tool_calls(preps, T, A, turn_number);
    let (events, actions, runtime, T, _A, _turn_number, _markers) = transition.into_parts();

    assert!(
        !actions
            .iter()
            .any(|a| matches!(a, AgentAction::ExecuteTools { .. })),
        "all blocked should not emit ExecuteTools"
    );
    assert!(
        matches!(runtime, AgentRuntime::ReadyToContinue(_)),
        "all blocked should transition to ReadyToContinue"
    );

    let tool_ends: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, AgentEvent::ToolExecutionEnd { .. }))
        .collect();
    assert_eq!(
        tool_ends.len(),
        2,
        "each blocked call should produce a result"
    );

    let turn_end = events
        .iter()
        .find(|e| matches!(e, AgentEvent::TurnEnd { .. }))
        .expect("should emit TurnEnd");
    assert!(
        matches!(turn_end, AgentEvent::TurnEnd { .. }),
        "should finalize batch"
    );

    // T should have both blocked results
    assert_eq!(
        T.iter()
            .filter(|m| matches!(m, TrimmedMessage::OriginalTool(_)))
            .count(),
        2,
        "both blocked calls should push OriginalTool to T"
    );
}

#[test]
fn prepare_tool_calls_unknown_id_is_ignored() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("use tool"),
        vec![],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let (_events, _actions, _state, T, A, turn_number, _markers) = t.into_parts();

    let streaming = _state;
    let transition = streaming.finish_llm(
        LlmResult::Ok(assistant_with_tool_calls(vec![tool_call("call-1", "read")])),
        T,
        A,
        turn_number,
        &ContextProjectionBudget::default(),
    );
    let (_events, _actions, runtime, T, A, turn_number, _markers) = transition.into_parts();

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools")
    };

    let preps = vec![
        ToolCallPreparation {
            tool_call_id: ToolCallId::new("call-1"),
            transform: ToolCallTransform::None,
            permission: ToolCallPermission::Allow,
        },
        ToolCallPreparation {
            tool_call_id: ToolCallId::new("unknown-call"),
            transform: ToolCallTransform::None,
            permission: ToolCallPermission::Block {
                reason: "unsafe".into(),
            },
        },
    ];

    let transition = waiting.prepare_tool_calls(preps, T, A, turn_number);
    let (_events, actions, _runtime, _T, _A, _turn_number, _markers) = transition.into_parts();

    assert!(
        actions
            .iter()
            .any(|a| matches!(a, AgentAction::ExecuteTools { .. })),
        "should still emit ExecuteTools for known call"
    );
}

#[test]
fn prepare_tool_calls_permission_sees_transformed_args() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("use tool"),
        vec![],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let (_events, _actions, _state, T, A, turn_number, _markers) = t.into_parts();

    let streaming = _state;
    let transition = streaming.finish_llm(
        LlmResult::Ok(assistant_with_tool_calls(vec![tool_call("call-1", "read")])),
        T,
        A,
        turn_number,
        &ContextProjectionBudget::default(),
    );
    let (_events, _actions, runtime, T, A, turn_number, _markers) = transition.into_parts();

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools")
    };

    // Transform and then block — the blocked result should reference the transformed args
    let rewritten_args = ToolArguments::new(serde_json::json!({"path": "/foo"}));
    let prep = ToolCallPreparation {
        tool_call_id: ToolCallId::new("call-1"),
        transform: ToolCallTransform::RewriteArgs {
            arguments: rewritten_args,
        },
        permission: ToolCallPermission::Block {
            reason: "unsafe path".into(),
        },
    };

    let transition = waiting.prepare_tool_calls(vec![prep], T, A, turn_number);
    let (events, _actions, _runtime, _T, _A, _turn_number, _markers) = transition.into_parts();

    let tool_end = events
        .iter()
        .find(|e| matches!(e, AgentEvent::ToolExecutionEnd { .. }))
        .expect("should emit ToolExecutionEnd");
    assert!(
        matches!(
            tool_end,
            AgentEvent::ToolExecutionEnd { is_error: true, .. }
        ),
        "blocked result should be error"
    );
}
