#![allow(non_snake_case)]

mod common;
use common::*;

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


#[test]
fn prepare_tool_calls_duplicate_id_is_blocked_once_without_start() {
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
            tool_call_id: ToolCallId::new("call-1"),
            transform: ToolCallTransform::None,
            permission: ToolCallPermission::Allow,
        },
    ];

    let transition = waiting.prepare_tool_calls(preps, T, A, turn_number);
    let (events, actions, runtime, T, _A, _turn_number, _markers) = transition.into_parts();

    assert!(
        !actions
            .iter()
            .any(|a| matches!(a, AgentAction::ExecuteTools { .. })),
        "duplicate preparation should not execute the tool"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, AgentEvent::ToolExecutionStart { .. })),
        "duplicate preparation should not emit ToolExecutionStart"
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, AgentEvent::ToolExecutionEnd { is_error: true, .. }))
            .count(),
        1,
        "duplicate preparation should emit one error result"
    );
    assert!(
        matches!(runtime, AgentRuntime::ReadyToContinue(_)),
        "duplicate preparation should finalize the blocked batch"
    );
    assert_eq!(
        T.iter()
            .filter(|m| matches!(m, TrimmedMessage::OriginalTool(_)))
            .count(),
        1,
        "duplicate preparation should push one OriginalTool"
    );
}

