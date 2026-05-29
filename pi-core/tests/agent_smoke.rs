use pi_core::{
    project, AgentAction, AgentEvent, AgentMessage, AgentOptions, AgentRuntime, AssistantMessage,
    Content, ContentDelta, ContextProjectionBudget, ContextProjectionState, JsonSchema, LlmChunk,
    LlmResult, Model, ProjectionInput, StopReason, TextContent, ToolArguments, ToolCall,
    ToolCallId, ToolDefinition, ToolExecutionUpdate, ToolName, ToolResult, ToolResultMessage,
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

fn start_user_turn(
    idle: pi_core::IdleAgent,
    text: &str,
) -> pi_core::Transition<pi_core::StreamingAgent> {
    idle.start_turn(AgentMessage::user(text), vec![])
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
    let t = idle.start_turn(AgentMessage::user("hello"), vec![]);

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
    let t = idle.start_turn(AgentMessage::user("hello"), vec![]);
    let streaming = t.state;

    let transition = streaming.finish_llm(LlmResult::done());
    let (_events, actions, runtime) = transition.into_parts();

    assert!(!runtime.state().is_streaming);
    assert!(actions
        .iter()
        .any(|a| matches!(a, AgentAction::Finished { .. })));
}

#[test]
fn reset_clears_state() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let t = idle.start_turn(AgentMessage::user("hello"), vec![]);
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
    let t = idle.start_turn(AgentMessage::user("use tools"), vec![]);
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
    let t = idle.start_turn(AgentMessage::user("use tool"), vec![]);
    let streaming = t.state;

    let transition =
        streaming.finish_llm(LlmResult::Ok(assistant_with_tool_calls(vec![tool_call(
            "call-1", "read",
        )])));
    let (_events, _actions, runtime) = transition.into_parts();

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools");
    };
    let transition =
        waiting.on_tool_done(ToolCallId::new("call-1"), Ok(ToolResult::text("result")));
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
    let t = idle.start_turn(AgentMessage::user("use tools"), vec![]);
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
    let transition =
        waiting.on_tool_done(ToolCallId::new("call-2"), Ok(ToolResult::text("continue")));
    let (events, actions, _runtime) = transition.into_parts();

    assert!(!events
        .iter()
        .any(|event| matches!(event, AgentEvent::AgentEnd { .. })));
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
    let t = idle.start_turn(AgentMessage::user("use tool"), vec![]);
    let streaming = t.state;

    let transition =
        streaming.finish_llm(LlmResult::Ok(assistant_with_tool_calls(vec![tool_call(
            "call-1", "read",
        )])));
    let (_events, _actions, runtime) = transition.into_parts();

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools");
    };
    let transition =
        waiting.on_tool_done(ToolCallId::new("call-1"), Ok(ToolResult::text("result")));
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
    let t = idle.start_turn(AgentMessage::user("use tools"), vec![]);
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

    assert!(events
        .iter()
        .any(|event| matches!(event, AgentEvent::AgentEnd { .. })));
    assert!(matches!(actions[0], AgentAction::Finished { .. }));
}

#[test]
fn text_delta_is_incremental_not_accumulated() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let t = idle.start_turn(AgentMessage::user("hello"), vec![]);
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
    let transition = streaming.finish_llm(LlmResult::done());
    let (_events, actions, _runtime) = transition.into_parts();
    assert!(actions
        .iter()
        .any(|a| matches!(a, AgentAction::Finished { .. })));
}

