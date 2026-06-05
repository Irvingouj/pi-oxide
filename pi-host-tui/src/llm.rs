//! Anthropic Messages API streaming client.
//!
//! Synchronous streaming via reqwest::blocking. Parses SSE events
//! and provides an iterator of LlmChunk values with collected state.

use std::io::Read;

// --- Public types ---

#[derive(Clone)]
pub struct LlmClient {
    client: reqwest::blocking::Client,
    api_key: String,
    base_url: String,
    model: String,
}

/// Streaming response from the Anthropic API.
///
/// Iterate via `for chunk in stream.by_ref()`, then call
/// `.usage()`, `.stop_reason()`, `.tool_calls()` for collected state.
pub struct LlmStream {
    reader: reqwest::blocking::Response,
    buffer: String,
    // Collected state (accumulated during iteration)
    tool_calls: Vec<PartialToolCall>,
    stop_reason: Option<String>,
    usage_input: Option<u32>,
    usage_output: Option<u32>,
    done: bool,
}

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
                let input: serde_json::Value = serde_json::from_str(&tc.input_json)
                    .unwrap_or(serde_json::Value::Object(Default::default()));
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

impl LlmClient {
    pub fn new(api_key: &str, base_url: &str, model: &str) -> Self {
        Self {
            client: reqwest::blocking::Client::new(),
            api_key: api_key.to_string(),
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
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
        let url = format!("{}/v1/messages", self.base_url);
        let body = self.build_body(system_prompt, messages, tools);

        tracing::debug!(url, model = %self.model, messages = messages.len(), tools = tools.len(), "POST /v1/messages");

        let resp = self
            .client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()?;

        tracing::debug!(status = %resp.status(), "API response");

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            return Err(format!("API error {status}: {text}").into());
        }

        Ok(LlmStream {
            reader: resp,
            buffer: String::with_capacity(8192),
            tool_calls: Vec::new(),
            stop_reason: None,
            usage_input: None,
            usage_output: None,
            done: false,
        })
    }

    fn build_body(
        &self,
        system_prompt: &str,
        messages: &[pi_core::AgentMessage],
        tools: &[pi_core::ToolDefinition],
    ) -> serde_json::Value {
        let api_messages = convert_messages(messages);
        let api_tools: Vec<serde_json::Value> = tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name.as_str(),
                    "description": t.description,
                    "input_schema": t.parameters.0,
                })
            })
            .collect();

        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": 16384,
            "system": system_prompt,
            "messages": api_messages,
            "stream": true,
        });

        if !api_tools.is_empty() {
            body["tools"] = serde_json::json!(api_tools);
        }

        body
    }
}

// --- SSE stream iterator ---

impl Iterator for LlmStream {
    type Item = pi_core::LlmChunk;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }

        loop {
            // Try to extract a complete SSE event from buffer
            if let Some(pos) = self.buffer.find("\n\n") {
                let event_text = self.buffer[..pos].to_string();
                self.buffer = self.buffer[pos + 2..].to_string();

                if let Some(chunk) = self.parse_sse_event(&event_text) {
                    return Some(chunk);
                }
                continue;
            }

            // Need more data
            let mut buf = [0u8; 8192];
            match self.reader.read(&mut buf) {
                Ok(0) => {
                    self.done = true;
                    return None;
                }
                Ok(n) => {
                    let text = String::from_utf8_lossy(&buf[..n]);
                    self.buffer.push_str(&text);
                }
                Err(e) => {
                    self.done = true;
                    return Some(pi_core::LlmChunk::Error {
                        message: e.to_string(),
                    });
                }
            }
        }
    }
}

impl LlmStream {
    fn parse_sse_event(&mut self, text: &str) -> Option<pi_core::LlmChunk> {
        let mut event_type = "";
        let mut data = "";

        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("event: ") {
                event_type = rest.trim();
            } else if let Some(rest) = line.strip_prefix("data: ") {
                data = rest;
            }
        }

        if data.is_empty() {
            return None;
        }

        tracing::trace!(event_type, data_len = data.len(), "SSE event");

