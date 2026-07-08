//! Async LLM stream using reqwest response body.
//!
//! Collects the full SSE response into memory, then parses chunks synchronously  
//! via `.next_chunk()`. For typical LLM responses (< 1MB), this avoids needing
//! tokio_util and is simpler while remaining fast enough for TUI rendering at 30fps.

use super::{CollectedToolCall, PartialToolCall, WireFormat};

/// Shared collected state — accumulated during SSE parsing.
#[derive(Default)]
pub(crate) struct CollectedState {
    pub tool_calls: Vec<PartialToolCall>,
    pub stop_reason: Option<String>,
    pub usage_input: Option<u32>,
    pub usage_output: Option<u32>,
}

/// Streaming response from an LLM provider — async version.
pub struct AsyncLlmStream {
    /// Full SSE text buffer (collected from HTTP body).
    text: String,
    pos: usize,
    wire_format: WireFormat,
    state: CollectedState,
    done: bool,
}

impl AsyncLlmStream {
    pub fn new(body_bytes: Vec<u8>, wire_format: WireFormat) -> Self {
        let text = String::from_utf8_lossy(&body_bytes).into_owned();
        Self {
            text,
            pos: 0,
            wire_format,
            state: CollectedState::default(),
            done: false,
        }
    }

    /// Read the next chunk from buffer (non-blocking after init).
    pub fn next_chunk(&mut self) -> Option<pi_core::LlmChunk> {
        if self.done || !self.has_remaining() {
            return None;
        }

        loop {
            let remaining = match self.text.get(self.pos..) {
                Some(s) => s,
                None => {
                    self.done = true;
                    return None;
                }
            };
            match self.wire_format {
                WireFormat::Anthropic => {
                    // Anthropic: SSE with \n\n event boundaries
                    if let Some(pos) = remaining.find("\n\n") {
                        let event_text = remaining[..pos].to_string();
                        self.pos += pos + 2;
                        if let Some(chunk) = parse_anthropic_sse_event(&mut self.state, &event_text)
                        {
                            return Some(chunk);
                        }
                    } else {
                        // No more \n\n — drain remaining as one event
                        if !remaining.is_empty() && remaining.contains('\n') {
                            let text = remaining.to_string();
                            self.pos += text.len();
                            if let Some(chunk) = parse_anthropic_sse_event(&mut self.state, &text) {
                                return Some(chunk);
                            }
                        }
                        break;
                    }
                }
                WireFormat::OpenAI => {
                    // OpenAI: each data: line is a frame, split by \n\n
                    if let Some(pos) = remaining.find('\n') {
                        let line = remaining[..pos].to_string();
                        self.pos += pos + 1;
                        if let Some(chunk) = parse_openai_sse_line(&mut self.state, &line) {
                            return Some(chunk);
                        }
                    } else {
                        break;
                    }
                }
            }

            // Check again after processing one event/line
            if !self.has_remaining() {
                self.done = true;
                return None;
            }
        }

        None
    }

    fn has_remaining(&self) -> bool {
        if self.pos >= self.text.len() {
            return false;
        }
        let remaining = &self.text[self.pos..];
        !remaining.is_empty()
    }

    pub fn usage(&self) -> Option<(u32, u32, u32)> {
        match (self.state.usage_input, self.state.usage_output) {
            (Some(i), Some(o)) => Some((i, o, i + o)),
            _ => None,
        }
    }

    pub fn stop_reason(&self) -> Option<&str> {
        self.state.stop_reason.as_deref()
    }

