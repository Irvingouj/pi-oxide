use pi_core::{
    AgentAction, AgentEvent, AgentMessage, AgentOptions, AgentRuntime, AssistantMessage, Content,
    LlmResult, Model, StopReason, ToolArguments, ToolCall, ToolCallId, ToolName, ToolResult,
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
        tools: vec![],
        steering_mode: pi_core::QueueMode::OneAtATime,
        follow_up_mode: pi_core::QueueMode::OneAtATime,
        tool_execution_mode: pi_core::ToolExecutionMode::Parallel,
        session_id: None,
        messages: vec![],
        session_state: None,
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

#[test]
fn agent_new_is_idle() {
    let runtime = AgentRuntime::new(dummy_options());
    assert!(!runtime.state().is_streaming);
    assert!(runtime.state().messages.is_empty());
}

#[test]
fn start_turn_returns_stream_action() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let t = idle.start_turn(AgentMessage::user("hello"));

    let runtime = t.state.into_runtime();
    assert!(runtime.state().is_streaming);
    assert_eq!(runtime.state().messages.len(), 1);

    assert!(t.events.iter().any(|e| matches!(e, AgentEvent::AgentStart)));
    assert!(t.events.iter().any(|e| matches!(e, AgentEvent::TurnStart)));

    assert_eq!(t.actions.len(), 1);
    assert!(matches!(t.actions[0], AgentAction::StreamLlm { .. }));
}

#[test]
fn on_llm_done_with_no_tools_finishes() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let t = idle.start_turn(AgentMessage::user("hello"));
    let streaming = t.state;

    let transition = streaming.finish_llm(LlmResult::done());
    let (_events, actions, runtime) = transition.into_parts();

    assert!(!runtime.state().is_streaming);
    assert!(actions.iter().any(|a| matches!(a, AgentAction::Finished { .. })));
}

#[test]
fn reset_clears_state() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let t = idle.start_turn(AgentMessage::user("hello"));
    let runtime = t.state.into_runtime();

    let runtime = runtime.reset();
    assert!(runtime.state().messages.is_empty());
    assert!(!runtime.state().is_streaming);
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
    let t = idle.start_turn(AgentMessage::user("use tools"));
    let streaming = t.state;

    let transition = streaming.finish_llm(LlmResult::Ok(assistant_with_tool_calls(vec![
        tool_call("call-1", "read"),
        tool_call("call-2", "write"),
    ])));
    let (_events, actions, runtime) = transition.into_parts();

    assert!(matches!(actions[0], AgentAction::ExecuteTools { .. }));
    assert_eq!(runtime.state().pending_tool_calls, vec!["call-1", "call-2"]);

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools");
    };
    let transition = waiting.on_tool_done(ToolCallId::new("call-1"), Ok(ToolResult::text("ok")));
    let (_events, _actions, runtime) = transition.into_parts();
    assert_eq!(runtime.state().pending_tool_calls, vec!["call-2"]);
}

#[test]
fn turn_end_after_tools_reports_assistant_and_tool_results() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let t = idle.start_turn(AgentMessage::user("use tool"));
    let streaming = t.state;

    let transition = streaming.finish_llm(LlmResult::Ok(assistant_with_tool_calls(vec![tool_call(
        "call-1", "read",
    )])));
    let (_events, _actions, runtime) = transition.into_parts();

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools");
    };
    let transition = waiting.on_tool_done(ToolCallId::new("call-1"), Ok(ToolResult::text("result")));
    let (events, actions, _runtime) = transition.into_parts();

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
    assert!(actions.is_empty(), "on_tool_done should return empty actions; host calls continue_turn()");
}

#[test]
fn tool_batch_terminates_only_when_all_results_terminate() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let t = idle.start_turn(AgentMessage::user("use tools"));
    let streaming = t.state;

    let transition = streaming.finish_llm(LlmResult::Ok(assistant_with_tool_calls(vec![
        tool_call("call-1", "read"),
        tool_call("call-2", "write"),
    ])));
    let (_events, _actions, runtime) = transition.into_parts();

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools");
    };
    let mut terminating = ToolResult::text("stop");
    terminating.terminate = Some(true);
    let transition = waiting.on_tool_done(ToolCallId::new("call-1"), Ok(terminating));
    let (_events, _actions, runtime) = transition.into_parts();

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools");
    };
    let transition = waiting.on_tool_done(ToolCallId::new("call-2"), Ok(ToolResult::text("continue")));
    let (events, actions, _runtime) = transition.into_parts();

    assert!(!events.iter().any(|event| matches!(event, AgentEvent::AgentEnd { .. })));
    assert!(actions.is_empty(), "non-unanimous termination should not finish; host calls continue_turn()");
}

#[test]
fn continue_turn_after_tools_resumes_llm() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let t = idle.start_turn(AgentMessage::user("use tool"));
    let streaming = t.state;

    let transition = streaming.finish_llm(LlmResult::Ok(assistant_with_tool_calls(vec![tool_call(
        "call-1", "read",
    )])));
    let (_events, _actions, runtime) = transition.into_parts();

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools");
    };
    let transition = waiting.on_tool_done(ToolCallId::new("call-1"), Ok(ToolResult::text("result")));
    let (_events, actions, runtime) = transition.into_parts();
    assert!(actions.is_empty());

    let AgentRuntime::ReadyToContinue(ready) = runtime else {
        panic!("expected ReadyToContinue");
    };
    let t = ready.continue_turn();
    assert_eq!(t.actions.len(), 1);
    assert!(matches!(t.actions[0], AgentAction::StreamLlm { .. }));
}

#[test]
fn tool_batch_terminates_when_all_terminate() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let t = idle.start_turn(AgentMessage::user("use tools"));
    let streaming = t.state;

    let transition = streaming.finish_llm(LlmResult::Ok(assistant_with_tool_calls(vec![
        tool_call("call-1", "read"),
        tool_call("call-2", "write"),
    ])));
    let (_events, _actions, runtime) = transition.into_parts();

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools");
    };
    let mut terminating = ToolResult::text("stop");
    terminating.terminate = Some(true);
    let transition = waiting.on_tool_done(ToolCallId::new("call-1"), Ok(terminating.clone()));
    let (_events, _actions, runtime) = transition.into_parts();

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools");
    };
    let transition = waiting.on_tool_done(ToolCallId::new("call-2"), Ok(terminating));
    let (events, actions, _runtime) = transition.into_parts();

    assert!(events.iter().any(|event| matches!(event, AgentEvent::AgentEnd { .. })));
    assert!(matches!(actions[0], AgentAction::Finished { .. }));
}
