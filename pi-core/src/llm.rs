use serde::{Deserialize, Serialize};

use crate::message::{AssistantMessage, StopReason};
use crate::types::{ApiName, ModelId, ModelName, ProviderName, ToolCallId};

/// Describes a concrete LLM model and its provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub id: ModelId,
    pub name: ModelName,
    pub api: ApiName,
    pub provider: ProviderName,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    pub reasoning: bool,
    pub context_window: u32,
    pub max_tokens: u32,
    #[serde(default)]
    pub capabilities: ModelCapabilities,
    #[serde(default)]
    pub cost: ModelCost,
}

/// Capabilities advertised by a model.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct ModelCapabilities {
    pub vision: bool,
    pub json_mode: bool,
    pub function_calling: bool,
    pub streaming: bool,
}

/// Per-token cost estimate.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct ModelCost {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

/// Supported LLM providers.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelProvider {
    OpenAi,
    Anthropic,
    Google,
    Ollama,
    Custom,
}

/// A chunk from an LLM streaming response.
/// Note: Not exported to TS — uses serde(flatten) which ts-rs can't handle,
/// and this type is only used internally by the host-side streaming pipeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LlmChunk {
    Start {
        #[serde(flatten)]
        partial: AssistantMessage,
    },
    TextDelta {
        text: String,
    },
    ThinkingDelta {
        text: String,
    },
    ToolCallDelta {
        tool_call_id: ToolCallId,
        delta: serde_json::Value,
    },
    Done,
    Error {
        message: String,
    },
}

/// Final result of an LLM stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LlmResult {
    Ok(AssistantMessage),
    Err { error: LlmError, aborted: bool },
}

impl LlmResult {
    pub fn finalize_message(self) -> AssistantMessage {
        match self {
            LlmResult::Ok(msg) => msg,
            LlmResult::Err { error, aborted } => {
                let mut msg = AssistantMessage::empty();
                msg.stop_reason = if aborted {
                    StopReason::Aborted
                } else {
                    StopReason::Error
                };
                msg.error_message = Some(error.message);
                msg
            }
        }
    }

    pub fn done() -> Self {
        LlmResult::Ok(AssistantMessage::empty())
    }
}

/// Error from the LLM provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[error("llm error: {message}")]
pub struct LlmError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}
