#![allow(non_snake_case)]

mod common;
use common::*;

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


