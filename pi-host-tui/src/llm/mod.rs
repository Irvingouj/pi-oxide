//! LLM streaming client supporting multiple provider wire formats.
//!
//! Synchronous streaming via reqwest::blocking. Parses SSE events
//! and provides an iterator of LlmChunk values with collected state.

mod discovery;
mod messages;
mod request;
mod stream;

#[cfg(test)]
mod tests;

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// Disambiguates the two wire protocols: Anthropic Messages vs OpenAI Chat Completions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WireFormat {
    #[default]
    Anthropic,
    OpenAI,
}

#[derive(Clone)]
pub struct LlmClient {
    client: reqwest::blocking::Client,
    api_key: String,
    base_url: String,
    model: String,
    wire_format: WireFormat,
}

/// Streaming response from an LLM provider.
///
/// Iterate via `for chunk in stream.by_ref()`, then call
/// `.usage()`, `.stop_reason()`, `.tool_calls()` for collected state.
pub struct LlmStream {
    reader: reqwest::blocking::Response,
    buffer: String,
    wire_format: WireFormat,
    // Collected state (accumulated during iteration)
    tool_calls: Vec<PartialToolCall>,
    stop_reason: Option<String>,
    usage_input: Option<u32>,
    usage_output: Option<u32>,
    done: bool,
}

/// A partial tool call being accumulated from the stream.
struct PartialToolCall {
    id: String,
    name: String,
    input_json: String,
}

impl LlmStream {
    pub fn usage(&self) -> Option<(u32, u32, u32)> {
        match (self.usage_input, self.usage_output) {
            (Some(i), Some(o)) => Some((i, o, i + o)),
            _ => None,
        }
    }

    pub fn stop_reason(&self) -> Option<&str> {
        self.stop_reason.as_deref()
    }

    pub fn tool_calls(&self) -> Vec<CollectedToolCall> {
        self.tool_calls
            .iter()
            .map(|tc| {
                let input: serde_json::Value = serde_json::from_str(&tc.input_json).unwrap_or_else(|e| {
                    tracing::warn!(tool_call_id = tc.id.as_str(), error = %e, "malformed tool input JSON, using empty object");
                    serde_json::Value::Object(Default::default())
                });
                CollectedToolCall {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    input,
                }
            })
            .collect()
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct CollectedToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Model discovery types
// ---------------------------------------------------------------------------

/// Minimal model info returned by discovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelInfo {
    pub id: String,
}

/// Runtime model discovery — fetches available models from a provider.
///
/// Deep module: one method, hides HTTP / wire-format details inside.
pub trait ModelDiscovery {
    fn list_models(&self) -> Result<Vec<ModelInfo>, Box<dyn std::error::Error>>;
}

// ---------------------------------------------------------------------------
// LlmClient — constructor, accessor, stream_sync
// ---------------------------------------------------------------------------

impl LlmClient {
    pub fn new(api_key: &str, base_url: &str, model: &str, wire_format: WireFormat) -> Self {
        Self {
            client: reqwest::blocking::Client::new(),
            api_key: api_key.to_string(),
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
            wire_format,
        }
    }

    pub fn model_id(&self) -> &str {
        &self.model
    }

    pub fn set_model(&mut self, model: &str) {
        self.model = model.to_string();
    }

    /// Start a streaming LLM request. Returns an iterator of LlmChunk values.
    pub fn stream_sync(
        &self,
        system_prompt: &str,
        messages: &[pi_core::AgentMessage],
        tools: &[pi_core::ToolDefinition],
    ) -> Result<LlmStream, Box<dyn std::error::Error>> {
        let url = if self.wire_format == WireFormat::OpenAI {
            format!("{}/v1/chat/completions", self.base_url)
        } else {
            format!("{}/v1/messages", self.base_url)
        };
        let body = self.build_body(system_prompt, messages, tools);

        tracing::debug!(%url, model = %self.model, messages = messages.len(), tools = tools.len(), "POST");

        let mut req = self
            .client
            .post(&url)
            .header("content-type", "application/json");

        match self.wire_format {
            WireFormat::Anthropic => {
                req = req
                    .header("x-api-key", &self.api_key)
                    .header("anthropic-version", "2023-06-01");
            }
            WireFormat::OpenAI => {
                req = req.header("authorization", format!("Bearer {}", self.api_key));
            }
        }

        let resp = req.json(&body).send()?;

        tracing::debug!(status = %resp.status(), "API response");

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            tracing::error!(status = %status, url = %url, model = %self.model, body = %text, "Non-2xx API response");
            return Err(format!("API error {status}: {text}").into());
        }

        Ok(LlmStream {
            reader: resp,
            buffer: String::with_capacity(8192),
            wire_format: self.wire_format,
            tool_calls: Vec::new(),
            stop_reason: None,
            usage_input: None,
            usage_output: None,
            done: false,
        })
    }
}

// ---------------------------------------------------------------------------
// Provider trait + feature-gated backend
// ---------------------------------------------------------------------------

/// Post-iteration accessors shared by live, recording, and replay streams.
#[allow(dead_code)]
pub trait LlmStreamState {
    fn usage(&self) -> Option<(u32, u32, u32)>;
    fn stop_reason(&self) -> Option<&str>;
    fn tool_calls(&self) -> Vec<CollectedToolCall>;
}

#[allow(dead_code)]
impl LlmStreamState for LlmStream {
    fn usage(&self) -> Option<(u32, u32, u32)> {
        LlmStream::usage(self)
    }
    fn stop_reason(&self) -> Option<&str> {
        LlmStream::stop_reason(self)
    }
    fn tool_calls(&self) -> Vec<CollectedToolCall> {
        LlmStream::tool_calls(self)
    }
}

/// The interface the TUI uses to talk to an LLM provider.
#[allow(dead_code)]
pub trait LlmProvider: Sized {
    type Stream: Iterator<Item = pi_core::LlmChunk> + LlmStreamState;

    fn stream_sync(
        &self,
        system_prompt: &str,
        messages: &[pi_core::AgentMessage],
        tools: &[pi_core::ToolDefinition],
    ) -> Result<Self::Stream, Box<dyn std::error::Error>>;

    fn model_id(&self) -> &str;
    fn set_model(&mut self, model: &str);
}

#[allow(dead_code)]
impl LlmProvider for LlmClient {
    type Stream = LlmStream;

    fn stream_sync(
        &self,
        system_prompt: &str,
        messages: &[pi_core::AgentMessage],
        tools: &[pi_core::ToolDefinition],
    ) -> Result<LlmStream, Box<dyn std::error::Error>> {
        LlmClient::stream_sync(self, system_prompt, messages, tools)
    }

    fn model_id(&self) -> &str {
        LlmClient::model_id(self)
    }

    fn set_model(&mut self, model: &str) {
        LlmClient::set_model(self, model);
    }
}

// Feature-gated backend type alias.
#[cfg(not(any(feature = "record", feature = "replay")))]
pub type LlmBackend = LlmClient;

#[cfg(feature = "record")]
pub type LlmBackend = crate::llm_record::RecordingLlmClient;

#[cfg(all(feature = "replay", not(feature = "record")))]
pub type LlmBackend = crate::llm_replay::ReplayLlmClient;