#[test]
fn context_projection_integrates_with_state_machine() {
    let options = dummy_options();

    let runtime = AgentRuntime::new(options);
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let t = idle.start_turn(AgentMessage::user("read file"), vec![]);
    let streaming = t.state;

    // Finish LLM with a tool call
    let transition =
        streaming.finish_llm(LlmResult::Ok(assistant_with_tool_calls(vec![tool_call(
            "call-1", "read",
        )])));
    let (_events, _actions, runtime) = transition.into_parts();

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools");
    };

    // Submit a long tool result (longer than Head strategy's 2000-char limit for 'read')
    let long_result = "a".repeat(3000);
    let transition =
        waiting.on_tool_done(ToolCallId::new("call-1"), Ok(ToolResult::text(long_result)));
    let (_events, _actions, runtime) = transition.into_parts();

    let AgentRuntime::ReadyToContinue(ready) = runtime else {
        panic!("expected ReadyToContinue");
    };
    let t = ready.continue_turn();

    // Extract LlmContext from the StreamLlm action
    let context = match &t.actions[0] {
        AgentAction::StreamLlm { context, .. } => context.clone(),
        other => panic!("expected StreamLlm, got {other:?}"),
    };

    // Context should include user, assistant, and tool_result messages
    assert_eq!(context.messages.len(), 3);

    // Add two full agent turns so the tool result is old enough (age >= min_age for 'read')
    let mut messages = context.messages.clone();
    // Turn 1: assistant + tool_result
    messages.push(AgentMessage::Assistant(assistant_with_tool_calls(vec![
        tool_call("call-2", "read"),
    ])));
    messages.push(AgentMessage::ToolResult(ToolResultMessage {
        role: "tool_result".to_string(),
        tool_call_id: ToolCallId::new("call-2"),
        tool_name: ToolName::new("read"),
        content: vec![Content::Text(TextContent {
            text: "ok".to_string(),
        })],
        details: None,
        is_error: false,
        timestamp: 2,
    }));
    messages.push(AgentMessage::user("turn 1"));
    // Turn 2: assistant + tool_result
    messages.push(AgentMessage::Assistant(assistant_with_tool_calls(vec![
        tool_call("call-3", "read"),
    ])));
    messages.push(AgentMessage::ToolResult(ToolResultMessage {
        role: "tool_result".to_string(),
        tool_call_id: ToolCallId::new("call-3"),
        tool_name: ToolName::new("read"),
        content: vec![Content::Text(TextContent {
            text: "ok".to_string(),
        })],
        details: None,
        is_error: false,
        timestamp: 3,
    }));
    messages.push(AgentMessage::user("turn 2"));

    // Project with a tight budget to force tool-result budgeting
    let projection = project(ProjectionInput {
        system_prompt: context.system_prompt.clone(),
        messages,
        budget: ContextProjectionBudget {
            max_tool_result_chars: 100,
            max_context_tokens: 1000,
            microcompact_after_turns: 5,
            compaction_threshold: 0.75,
        },
        state: ContextProjectionState::default(),
    });

    // Short context should not drop any messages
    assert_eq!(projection.report.dropped_messages, 0);

    // The tool result should be budgeted (previewed to Head max_chars 2000 + marker overhead)
    let tool_result_msg = projection
        .projected_messages
        .iter()
        .find_map(|m| match m {
            AgentMessage::ToolResult(tr) => Some(tr),
            _ => None,
        })
        .expect("projected messages should contain tool result");

    let result_text = tool_result_msg
        .content
        .iter()
        .filter_map(|c| match c {
            Content::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("");

    assert!(
        result_text.len() < 3000,
        "tool result should be budgeted, got {} chars",
        result_text.len()
    );
    assert!(
        result_text.contains("<context-artifact"),
        "budgeted result should contain preview marker"
    );

    // Token estimate should be positive
    assert!(projection.report.estimated_tokens > 0);
}

#[test]
fn tool_done_unknown_id_is_noop() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let t = idle.start_turn(AgentMessage::user("use tools"), vec![]);
    let streaming = t.state;

    let transition =
        streaming.finish_llm(LlmResult::Ok(assistant_with_tool_calls(vec![tool_call(
            "call-1", "read",
        )])));
    let (_events, _actions, runtime) = transition.into_parts();

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools");
    };

    // Submit result for a different tool call ID
    let transition = waiting.on_tool_done(
        ToolCallId::new("unknown-call"),
        Ok(ToolResult::text("result")),
    );
    let (_events, _actions, runtime) = transition.into_parts();

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
    let t = idle.start_turn(AgentMessage::user("edit file"), vec![]);
    let streaming = t.state;

    let transition =
        streaming.finish_llm(LlmResult::Ok(assistant_with_tool_calls(vec![tool_call(
            "call-1", "edit",
        )])));
    let (_events, _actions, runtime) = transition.into_parts();

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools");
    };

    // edit uses KeepFull strategy — a moderately large result should stay inline
    let large_result = "x".repeat(300);
    let transition = waiting.on_tool_done(
        ToolCallId::new("call-1"),
        Ok(ToolResult::text(large_result)),
    );
    let (_events, _actions, runtime) = transition.into_parts();

    let AgentRuntime::ReadyToContinue(ready) = runtime else {
        panic!("expected ReadyToContinue");
    };
    let t = ready.continue_turn();

    let context = match &t.actions[0] {
        AgentAction::StreamLlm { context, .. } => context.clone(),
        other => panic!("expected StreamLlm, got {other:?}"),
    };

    let projection = project(ProjectionInput {
        system_prompt: context.system_prompt.clone(),
        messages: context.messages.clone(),
        budget: ContextProjectionBudget {
            max_tool_result_chars: 100,
            max_context_tokens: 5000,
            microcompact_after_turns: 5,
            compaction_threshold: 0.75,
        },
        state: ContextProjectionState::default(),
    });

    let tool_result_msg = projection
        .projected_messages
        .iter()
        .find_map(|m| match m {
            AgentMessage::ToolResult(tr) => Some(tr),
            _ => None,
        })
        .expect("projected messages should contain tool result");

    let result_text = tool_result_msg
        .content
        .iter()
        .filter_map(|c| match c {
            Content::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("");

    // KeepFull means the full text is preserved inline, not replaced with a preview marker
    assert!(
        !result_text.contains("<context-artifact"),
        "KeepFull strategy should not insert preview marker, got: {result_text:.100}..."
    );
    assert!(
        result_text.len() >= 300,
        "KeepFull should preserve full result, got {} chars",
        result_text.len()
    );
}

#[test]
fn agent_runtime_delegation_exercise() {
    let mut runtime = AgentRuntime::new(dummy_options());

    // Idle — exercise state, session_state, set_session_state, on_tool_started, on_tool_update
    assert!(!runtime.state().is_streaming);
    let mut state = runtime.session_state().clone();
    state.name = "test-session".into();
    runtime.set_session_state(state);
    assert_eq!(runtime.session_state().name, "test-session");
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
    let t = idle.start_turn(AgentMessage::user("hello"), vec![]);
    runtime = t.state.into_runtime();
    assert!(runtime.state().is_streaming);
    assert!(runtime.on_tool_started(ToolCallId::new("x")).is_empty());

    // WaitingTools — on_tool_started should emit event for pending tool
    let AgentRuntime::Streaming(streaming) = runtime else {
        panic!("expected Streaming");
    };
    let transition =
        streaming.finish_llm(LlmResult::Ok(assistant_with_tool_calls(vec![tool_call(
            "call-1", "read",
        )])));
    runtime = transition.into_parts().2;
    assert!(!runtime.state().is_streaming);

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
    let transition = waiting.on_tool_done(ToolCallId::new("call-1"), Ok(ToolResult::text("ok")));
    runtime = transition.into_parts().2;
    assert!(matches!(runtime, AgentRuntime::ReadyToContinue(_)));
    runtime.state_mut().model.id = "changed".into();
    assert_eq!(runtime.state().model.id.as_str(), "changed");

    // Finished
    let AgentRuntime::ReadyToContinue(ready) = runtime else {
        panic!("expected ReadyToContinue");
    };
    let t = ready.continue_turn();
    let streaming = t.state;
    let transition = streaming.finish_llm(LlmResult::done());
    runtime = transition.into_parts().2;
    assert!(matches!(runtime, AgentRuntime::Finished(_)));
    assert!(!runtime.state().is_streaming);

    // Aborted
    let AgentRuntime::Finished(finished) = runtime else {
        panic!("expected Finished");
    };
    let t = finished.restart();
    runtime = t.state.into_runtime();
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let t = idle.start_turn(AgentMessage::user("hello"), vec![]);
    let streaming = t.state;
    let transition = streaming.abort();
    runtime = transition.state.into_runtime();
    assert!(matches!(runtime, AgentRuntime::Aborted(_)));
    assert!(!runtime.state().is_streaming);
}

#[test]
fn abort_during_streaming_clears_queues_and_emits_agent_end() {
    let mut runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let t = idle.start_turn(AgentMessage::user("hello"), vec![]);
    let mut streaming = t.state;

    // Feed a partial chunk so messages contain a streaming assistant
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

    let transition = streaming.abort();
    let events = transition.events;
    runtime = transition.state.into_runtime();

    assert!(matches!(runtime, AgentRuntime::Aborted(_)));
    assert!(!runtime.state().is_streaming);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AgentEvent::AgentEnd { .. })),
        "abort should emit AgentEnd"
    );
    // Partial message remains in transcript
    assert_eq!(
        runtime.state().messages.len(),
        2,
        "user + partial assistant"
    );
}

