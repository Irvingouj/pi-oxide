pub use pi_core::{
    AgentAction, AgentEvent, AgentMessage, AgentOptions, AgentRuntime, Artifacts, AssistantMessage,
    ChangeMarker, Content, ContentDelta, ContextProjectionBudget, ContinueTurnTransition, LlmChunk,
    LlmResult, Model, StartTurnTransition, StopReason, TextContent, ToolArguments, ToolCall,
    ToolCallId, ToolCallPermission, ToolCallPreparation, ToolCallTransform, ToolDefinition,
    ToolExecutionUpdate, ToolName, ToolResult, TrimmedMessage,
};

pub fn dummy_model() -> Model {
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

pub fn dummy_options() -> AgentOptions {
    AgentOptions {
        system_prompt: "You are a test agent.".to_string(),
        model: dummy_model(),
        thinking_level: pi_core::ThinkingLevel::Off,
        steering_mode: pi_core::QueueMode::OneAtATime,
        follow_up_mode: pi_core::QueueMode::OneAtATime,
        tool_execution_mode: pi_core::ExecutionMode::Parallel,
        session_id: None,
    }
}

pub fn assistant_with_tool_calls(calls: Vec<ToolCall>) -> AssistantMessage {
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

pub fn tool_call(id: &str, name: &str) -> ToolCall {
    ToolCall {
        id: ToolCallId::new(id),
        name: ToolName::new(name),
        arguments: ToolArguments::new(serde_json::json!({})),
    }
}

/// Default empty T, A, turn_number for tests.
pub fn empty() -> (Vec<TrimmedMessage>, Artifacts, u32) {
    (vec![], Artifacts::new(), 0)
}
