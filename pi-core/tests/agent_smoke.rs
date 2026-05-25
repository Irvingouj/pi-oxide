use pi_core::{
    Agent, AgentAction, AgentEvent, AgentMessage, AgentOptions, AssistantMessage, Content,
    LlmResult, Model, Phase, StopReason, ToolArguments, ToolCall, ToolCallId, ToolName, ToolResult,
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
    let agent = Agent::new(dummy_options());
    assert_eq!(agent.phase, Phase::Idle);
    assert!(!agent.state().is_streaming);
    assert!(agent.state().messages.is_empty());
}

#[test]
fn start_turn_returns_stream_action() {
    let mut agent = Agent::new(dummy_options());
    let (events, actions) = agent.start_turn(AgentMessage::user("hello"));

    assert_eq!(agent.phase, Phase::Streaming);
    assert!(agent.state().is_streaming);
    assert_eq!(agent.state().messages.len(), 1);

    // Should emit: AgentStart, TurnStart, MessageStart, MessageEnd
    assert!(events.iter().any(|e| matches!(e, AgentEvent::AgentStart)));
    assert!(events.iter().any(|e| matches!(e, AgentEvent::TurnStart)));

    // Should request LLM streaming
    assert_eq!(actions.len(), 1);
    assert!(matches!(actions[0], AgentAction::StreamLlm { .. }));
}

#[test]
fn on_llm_done_with_no_tools_finishes() {
    let mut agent = Agent::new(dummy_options());
    agent.start_turn(AgentMessage::user("hello"));

    let result = LlmResult::done();
    let (events, actions) = agent.on_llm_done(result);

    assert_eq!(agent.phase, Phase::Idle);
    assert!(!agent.state().is_streaming);

    assert!(events
        .iter()
        .any(|e| matches!(e, AgentEvent::AgentEnd { .. })));
    assert!(matches!(actions[0], AgentAction::Finished { .. }));
}

#[test]
fn reset_clears_state() {
    let mut agent = Agent::new(dummy_options());
    agent.start_turn(AgentMessage::user("hello"));
    agent.reset();

    assert_eq!(agent.phase, Phase::Idle);
    assert!(agent.state().messages.is_empty());
    assert!(!agent.state().is_streaming);
}

#[test]
fn serialization_roundtrip() {
    let agent = Agent::new(dummy_options());
    let json = serde_json::to_string(agent.state()).unwrap();
    let _deserialized: pi_core::AgentState = serde_json::from_str(&json).unwrap();
}

#[test]
fn tool_calls_update_public_pending_state() {
    let mut agent = Agent::new(dummy_options());
    agent.start_turn(AgentMessage::user("use tools"));

    let (_, actions) = agent.on_llm_done(LlmResult::Ok(assistant_with_tool_calls(vec![
        tool_call("call-1", "read"),
        tool_call("call-2", "write"),
    ])));

    assert!(matches!(actions[0], AgentAction::ExecuteTools { .. }));
    assert_eq!(agent.state().pending_tool_calls, vec!["call-1", "call-2"]);

    agent.on_tool_done(ToolCallId::new("call-1"), Ok(ToolResult::text("ok")));
    assert_eq!(agent.state().pending_tool_calls, vec!["call-2"]);
}

#[test]
fn turn_end_after_tools_reports_assistant_and_tool_results() {
    let mut agent = Agent::new(dummy_options());
    agent.start_turn(AgentMessage::user("use tool"));
    agent.on_llm_done(LlmResult::Ok(assistant_with_tool_calls(vec![tool_call(
        "call-1", "read",
    )])));

    let (events, actions) =
        agent.on_tool_done(ToolCallId::new("call-1"), Ok(ToolResult::text("result")));

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
    let mut agent = Agent::new(dummy_options());
    agent.start_turn(AgentMessage::user("use tools"));
    agent.on_llm_done(LlmResult::Ok(assistant_with_tool_calls(vec![
        tool_call("call-1", "read"),
        tool_call("call-2", "write"),
    ])));

    let mut terminating = ToolResult::text("stop");
    terminating.terminate = Some(true);
    agent.on_tool_done(ToolCallId::new("call-1"), Ok(terminating));

    let (events, actions) =
        agent.on_tool_done(ToolCallId::new("call-2"), Ok(ToolResult::text("continue")));

    assert!(!events
        .iter()
        .any(|event| matches!(event, AgentEvent::AgentEnd { .. })));
    assert!(actions.is_empty(), "non-unanimous termination should not finish; host calls continue_turn()");
}

#[test]
fn continue_turn_after_tools_resumes_llm() {
    let mut agent = Agent::new(dummy_options());
    agent.start_turn(AgentMessage::user("use tool"));
    agent.on_llm_done(LlmResult::Ok(assistant_with_tool_calls(vec![tool_call(
        "call-1", "read",
    )])));

    let (_events, actions) =
        agent.on_tool_done(ToolCallId::new("call-1"), Ok(ToolResult::text("result")));
    assert!(actions.is_empty());

    let (_events, actions) = agent.continue_turn();
    assert_eq!(actions.len(), 1);
    assert!(matches!(actions[0], AgentAction::StreamLlm { .. }));
}

#[test]
fn tool_batch_terminates_when_all_terminate() {
    let mut agent = Agent::new(dummy_options());
    agent.start_turn(AgentMessage::user("use tools"));
    agent.on_llm_done(LlmResult::Ok(assistant_with_tool_calls(vec![
        tool_call("call-1", "read"),
        tool_call("call-2", "write"),
    ])));

    let mut terminating = ToolResult::text("stop");
    terminating.terminate = Some(true);
    agent.on_tool_done(ToolCallId::new("call-1"), Ok(terminating.clone()));

    let (events, actions) =
        agent.on_tool_done(ToolCallId::new("call-2"), Ok(terminating));

    assert!(events
        .iter()
        .any(|event| matches!(event, AgentEvent::AgentEnd { .. })));
    assert!(matches!(actions[0], AgentAction::Finished { .. }));
}