#[test]
fn abort_from_waiting_tools_clears_pending_tools() {
    let mut runtime = AgentRuntime::new(dummy_options());

    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let t = idle.start_turn(AgentMessage::user("run tool"), vec![]);
    let streaming = t.state;

    let assistant = assistant_with_tool_calls(vec![tool_call("tc-1", "test_tool")]);
    let transition = streaming.finish_llm(LlmResult::Ok(assistant));
    runtime = transition.into_parts().2;
    assert!(matches!(runtime, AgentRuntime::WaitingTools(_)));
    assert_eq!(runtime.state().pending_tool_calls.len(), 1);

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools");
    };
    let transition = waiting.abort();
    runtime = transition.state.into_runtime();

    assert!(matches!(runtime, AgentRuntime::Aborted(_)));
    assert!(
        runtime.state().pending_tool_calls.is_empty(),
        "abort should clear pending tool calls"
    );
    assert!(!runtime.state().is_streaming);
    assert!(
        transition
            .events
            .iter()
            .any(|e| matches!(e, AgentEvent::AgentEnd { .. })),
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
    let mut runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let t = idle.start_turn(AgentMessage::user("hello"), vec![]);
    let streaming = t.state;

    // Finish LLM with a tool call so we land in WaitingTools
    let transition =
        streaming.finish_llm(LlmResult::Ok(assistant_with_tool_calls(vec![tool_call(
            "tc-1",
            "test_tool",
        )])));
    runtime = transition.into_parts().2;
    assert!(matches!(runtime, AgentRuntime::WaitingTools(_)));

    // Complete the tool to reach ReadyToContinue
    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools");
    };
    let transition = waiting.on_tool_done(ToolCallId::new("tc-1"), Ok(ToolResult::text("ok")));
    runtime = transition.into_parts().2;
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
    let t = ready.continue_turn();
    let next_streaming = t.state;
    let transition = next_streaming.finish_llm(LlmResult::done());
    runtime = transition.into_parts().2;

    // After continuing, the steer message should have been drained into messages
    assert!(
        runtime.state().messages.iter().any(|m| {
            if let AgentMessage::User(u) = m {
                u.content.iter().any(|c| {
                    if let Content::Text(t) = c {
                        t.text.contains("steer while ready")
                    } else {
                        false
                    }
                })
            } else {
                false
            }
        }),
        "steer message should appear in transcript after continue_turn"
    );
}

