use std::collections::BTreeMap;

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

impl ToolResultMessage {
    pub fn with_content(&self, new_content: Vec<Content>) -> Self {
        Self {
            role: self.role.clone(),
            tool_call_id: self.tool_call_id.clone(),
            tool_name: self.tool_name.clone(),
            content: new_content,
            details: self.details.clone(),
            is_error: self.is_error,
            timestamp: self.timestamp,
        }
    }
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

/// A message in the trimmed context (T).
///
/// T is the already-projected message sequence ready for LLM consumption.
/// Projection happens once at turn end; old messages are never re-projected.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TrimmedMessage {
    User(UserMessage),
    Assistant(AssistantMessage),
    /// Full original tool result, not yet projected.
    /// May be projected to ProjectedTool in a future turn (defer).
    OriginalTool(OriginalToolResult),
    /// Projected tool result. Only preview + artifact reference.
    /// One-way: once ProjectedTool, never reverts to OriginalTool.
    ProjectedTool(ProjectedToolResult),
    /// Compaction summary replacing older messages.
    Compaction(CompactionSummary),
}

/// Original (unprojected) tool result in T.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OriginalToolResult {
    pub entry_id: String,
    pub tool_call_id: ToolCallId,
    pub tool_name: ToolName,
    pub content: Vec<Content>,
    pub is_error: bool,
    /// Turn number when this result was created (for age-based defer).
    pub turn: u32,
}

impl OriginalToolResult {
    pub fn content_char_count(&self) -> usize {
        self.content
            .iter()
            .filter_map(|c| match c {
                Content::Text(t) => Some(t.text.chars().count()),
                _ => None,
            })
            .sum()
    }

    pub fn preview(&self, max_chars: usize) -> String {
        let text: String = self
            .content
            .iter()
            .filter_map(|c| match c {
                Content::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        if text.chars().count() <= max_chars {
            text
        } else {
            text.chars().take(max_chars).collect()
        }
    }
}

/// Projected tool result in T. Only preview text + artifact reference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectedToolResult {
    pub entry_id: String,
    pub tool_call_id: ToolCallId,
    pub tool_name: ToolName,
    pub preview: String,
    /// Key into A where the full original content lives.
    pub artifact_id: String,
    pub original_char_count: usize,
    pub is_error: bool,
}

/// Compaction summary replacing older messages in T.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompactionSummary {
    pub summary: String,
    pub compacted_entry_ids: Vec<String>,
    pub tokens_before: usize,
}

/// Artifacts map: entry_id → original tool result that was projected away.
/// A is the complement of T: T ∪ A = complete conversation.
pub type Artifacts = BTreeMap<String, OriginalToolResult>;

fn current_timestamp() -> u64 {
    crate::timestamp::current_timestamp()
}
