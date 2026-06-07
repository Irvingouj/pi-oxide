#![allow(non_snake_case)]

mod common;
use common::*;

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
fn agent_runtime_delegation_exercise() {
    let mut runtime = AgentRuntime::new(dummy_options());

    // Idle — exercise state
    assert!(matches!(runtime, AgentRuntime::Idle(_)));

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

    // PreToolCall
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
    assert!(matches!(runtime, AgentRuntime::PreToolCall(_)));

    // ExecutingTools — on_tool_started should emit ToolExecutionUpdate
    let AgentRuntime::PreToolCall(pre) = runtime else {
        panic!("expected PreToolCall");
    };
    let prep = ToolCallPreparation {
        tool_call_id: ToolCallId::new("call-1"),
        transform: ToolCallTransform::None,
        permission: ToolCallPermission::Allow,
    };
    let transition = pre.prepare_tool_calls(vec![prep], vec![], Artifacts::new(), 0);
    let (_events, _actions, mut runtime, _T, _A, _turn_number, _markers) = transition.into_parts();
    assert!(matches!(runtime, AgentRuntime::ExecutingTools(_)));

    let AgentRuntime::ExecutingTools(mut exec) = runtime else {
        panic!("expected ExecutingTools");
    };
    let events = exec.on_tool_started(ToolCallId::new("call-1"));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AgentEvent::ToolExecutionUpdate { .. })),
        "on_tool_started for pending tool should emit ToolExecutionUpdate"
    );
    runtime = exec.into_runtime();

    // ReadyToContinue — state_mut should work
    let AgentRuntime::ExecutingTools(exec) = runtime else {
        panic!("expected ExecutingTools");
    };
    let result = exec.on_tool_done(
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
    assert!(matches!(runtime, AgentRuntime::PreToolCall(_)));

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
