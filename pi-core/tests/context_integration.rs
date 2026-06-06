#![allow(non_snake_case)]

mod common;
use common::*;

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