    pub fn tool_calls(&self) -> Vec<CollectedToolCall> {
        self.state
            .tool_calls
            .iter()
            .map(|tc| {
                let input: serde_json::Value = serde_json::from_str(&tc.input_json).unwrap_or_else(|e| {
                    tracing::warn!(tool_call_id = tc.id.as_str(), error = %e, "malformed tool input JSON");
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

    #[allow(dead_code)] // used by consumers to check for more chunks
    pub fn has_more(&self) -> bool {
        !self.done && self.has_remaining()
    }
}

// ---------------------------------------------------------------------------
// Shared SSE parsing — used by both sync and async streams
// ---------------------------------------------------------------------------

/// Parse an Anthropic-format SSE event (delimited by \n\n).
pub(super) fn parse_anthropic_sse_event(
    state: &mut CollectedState,
    text: &str,
) -> Option<pi_core::LlmChunk> {
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
                if let Some(input_tokens) = msg
                    .get("message")
                    .and_then(|m| m.get("usage"))
                    .and_then(|u| u.get("input_tokens"))
                    .and_then(|v| v.as_u64())
                {
                    state.usage_input = Some(input_tokens as u32);
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
                        state.tool_calls.push(PartialToolCall {
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
                            if let Some(tc) = state.tool_calls.last_mut() {
                                tc.input_json.push_str(&json_str);
                            }
                        }
                        _ => {}
                    }
                }
            }
            None
        }
        "content_block_stop" | "message_delta" => {
            if let Ok(delta) = serde_json::from_str::<serde_json::Value>(data) {
                // message_delta: extract output tokens + stop_reason
                if event_type == "message_delta" {
                    if let Some(output_tokens) = delta
                        .get("usage")
                        .and_then(|u| u.get("output_tokens"))
                        .and_then(|v| v.as_u64())
                    {
                        state.usage_output = Some(output_tokens as u32);
                    }
                    if let Some(stop) = delta
                        .get("delta")
                        .and_then(|d| d.get("stop_reason"))
                        .and_then(|v| v.as_str())
                    {
                        state.stop_reason = Some(stop.to_string());
                    }
                }
            }
            None
        }
        "message_stop" => Some(pi_core::LlmChunk::Done),
        "error" => {
            let msg = match serde_json::from_str::<serde_json::Value>(data) {
                Ok(v) => v
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown API error")
                    .to_string(),
                Err(_) => "unknown API error".into(),
            };
            Some(pi_core::LlmChunk::Error { message: msg })
        }
        _ => None,
    }
}

/// Parse a single line from OpenAI SSE stream.
pub(super) fn parse_openai_sse_line(
    state: &mut CollectedState,
    line: &str,
) -> Option<pi_core::LlmChunk> {
    let data = line.strip_prefix("data: ")?;

    if data == "[DONE]" {
        return Some(pi_core::LlmChunk::Done);
    }

    let Ok(json) = serde_json::from_str::<serde_json::Value>(data) else {
        return None;
    };

    // Extract usage from the first frame that has it
    if let Some(usage) = json.get("usage") {
        if let Some(i) = usage.get("prompt_tokens").and_then(|v| v.as_u64()) {
            state.usage_input = Some(i as u32);
        }
        if let Some(o) = usage.get("completion_tokens").and_then(|v| v.as_u64()) {
            state.usage_output = Some(o as u32);
        }
    }

    // Check finish_reason first
    if let Some(finish_reason) = json
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|c| c.first())
        .and_then(|c| c.get("finish_reason"))
        .and_then(|v| v.as_str())
    {
        let stop = match finish_reason {
            "tool_calls" => "tool_use",
            "length" => "max_tokens",
            _ => "end_turn",
        };
        state.stop_reason = Some(stop.to_string());
    }

    let delta = json
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|c| c.first())
        .and_then(|c| c.get("delta"))?;

    // Text delta
    if let Some(text) = delta.get("content").and_then(|v| v.as_str()) {
        if !text.is_empty() {
            return Some(pi_core::LlmChunk::TextDelta {
                text: text.to_string(),
            });
        }
    }

    // Tool call deltas — accumulate by index
    if let Some(tc_deltas) = delta.get("tool_calls").and_then(|v| v.as_array()) {
        for tc in tc_deltas {
            let index = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            while state.tool_calls.len() <= index {
                state.tool_calls.push(PartialToolCall {
                    id: String::new(),
                    name: String::new(),
                    input_json: String::new(),
                });
            }
            let entry = &mut state.tool_calls[index];
            if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                entry.id = id.to_string();
            }
            if let Some(func) = tc.get("function") {
                if let Some(name) = func.get("name").and_then(|v| v.as_str()) {
                    entry.name = name.to_string();
                }
                if let Some(args) = func.get("arguments").and_then(|v| v.as_str()) {
                    entry.input_json.push_str(args);
                }
            }
        }
    }

    None
}
