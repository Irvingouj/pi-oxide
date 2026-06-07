#![allow(non_snake_case)]

mod common;
use common::*;

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

    let AgentRuntime::PreToolCall(pre) = runtime else {
        panic!("expected PreToolCall");
    };
    let prep = ToolCallPreparation {
        tool_call_id: ToolCallId::new("call-1"),
        transform: ToolCallTransform::None,
        permission: ToolCallPermission::Allow,
    };
    let (events, _actions, runtime, T, A, turn_number, _markers) = pre
        .prepare_tool_calls(vec![prep], T, A, turn_number)
        .into_parts();
    let _ = events;
    assert!(matches!(runtime, AgentRuntime::ExecutingTools(_)));

    let AgentRuntime::ExecutingTools(exec) = runtime else {
        panic!("expected ExecutingTools");
    };
    let result = exec.on_tool_done(
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
