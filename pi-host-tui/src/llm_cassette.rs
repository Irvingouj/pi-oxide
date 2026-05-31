//! Cassette format for LLM recording/replay.
//!
//! Shared serialization types used by both the `record` and `replay` features.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cassette {
    pub version: u32,
    pub model: String,
    pub entries: Vec<CassetteEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CassetteEntry {
    pub request: CassetteRequest,
    pub response: CassetteResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CassetteRequest {
    pub system_prompt: String,
    pub messages: Vec<pi_core::AgentMessage>,
    pub tools: Vec<pi_core::ToolDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CassetteResponse {
    pub chunks: Vec<pi_core::LlmChunk>,
    pub usage: Option<(u32, u32, u32)>,
    pub stop_reason: Option<String>,
    pub tool_calls: Vec<super::llm::CollectedToolCall>,
}