        match event_type {
            "message_start" => {
                if let Ok(msg) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(input) = msg
                        .get("message")
                        .and_then(|m| m.get("usage"))
                        .and_then(|u| u.get("input_tokens"))
                        .and_then(|v| v.as_u64())
                    {
                        self.usage_input = Some(input as u32);
                    }
                }
                None
            }
            "content_block_start" => {
                if let Ok(block) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(cb) = block.get("content_block") {
                        if cb.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                            let id = cb
                                .get("id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let name = cb
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            self.tool_calls.push(PartialToolCall {
                                id,
                                name,
                                input_json: String::new(),
                            });
                        }
                    }
                }
                None
            }
            "content_block_delta" => {
                if let Ok(delta) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(d) = delta.get("delta") {
                        match d.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                            "text_delta" => {
                                let text = d
                                    .get("text")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                return Some(pi_core::LlmChunk::TextDelta { text });
                            }
                            "input_json_delta" => {
                                let json_str = d
                                    .get("partial_json")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                if let Some(tc) = self.tool_calls.last_mut() {
                                    tc.input_json.push_str(&json_str);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                None
            }
            "content_block_stop" => None,
            "message_delta" => {
                if let Ok(delta) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(output) = delta
                        .get("usage")
                        .and_then(|u| u.get("output_tokens"))
                        .and_then(|v| v.as_u64())
                    {
                        self.usage_output = Some(output as u32);
                    }
                    if let Some(stop) = delta
                        .get("delta")
                        .and_then(|d| d.get("stop_reason"))
                        .and_then(|v| v.as_str())
                    {
                        self.stop_reason = Some(stop.to_string());
                    }
                }
                None
            }
            "message_stop" => {
                self.done = true;
                Some(pi_core::LlmChunk::Done)
            }
            "error" => {
                let msg = match serde_json::from_str::<serde_json::Value>(data) {
                    Ok(v) => v
                        .get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown API error")
                        .to_string(),
                    Err(_) => "unknown API error".to_string(),
                };
                self.done = true;
                Some(pi_core::LlmChunk::Error { message: msg })
            }
            _ => None,
        }
    }
}

// --- Message conversion ---

fn convert_messages(messages: &[pi_core::AgentMessage]) -> Vec<serde_json::Value> {
    let mut result = Vec::new();

    for msg in messages {
        match msg {
            pi_core::AgentMessage::User(user_msg) => {
                let text = user_msg
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        pi_core::Content::Text(t) => Some(t.text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");

                if !text.is_empty() {
                    result.push(serde_json::json!({
                        "role": "user",
                        "content": text,
                    }));
                }
            }
            pi_core::AgentMessage::Assistant(asst_msg) => {
                let blocks: Vec<serde_json::Value> = asst_msg
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        pi_core::Content::Text(t) => {
                            if t.text.is_empty() {
                                return None;
                            }
                            Some(serde_json::json!({ "type": "text", "text": t.text }))
                        }
                        pi_core::Content::ToolCall(tc) => Some(serde_json::json!({
                            "type": "tool_use",
                            "id": tc.id.as_str(),
                            "name": tc.name.as_str(),
                            "input": tc.arguments.0,
                        })),
                        _ => None,
                    })
                    .collect();

                result.push(serde_json::json!({
                    "role": "assistant",
                    "content": blocks,
                }));
            }
            pi_core::AgentMessage::ToolResult(tr_msg) => {
                let text = tr_msg
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        pi_core::Content::Text(t) => Some(t.text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");

                result.push(serde_json::json!({
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": tr_msg.tool_call_id.as_str(),
                        "content": text,
                        "is_error": tr_msg.is_error,
                    }],
                }));
            }
        }
    }

    result
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_llm_client_construction() {
        let client = LlmClient::new(
            "test-key",
            "https://api.anthropic.com",
            "claude-sonnet-4-20250514",
        );
        assert_eq!(client.model_id(), "claude-sonnet-4-20250514");
    }

    #[test]
    fn test_llm_client_trailing_slash() {
        let client = LlmClient::new("key", "https://api.anthropic.com/", "model");
        assert_eq!(client.base_url, "https://api.anthropic.com");
    }

    #[test]
    fn test_convert_user_message() {
        let msgs = vec![pi_core::AgentMessage::user("hello")];
        let converted = convert_messages(&msgs);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["role"], "user");
        assert_eq!(converted[0]["content"], "hello");
    }

    #[test]
    fn test_convert_assistant_message() {
        let msgs = vec![pi_core::AgentMessage::assistant_text("hi there")];
        let converted = convert_messages(&msgs);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["role"], "assistant");
    }
}

#[cfg(feature = "record")]
pub type LlmBackend = crate::llm_record::RecordingLlmClient;

#[cfg(all(feature = "replay", not(feature = "record")))]
pub type LlmBackend = crate::llm_replay::ReplayLlmClient;
