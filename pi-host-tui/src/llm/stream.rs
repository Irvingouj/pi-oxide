use super::{LlmStream, PartialToolCall, WireFormat};
use std::io::Read;

// --- SSE stream iterator ---

impl Iterator for LlmStream {
    type Item = pi_core::LlmChunk;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }

        loop {
            match self.wire_format {
                WireFormat::Anthropic => {
                    // Anthropic: SSE with \n\n event boundaries
                    if let Some(pos) = self.buffer.find("\n\n") {
                        let event_text = self.buffer[..pos].to_string();
                        self.buffer = self.buffer[pos + 2..].to_string();
                        if let Some(chunk) = self.parse_sse_event(&event_text) {
                            return Some(chunk);
                        }
                        continue;
                    }
                }
                WireFormat::OpenAI => {
                    // OpenAI: each data: line is a frame, split by \n
                    if let Some(pos) = self.buffer.find('\n') {
                        let line = self.buffer[..pos].to_string();
                        self.buffer = self.buffer[pos + 1..].to_string();
                        if let Some(chunk) = self.parse_openai_sse_line(&line) {
                            return Some(chunk);
                        }
                        continue;
                    }
                }
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

    /// Parse a single line from OpenAI SSE stream.
    /// OpenAI sends `data: <json>` per line, no event: prefix, and `data: [DONE]` to end.
    fn parse_openai_sse_line(&mut self, line: &str) -> Option<pi_core::LlmChunk> {
        let data = line.strip_prefix("data: ")?;

        if data == "[DONE]" {
            self.done = true;
            return Some(pi_core::LlmChunk::Done);
        }

        let Ok(json) = serde_json::from_str::<serde_json::Value>(data) else {
            return None;
        };

        // Extract usage from the first frame that has it
        if let Some(usage) = json.get("usage") {
            if let Some(i) = usage.get("prompt_tokens").and_then(|v| v.as_u64()) {
                self.usage_input = Some(i as u32);
            }
            if let Some(o) = usage.get("completion_tokens").and_then(|v| v.as_u64()) {
                self.usage_output = Some(o as u32);
            }
        }

        let delta = json
            .get("choices")
            .and_then(|c| c.as_array())
            .and_then(|c| c.first())
            .and_then(|c| c.get("delta"))?;

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
            self.stop_reason = Some(stop.to_string());
        }

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
                // Ensure we have enough slots
                while self.tool_calls.len() <= index {
                    self.tool_calls.push(PartialToolCall {
                        id: String::new(),
                        name: String::new(),
                        input_json: String::new(),
                    });
                }
                let entry = &mut self.tool_calls[index];
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
}
