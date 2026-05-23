use serde::{Deserialize, Serialize};

use pi_core::Model;

/// Options for an LLM streaming request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct StreamOptions {
    pub api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<std::collections::HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Events from an LLM stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LlmEvent {
    Start {
        model: String,
    },
    TextDelta {
        text: String,
    },
    ThinkingDelta {
        text: String,
    },
    ToolCallStart {
        tool_call_id: String,
        name: String,
    },
    ToolCallDelta {
        tool_call_id: String,
        delta: serde_json::Value,
    },
    ToolCallEnd {
        tool_call_id: String,
    },
    Usage {
        input: u32,
        output: u32,
    },
    Done,
    Error {
        message: String,
    },
}

/// Result of a complete stream.
pub struct LlmStream {
    pub model: Model,
    pub events: Vec<LlmEvent>,
}

/// Error during streaming.
#[derive(Debug, Clone, PartialEq, thiserror::Error, Serialize, Deserialize)]
pub enum StreamError {
    #[error("network error: {0}")]
    Network(String),
    #[error("auth error: {0}")]
    Auth(String),
    #[error("rate limited")]
    RateLimited,
    #[error("model error: {0}")]
    Model(String),
    #[error("aborted")]
    Aborted,
}
