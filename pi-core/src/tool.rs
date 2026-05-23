use serde::{Deserialize, Serialize};

use crate::types::{JsonSchema, ToolDetails, ToolName};

/// Metadata describing a tool available to the agent.
/// Hosts implement the actual execution; core only holds the schema.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: ToolName,
    pub label: String,
    pub description: String,
    pub parameters: JsonSchema,
    #[serde(rename = "execution_mode", default)]
    pub execution_mode: ExecutionMode,
}

/// Whether a tool can run concurrently with other tools.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    #[default]
    Parallel,
    Sequential,
}

/// Controls how multiple tool calls from a single assistant message are handled.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolExecutionMode {
    #[default]
    Parallel,
    Sequential,
}

/// Result of a tool execution, returned by the host via `on_tool_done`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    pub content: Vec<crate::message::Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<ToolDetails>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminate: Option<bool>,
}

impl ToolResult {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![crate::message::Content::Text(crate::message::TextContent {
                text: text.into(),
            })],
            details: None,
            terminate: None,
        }
    }

    pub fn partial_text(text: impl Into<String>) -> Self {
        Self::text(text)
    }
}

/// Error produced during tool execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[error("tool error: {message}")]
pub struct ToolError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<ToolDetails>,
}

impl ToolError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }
}