#[test]
fn cancel_tool_removes_pending_and_emits_cancellation() {
    let mut runtime = AgentRuntime::new(dummy_options());

    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let t = idle.start_turn(AgentMessage::user("run tool"), vec![]);
    let streaming = t.state;

    let assistant = assistant_with_tool_calls(vec![tool_call("tc-1", "test_tool")]);
    let transition = streaming.finish_llm(LlmResult::Ok(assistant));
    runtime = transition.into_parts().2;
    assert!(matches!(runtime, AgentRuntime::WaitingTools(_)));
    assert_eq!(runtime.state().pending_tool_calls.len(), 1);

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools");
    };
    let transition = waiting.cancel_tool(
        ToolCallId::new("tc-1"),
        pi_core::CancelReason::UserRequested,
    );
    let (events, _actions, new_runtime) = transition.into_parts();

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
        parameters: JsonSchema::new(serde_json::json!({})),
        execution_mode: Default::default(),
        tool_run_mode: Default::default(),
    };
    let t = idle.start_turn(AgentMessage::user("read file"), vec![tool.clone()]);

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
    let mut runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let tool = ToolDefinition {
        name: ToolName::new("test_tool"),
        label: "Test".into(),
        description: "A test tool.".into(),
        parameters: JsonSchema::new(serde_json::json!({})),
        execution_mode: Default::default(),
        tool_run_mode: Default::default(),
    };
    let t = idle.start_turn(AgentMessage::user("use tool"), vec![tool.clone()]);
    let streaming = t.state;

    let assistant = assistant_with_tool_calls(vec![tool_call("tc-1", "test_tool")]);
    let transition = streaming.finish_llm(LlmResult::Ok(assistant));
    runtime = transition.into_parts().2;
    assert!(matches!(runtime, AgentRuntime::WaitingTools(_)));

    let AgentRuntime::WaitingTools(waiting) = runtime else {
        panic!("expected WaitingTools");
    };
    let transition = waiting.on_tool_done(ToolCallId::new("tc-1"), Ok(ToolResult::text("ok")));
    runtime = transition.into_parts().2;
    assert!(matches!(runtime, AgentRuntime::ReadyToContinue(_)));

    let AgentRuntime::ReadyToContinue(ready) = runtime else {
        panic!("expected ReadyToContinue");
    };
    let t = ready.continue_turn();
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
    let mut runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let tool = ToolDefinition {
        name: ToolName::new("test_tool"),
        label: "Test".into(),
        description: "A test tool.".into(),
        parameters: JsonSchema::new(serde_json::json!({})),
        execution_mode: Default::default(),
        tool_run_mode: Default::default(),
    };
    let t = idle.start_turn(AgentMessage::user("use tool"), vec![tool]);
    let streaming = t.state;

    let transition = streaming.abort();
    runtime = transition.state.into_runtime();
    assert!(matches!(runtime, AgentRuntime::Aborted(_)));

    let AgentRuntime::Aborted(aborted) = runtime else {
        panic!("expected Aborted");
    };
    let transition = aborted.restart();
    let idle = transition.state;
    let t = idle.start_turn(AgentMessage::user("hello again"), vec![]);
    let context = match &t.actions[0] {
        AgentAction::StreamLlm { context, .. } => context,
        other => panic!("expected StreamLlm, got {other:?}"),
    };
    assert!(context.tools.is_empty(), "abort should clear turn_tools");
}

#[test]
fn reset_clears_current_turn_tools() {
    let mut runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let tool = ToolDefinition {
        name: ToolName::new("test_tool"),
        label: "Test".into(),
        description: "A test tool.".into(),
        parameters: JsonSchema::new(serde_json::json!({})),
        execution_mode: Default::default(),
        tool_run_mode: Default::default(),
    };
    let t = idle.start_turn(AgentMessage::user("use tool"), vec![tool]);
    runtime = t.state.into_runtime();

    runtime = runtime.reset();
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let t = idle.start_turn(AgentMessage::user("hello again"), vec![]);
    let context = match &t.actions[0] {
        AgentAction::StreamLlm { context, .. } => context,
        other => panic!("expected StreamLlm, got {other:?}"),
    };
    assert!(context.tools.is_empty(), "reset should clear turn_tools");
}
