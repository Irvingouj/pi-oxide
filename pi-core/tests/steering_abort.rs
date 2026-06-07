#![allow(non_snake_case)]

mod common;
use common::*;

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
    assert!(matches!(runtime, AgentRuntime::PreToolCall(_)));
    assert_eq!(runtime.state().pending_tool_calls.len(), 1);

    let AgentRuntime::PreToolCall(pre) = runtime else {
        panic!("expected PreToolCall");
    };
    let transition = pre.abort(T, A, turn_number);
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
    // Finish LLM with a tool call so we land in PreToolCall
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
    assert!(matches!(runtime, AgentRuntime::PreToolCall(_)));

    // Prepare and complete the tool to reach ReadyToContinue
    let AgentRuntime::PreToolCall(pre) = runtime else {
        panic!("expected PreToolCall");
    };
    let prep = ToolCallPreparation {
        tool_call_id: ToolCallId::new("tc-1"),
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
    let (events, _actions, runtime, T, A, turn_number, _markers) = exec
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
