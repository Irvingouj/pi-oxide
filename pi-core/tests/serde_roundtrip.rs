//! JSON serde roundtrip tests for message and event types.

use pi_core::{
    AgentAction, AgentEvent, AgentMessage, CancelReason, Content, LlmContext, StopReason,
    TextContent, ToolCall, ToolCallId, ToolName, ToolArguments, ToolResultMessage,
    UserMessage, AssistantMessage,
};

#[test]
fn agent_message_user_roundtrip() {
    let msg = AgentMessage::User(UserMessage::new_text("hello"));
    let json = serde_json::to_string(&msg).unwrap();
    let back: AgentMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(msg, back);
}

#[test]
fn agent_message_assistant_roundtrip() {
    let msg = AgentMessage::Assistant(AssistantMessage {
        content: vec![Content::Text(TextContent {
            text: "response".into(),
        })],
        api: "test".into(),
        provider: "test".into(),
        model: "test-model".into(),
        stop_reason: StopReason::EndTurn,
        error_message: None,
        timestamp: 1,
        usage: Default::default(),
    });
    let json = serde_json::to_string(&msg).unwrap();
    let back: AgentMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(msg, back);
}

#[test]
fn tool_result_message_roundtrip() {
    let msg = ToolResultMessage {
        role: "tool_result".into(),
        tool_call_id: ToolCallId::new("tc-1"),
        tool_name: ToolName::new("read"),
        content: vec![Content::Text(TextContent {
            text: "file contents".into(),
        })],
        details: None,
        is_error: false,
        timestamp: 2,
    };
    let json = serde_json::to_string(&msg).unwrap();
    let back: ToolResultMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(msg, back);
}

#[test]
fn agent_event_turn_end_roundtrip() {
    let event = AgentEvent::TurnEnd {
        message: AgentMessage::user("done"),
        tool_results: vec![],
    };
    let json = serde_json::to_string(&event).unwrap();
    let back: AgentEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(event, back);
}

#[test]
fn agent_event_tool_execution_start_roundtrip() {
    let event = AgentEvent::ToolExecutionStart {
        tool_call_id: ToolCallId::new("tc-1"),
        tool_name: ToolName::new("bash"),
        args: Some(ToolArguments::new(serde_json::json!({"cmd": "ls"}))),
    };
    let json = serde_json::to_string(&event).unwrap();
    let back: AgentEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(event, back);
}

#[test]
fn agent_event_queue_update_roundtrip() {
    let event = AgentEvent::QueueUpdate {
        steer: vec![AgentMessage::user("steer")],
        follow_up: vec![],
    };
    let json = serde_json::to_string(&event).unwrap();
    let back: AgentEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(event, back);
}

#[test]
fn agent_action_stream_llm_roundtrip() {
    let action = AgentAction::StreamLlm {
        context: LlmContext {
            system_prompt: "You are helpful.".into(),
            messages: vec![AgentMessage::user("hi")],
            tools: vec![],
        },
        session_id: None,
    };
    let json = serde_json::to_string(&action).unwrap();
    let back: AgentAction = serde_json::from_str(&json).unwrap();
    assert_eq!(action, back);
}

#[test]
fn agent_action_prepare_tool_calls_roundtrip() {
    let action = AgentAction::PrepareToolCalls {
        calls: vec![ToolCall {
            id: ToolCallId::new("tc-1"),
            name: ToolName::new("read"),
            arguments: ToolArguments::new(serde_json::json!({})),
        }],
    };
    let json = serde_json::to_string(&action).unwrap();
    let back: AgentAction = serde_json::from_str(&json).unwrap();
    assert_eq!(action, back);
}

#[test]
fn agent_action_finished_roundtrip() {
    let action = AgentAction::Finished;
    let json = serde_json::to_string(&action).unwrap();
    let back: AgentAction = serde_json::from_str(&json).unwrap();
    assert_eq!(action, back);
}

#[test]
fn cancel_reason_dependency_failed_roundtrip() {
    let reason = CancelReason::DependencyFailed {
        cause_tool_call_id: ToolCallId::new("tc-0"),
    };
    let json = serde_json::to_string(&reason).unwrap();
    let back: CancelReason = serde_json::from_str(&json).unwrap();
    assert_eq!(reason, back);
}
