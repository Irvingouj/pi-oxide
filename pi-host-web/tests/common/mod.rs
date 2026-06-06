pub use pi_host_web::dto::*;
pub use pi_host_web::host_agent_api::{
    create_host_agent, destroy_host_agent, get_host_agent_persist_data, host_accept_compaction,
    host_continue_turn, host_llm_done, host_prepare_tool_calls, host_steer, host_tool_cancelled,
    host_tool_done, restore_host_agent, start_turn,
};
pub use pi_host_web::host_state_api::{
    destroy_host_state, estimate_tokens_export, estimate_tokens_for_text_export,
    get_host_state_persist_data, restore_host_state, restore_host_state_from_json,
};

pub fn dummy_options() -> AgentOptions {
    AgentOptions {
        system_prompt: "test agent".to_string(),
        model: Model {
            id: ModelId("test-model".to_string()),
            name: ModelName("Test".to_string()),
            api: ApiName("test".to_string()),
            provider: ProviderName("test".to_string()),
            base_url: None,
            reasoning: false,
            context_window: 4096,
            max_tokens: 1024,
            capabilities: Default::default(),
            cost: Default::default(),
        },
        thinking_level: Default::default(),
        steering_mode: Default::default(),
        follow_up_mode: Default::default(),
        tool_execution_mode: Default::default(),
        session_id: None,
    }
}

pub fn default_budget() -> ContextProjectionBudget {
    ContextProjectionBudget {
        max_tool_result_chars: 50000,
        max_context_tokens: 100000,
        microcompact_after_turns: 5,
        compaction_threshold: 0.75,
    }
}

pub fn make_tool_def(name: &str) -> ToolDefinition {
    ToolDefinition {
        name: ToolName(name.to_string()),
        label: "Test".to_string(),
        description: "A test tool.".to_string(),
        parameters: JsonSchema(serde_json::json!({})),
        execution_mode: Default::default(),
        tool_run_mode: Default::default(),
    }
}

pub fn make_user_prompt(text: &str) -> AgentMessage {
    AgentMessage::User(UserMessage {
        content: vec![Content::Text(TextContent {
            text: text.to_string(),
        })],
        timestamp: 1,
    })
}

pub fn make_assistant_text(text: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![Content::Text(TextContent {
            text: text.to_string(),
        })],
        api: ApiName("test".to_string()),
        provider: ProviderName("test".to_string()),
        model: ModelId("test".to_string()),
        stop_reason: StopReason::EndTurn,
        error_message: None,
        timestamp: 1,
        usage: TokenUsage::default(),
    }
}

pub fn make_assistant_with_tool(name: &str, id: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![Content::ToolCall(ToolCall {
            id: ToolCallId(id.to_string()),
            name: ToolName(name.to_string()),
            arguments: ToolArguments(serde_json::json!({})),
        })],
        api: ApiName("test".to_string()),
        provider: ProviderName("test".to_string()),
        model: ModelId("test".to_string()),
        stop_reason: StopReason::ToolUse,
        error_message: None,
        timestamp: 1,
        usage: TokenUsage::default(),
    }
}
