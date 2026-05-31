//! Replay LLM client — loads a recorded cassette and replays canned responses.

use std::cell::Cell;
use std::path::Path;

use pi_core::LlmChunk;
use tracing::{debug, info, warn};

use crate::llm::{CollectedToolCall, LlmProvider, LlmStreamState};
use crate::llm_cassette::{Cassette, CassetteResponse};

pub struct ReplayLlmClient {
    model: String,
    entries: Vec<CassetteResponse>,
    call_index: Cell<usize>,
}

impl ReplayLlmClient {
    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let json = std::fs::read_to_string(path)?;
        let cassette: Cassette = serde_json::from_str(&json)?;
        info!(entries = cassette.entries.len(), model = %cassette.model, "loaded cassette for replay");
        let model = cassette.model.clone();
        let entries: Vec<CassetteResponse> =
            cassette.entries.into_iter().map(|e| e.response).collect();
        Ok(Self {
            model,
            entries,
            call_index: Cell::new(0),
        })
    }
}

impl LlmProvider for ReplayLlmClient {
    type Stream = ReplayLlmStream;

    fn stream_sync(
        &self,
        system_prompt: &str,
        messages: &[pi_core::AgentMessage],
        tools: &[pi_core::ToolDefinition],
    ) -> Result<ReplayLlmStream, Box<dyn std::error::Error>> {
        let idx = self.call_index.get();
        let response = self.entries.get(idx).ok_or_else(|| {
            format!(
                "cassette exhausted: call {} but only {} entries recorded",
                idx,
                self.entries.len()
            )
        })?;

        debug!(
            call = idx,
            messages = messages.len(),
            tools = tools.len(),
            chunks = response.chunks.len(),
            "replaying LLM call"
        );

        self.call_index.set(idx + 1);

        Ok(ReplayLlmStream {
            chunks: response.chunks.clone(),
            index: Cell::new(0),
            usage: response.usage,
            stop_reason: response.stop_reason.clone(),
            tool_calls: response.tool_calls.clone(),
        })
    }

    fn model_id(&self) -> &str {
        &self.model
    }

    fn set_model(&mut self, _model: &str) {
        warn!("set_model called during replay — ignored");
    }
}

pub struct ReplayLlmStream {
    chunks: Vec<LlmChunk>,
    index: Cell<usize>,
    usage: Option<(u32, u32, u32)>,
    stop_reason: Option<String>,
    tool_calls: Vec<CollectedToolCall>,
}

impl Iterator for ReplayLlmStream {
    type Item = LlmChunk;

    fn next(&mut self) -> Option<Self::Item> {
        let idx = self.index.get();
        if idx >= self.chunks.len() {
            return None;
        }
        let chunk = self.chunks[idx].clone();
        self.index.set(idx + 1);
        Some(chunk)
    }
}

impl LlmStreamState for ReplayLlmStream {
    fn usage(&self) -> Option<(u32, u32, u32)> {
        self.usage
    }
    fn stop_reason(&self) -> Option<&str> {
        self.stop_reason.as_deref()
    }
    fn tool_calls(&self) -> Vec<CollectedToolCall> {
        self.tool_calls.clone()
    }
}
