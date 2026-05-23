use serde::{Deserialize, Serialize};

use crate::types::{
    ApiName, ModelId, ProviderName, ToolArguments, ToolCallId, ToolDetails, ToolName,
};

/// A user message sent to the agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserMessage {
    pub content: Vec<Content>,
    pub timestamp: u64,
}

impl UserMessage {
    pub fn new_text(text: impl Into<String>) -> Self {
        Self {
            content: vec![Content::Text(TextContent { text: text.into() })],
            timestamp: current_timestamp(),
        }
    }

    pub fn with_image(mut self, image: ImageContent) -> Self {
        self.content.push(Content::Image(image));
        self
    }
}

/// An assistant message produced by the LLM.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantMessage {
    pub content: Vec<Content>,
    pub api: ApiName,
    pub provider: ProviderName,
    pub model: ModelId,
    pub stop_reason: StopReason,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub timestamp: u64,
    pub usage: TokenUsage,
}

impl AssistantMessage {
    pub fn empty() -> Self {
        Self {
            content: vec![Content::Text(TextContent {
                text: String::new(),
            })],
            api: ApiName::new(""),
            provider: ProviderName::new(""),
            model: ModelId::new(""),
            stop_reason: StopReason::EndTurn,
            error_message: None,
            timestamp: current_timestamp(),
            usage: TokenUsage::default(),
        }
    }
}

/// A message carrying the result of a tool execution back to the LLM.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResultMessage {
    #[serde(skip_deserializing, default = "tool_result_role")]
    pub role: String,
    pub tool_call_id: ToolCallId,
    pub tool_name: ToolName,
    pub content: Vec<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<ToolDetails>,
    pub is_error: bool,
    pub timestamp: u64,
}

fn tool_result_role() -> String {
    "tool_result".to_string()
}

/// Union of all message types visible to the core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum AgentMessage {
    User(UserMessage),
    Assistant(AssistantMessage),
    ToolResult(ToolResultMessage),
}

impl AgentMessage {
    pub fn user(text: impl Into<String>) -> Self {
        Self::User(UserMessage::new_text(text))
    }

    pub fn assistant_text(text: impl Into<String>) -> Self {
        let mut msg = AssistantMessage::empty();
        msg.content = vec![Content::Text(TextContent { text: text.into() })];
        Self::Assistant(msg)
    }

    pub fn timestamp(&self) -> u64 {
        match self {
            AgentMessage::User(u) => u.timestamp,
            AgentMessage::Assistant(a) => a.timestamp,
            AgentMessage::ToolResult(t) => t.timestamp,
        }
    }
}

/// Content block within a message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Content {
    Text(TextContent),
    Image(ImageContent),
    ToolCall(ToolCall),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextContent {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageContent {
    pub media_type: String,
    pub data: String,
}

/// A tool call requested by the assistant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: ToolCallId,
    pub name: ToolName,
    pub arguments: ToolArguments,
}

/// Token usage statistics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct TokenUsage {
    pub input: u32,
    pub output: u32,
    pub cache_read: u32,
    pub cache_write: u32,
    pub total_tokens: u32,
}

/// Why an assistant response stopped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    ToolUse,
    Aborted,
    Error,
}

fn current_timestamp() -> u64 {
    crate::timestamp::current_timestamp()
}
