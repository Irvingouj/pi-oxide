//! Recording LLM client — wraps a real LlmClient and captures every call to a cassette.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use pi_core::LlmChunk;
use tracing::{debug, info};

use crate::llm::{CollectedToolCall, LlmClient, LlmProvider, LlmStreamState};
use crate::llm_cassette::{Cassette, CassetteEntry, CassetteRequest, CassetteResponse};

pub struct RecordingLlmClient {
    inner: LlmClient,
    cassette: Arc<Mutex<Cassette>>,
    output_path: PathBuf,
}

impl RecordingLlmClient {
    pub fn new(
        api_key: &str,
        base_url: &str,
        model: &str,
        wire_format: crate::llm::WireFormat,
        output_path: PathBuf,
    ) -> Self {
        info!(path = %output_path.display(), "LLM recording enabled");
        Self {
            inner: LlmClient::new(api_key, base_url, model, wire_format),
            cassette: Arc::new(Mutex::new(Cassette {
                version: 1,
                model: model.to_string(),
                entries: Vec::new(),
            })),
            output_path,
        }
    }
}

impl LlmProvider for RecordingLlmClient {
    type Stream = RecordingLlmStream;

    fn stream_sync(
        &self,
        system_prompt: &str,
        messages: &[pi_core::AgentMessage],
        tools: &[pi_core::ToolDefinition],
    ) -> Result<RecordingLlmStream, Box<dyn std::error::Error>> {
        let inner_stream = self.inner.stream_sync(system_prompt, messages, tools)?;
        debug!(
            messages = messages.len(),
            tools = tools.len(),
            "recording LLM call"
        );

        Ok(RecordingLlmStream {
            inner: inner_stream,
            request: CassetteRequest {
                system_prompt: system_prompt.to_string(),
                messages: messages.to_vec(),
                tools: tools.to_vec(),
            },
            chunks: Vec::new(),
            cassette: self.cassette.clone(),
        })
    }

    fn model_id(&self) -> &str {
        self.inner.model_id()
    }

    fn set_model(&mut self, model: &str) {
        self.inner.set_model(model);
    }
}

impl Drop for RecordingLlmClient {
    fn drop(&mut self) {
        let cassette = self.cassette.lock().unwrap();
        match serde_json::to_string_pretty(&*cassette) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&self.output_path, json) {
                    tracing::error!(path = %self.output_path.display(), error = %e, "failed to write cassette");
                } else {
                    info!(path = %self.output_path.display(), entries = cassette.entries.len(), "cassette written");
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "failed to serialize cassette");
            }
        }
    }
}

pub struct RecordingLlmStream {
    inner: crate::llm::LlmStream,
    request: CassetteRequest,
    chunks: Vec<LlmChunk>,
    cassette: Arc<Mutex<Cassette>>,
}

impl Iterator for RecordingLlmStream {
    type Item = LlmChunk;

    fn next(&mut self) -> Option<Self::Item> {
        let chunk = self.inner.next()?;
        self.chunks.push(chunk.clone());

        if matches!(chunk, LlmChunk::Done | LlmChunk::Error { .. }) {
            self.finalize_entry();
        }
        Some(chunk)
    }
}

impl RecordingLlmStream {
    fn finalize_entry(&mut self) {
        let usage = crate::llm::LlmStream::usage(&self.inner);
        let stop_reason = crate::llm::LlmStream::stop_reason(&self.inner).map(|s| s.to_string());
        let tool_calls = crate::llm::LlmStream::tool_calls(&self.inner);

        let entry = CassetteEntry {
            request: std::mem::replace(
                &mut self.request,
                CassetteRequest {
                    system_prompt: String::new(),
                    messages: Vec::new(),
                    tools: Vec::new(),
                },
            ),
            response: CassetteResponse {
                chunks: std::mem::take(&mut self.chunks),
                usage,
                stop_reason,
                tool_calls,
            },
        };

        self.cassette.lock().unwrap().entries.push(entry);
    }
}

impl LlmStreamState for RecordingLlmStream {
    fn usage(&self) -> Option<(u32, u32, u32)> {
        crate::llm::LlmStream::usage(&self.inner)
    }
    fn stop_reason(&self) -> Option<&str> {
        crate::llm::LlmStream::stop_reason(&self.inner)
    }
    fn tool_calls(&self) -> Vec<CollectedToolCall> {
        crate::llm::LlmStream::tool_calls(&self.inner)
    }
}
