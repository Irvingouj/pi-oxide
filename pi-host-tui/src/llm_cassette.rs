//! Cassette format for LLM recording/replay.
//!
//! Shared serialization types used by both the `record` and `replay` features.
//!
//! ## Relationship to `pi-record-server`
//!
//! This cassette system operates at the typed `pi_core` layer — it records
//! `pi_core::LlmChunk` values produced by the SSE parser. It is used
//! in-process via feature flags (`--features record`, `--features replay`).
//!
//! The separate `pi-record-server` crate operates at the raw HTTP layer —
//! it records raw SSE bytes (base64-encoded) without parsing them. It is
//! used as a standalone proxy process (no feature flags, no recompilation).
//!
//! Use `pi-record-server` for integration tests that need HTTP-level fidelity
//! (e.g., catching wire-format bugs). Use the feature-gated system for
//! fast in-process unit tests that don't need a separate server process.

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
