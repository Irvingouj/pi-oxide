//! LLM streaming client supporting multiple provider wire formats.
//!
//! Synchronous streaming via reqwest::blocking. Parses SSE events
//! and provides an iterator of LlmChunk values with collected state.

use std::io::Read;

// --- Public types ---
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

// ---------------------------------------------------------------------------
// Wire-format request types
// ---------------------------------------------------------------------------

/// Anthropic Messages API request body.
#[derive(Debug, serde::Serialize)]
struct AnthropicRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    system: &'a str,
    messages: Vec<serde_json::Value>,
    stream: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<AnthropicTool<'a>>,
}

#[derive(Debug, serde::Serialize)]
struct AnthropicTool<'a> {
    name: &'a str,
    description: &'a str,
    input_schema: &'a serde_json::Value,
}

/// OpenAI Chat Completions API request body.
#[derive(Debug, serde::Serialize)]
struct OpenAIRequest<'a> {
    model: &'a str,
    max_completion_tokens: u32,
    messages: Vec<serde_json::Value>,
    stream: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<OpenAITool<'a>>,
}

#[derive(Debug, serde::Serialize)]
struct OpenAITool<'a> {
    #[serde(rename = "type")]
    tool_type: &'a str,
    function: OpenAIFunction<'a>,
}

#[derive(Debug, serde::Serialize)]
struct OpenAIFunction<'a> {
    name: &'a str,
    description: &'a str,
    parameters: &'a serde_json::Value,
}

impl<'a> From<&'a pi_core::ToolDefinition> for AnthropicTool<'a> {
    fn from(t: &'a pi_core::ToolDefinition) -> Self {
        AnthropicTool {
            name: t.name.as_str(),
            description: &t.description,
            input_schema: &t.parameters.0,
        }
    }
}

impl<'a> From<&'a pi_core::ToolDefinition> for OpenAITool<'a> {
    fn from(t: &'a pi_core::ToolDefinition) -> Self {
        OpenAITool {
            tool_type: "function",
            function: OpenAIFunction {
                name: t.name.as_str(),
                description: &t.description,
                parameters: &t.parameters.0,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Wire-format message types
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Serialize)]
#[serde(tag = "type")]
enum AnthropicContentBlock<'a> {
    Text {
        text: &'a str,
    },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: &'a str,
        name: &'a str,
        input: &'a serde_json::Value,
    },
}

#[derive(Debug, serde::Serialize)]
struct AnthropicUserMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, serde::Serialize)]
struct AnthropicAssistantMessage<'a> {
    role: &'a str,
    content: Vec<AnthropicContentBlock<'a>>,
}

#[derive(Debug, serde::Serialize)]
#[serde(tag = "type")]
enum AnthropicToolResultBlock {
    #[serde(rename = "tool_result")]
    ToolResult {
        #[serde(rename = "tool_use_id")]
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
}

#[derive(Debug, serde::Serialize)]
struct AnthropicToolResultMessage<'a> {
    role: &'a str,
    content: Vec<AnthropicToolResultBlock>,
}

#[derive(Debug, serde::Serialize)]
struct OpenAISystemMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, serde::Serialize)]
struct OpenAIUserMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, serde::Serialize)]
struct OpenAIFunctionCall<'a> {
    name: &'a str,
    arguments: &'a serde_json::Value,
}

#[derive(Debug, serde::Serialize)]
struct OpenAIToolCall<'a> {
    id: &'a str,
    #[serde(rename = "type")]
    tool_type: &'a str,
    function: OpenAIFunctionCall<'a>,
}

#[derive(Debug, serde::Serialize)]
struct OpenAIAssistantMessage<'a> {
    role: &'a str,
    content: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAIToolCall<'a>>>,
}

#[derive(Debug, serde::Serialize)]
struct OpenAIToolResultMessage<'a> {
    role: &'a str,
    #[serde(rename = "tool_call_id")]
    tool_call_id: &'a str,
    content: &'a str,
}

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

        tracing::debug!(url, model = %self.model, messages = messages.len(), tools = tools.len(), "POST");

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

    fn build_body(
        &self,
        system_prompt: &str,
        messages: &[pi_core::AgentMessage],
        tools: &[pi_core::ToolDefinition],
    ) -> serde_json::Value {
        match self.wire_format {
            WireFormat::Anthropic => {
                let body = AnthropicRequest {
                    model: &self.model,
                    max_tokens: 16384,
                    system: system_prompt,
                    messages: convert_messages(messages),
                    stream: true,
                    tools: tools.iter().map(AnthropicTool::from).collect(),
                };
                serde_json::to_value(&body).expect("serialize anthropic request")
            }
            WireFormat::OpenAI => {
                let body = OpenAIRequest {
                    model: &self.model,
                    max_completion_tokens: 16384,
                    messages: convert_messages_openai(messages, system_prompt),
                    stream: true,
                    tools: tools.iter().map(OpenAITool::from).collect(),
                };
                serde_json::to_value(&body).expect("serialize openai request")
            }
        }
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

// --- Message conversion ---

fn convert_messages(messages: &[pi_core::AgentMessage]) -> Vec<serde_json::Value> {
    let mut result = Vec::new();

    let user_text = |content: &[pi_core::Content]| {
        content
            .iter()
            .filter_map(|c| match c {
                pi_core::Content::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    let mut i = 0;
    while i < messages.len() {
        let msg = &messages[i];
        match msg {
            pi_core::AgentMessage::User(user_msg) => {
                // Merge consecutive user messages into one — strict
                // Anthropic-compatible endpoints reject adjacent same-role
                // messages ("roles must alternate").
                let mut texts = vec![user_text(&user_msg.content)];
                while i + 1 < messages.len()
                    && matches!(messages[i + 1], pi_core::AgentMessage::User(_))
                {
                    i += 1;
                    if let pi_core::AgentMessage::User(u) = &messages[i] {
                        texts.push(user_text(&u.content));
                    }
                }
                let joined = texts
                    .into_iter()
                    .filter(|t| !t.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n");
                if !joined.is_empty() {
                    result.push(
                        serde_json::to_value(AnthropicUserMessage {
                            role: "user",
                            content: &joined,
                        })
                        .expect("serialize user message"),
                    );
                }
                i += 1;
            }
            pi_core::AgentMessage::Assistant(asst_msg) => {
                let blocks: Vec<AnthropicContentBlock> = asst_msg
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        pi_core::Content::Text(t) => {
                            if t.text.is_empty() {
                                return None;
                            }
                            Some(AnthropicContentBlock::Text { text: &t.text })
                        }
                        pi_core::Content::ToolCall(tc) => Some(AnthropicContentBlock::ToolUse {
                            id: tc.id.as_str(),
                            name: tc.name.as_str(),
                            input: &tc.arguments.0,
                        }),
                        _ => None,
                    })
                    .collect();

                // Skip empty assistant content — Anthropic rejects it.
                if !blocks.is_empty() {
                    result.push(
                        serde_json::to_value(AnthropicAssistantMessage {
                            role: "assistant",
                            content: blocks,
                        })
                        .expect("serialize assistant message"),
                    );
                }
                i += 1;
            }
            pi_core::AgentMessage::ToolResult(tr_msg) => {
                // Coalesce consecutive tool_results into one user message.
                let content = user_text(&tr_msg.content);
                let mut blocks = vec![AnthropicToolResultBlock::ToolResult {
                    tool_use_id: tr_msg.tool_call_id.as_str().to_string(),
                    content,
                    is_error: tr_msg.is_error,
                }];
                while i + 1 < messages.len()
                    && matches!(messages[i + 1], pi_core::AgentMessage::ToolResult(_))
                {
                    i += 1;
                    if let pi_core::AgentMessage::ToolResult(tr) = &messages[i] {
                        let content = user_text(&tr.content);
                        blocks.push(AnthropicToolResultBlock::ToolResult {
                            tool_use_id: tr.tool_call_id.as_str().to_string(),
                            content,
                            is_error: tr.is_error,
                        });
                    }
                }
                result.push(
                    serde_json::to_value(AnthropicToolResultMessage {
                        role: "user",
                        content: blocks,
                    })
                    .expect("serialize tool result message"),
                );
                i += 1;
            }
        }
    }

    result
}

/// Convert agent messages to OpenAI Chat Completions wire format.
fn convert_messages_openai(
    messages: &[pi_core::AgentMessage],
    system_prompt: &str,
) -> Vec<serde_json::Value> {
    let user_text = |content: &[pi_core::Content]| {
        content
            .iter()
            .filter_map(|c| match c {
                pi_core::Content::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    let mut result = Vec::new();

    // System prompt as first message
    if !system_prompt.is_empty() {
        result.push(
            serde_json::to_value(OpenAISystemMessage {
                role: "system",
                content: system_prompt,
            })
            .expect("serialize system message"),
        );
    }

    for msg in messages {
        match msg {
            pi_core::AgentMessage::User(user_msg) => {
                let text = user_text(&user_msg.content);
                if !text.is_empty() {
                    result.push(
                        serde_json::to_value(OpenAIUserMessage {
                            role: "user",
                            content: &text,
                        })
                        .expect("serialize user message"),
                    );
                }
            }
            pi_core::AgentMessage::Assistant(asst_msg) => {
                let mut content = String::new();
                let mut tool_calls: Vec<OpenAIToolCall> = Vec::new();
                for c in &asst_msg.content {
                    match c {
                        pi_core::Content::Text(t) => {
                            content.push_str(&t.text);
                        }
                        pi_core::Content::ToolCall(tc) => {
                            tool_calls.push(OpenAIToolCall {
                                id: tc.id.as_str(),
                                tool_type: "function",
                                function: OpenAIFunctionCall {
                                    name: tc.name.as_str(),
                                    arguments: &tc.arguments.0,
                                },
                            });
                        }
                        _ => {}
                    }
                }
                // OpenAI requires non-empty content; use " " if empty
                let content_val = if content.is_empty() {
                    " "
                } else {
                    content.as_str()
                };
                result.push(
                    serde_json::to_value(OpenAIAssistantMessage {
                        role: "assistant",
                        content: content_val,
                        tool_calls: if tool_calls.is_empty() {
                            None
                        } else {
                            Some(tool_calls)
                        },
                    })
                    .expect("serialize assistant message"),
                );
            }
            pi_core::AgentMessage::ToolResult(tr_msg) => {
                let text = user_text(&tr_msg.content);
                result.push(
                    serde_json::to_value(OpenAIToolResultMessage {
                        role: "tool",
                        tool_call_id: tr_msg.tool_call_id.as_str(),
                        content: &text,
                    })
                    .expect("serialize tool result message"),
                );
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

#[cfg(feature = "record")]
pub type LlmBackend = crate::llm_record::RecordingLlmClient;

#[cfg(all(feature = "replay", not(feature = "record")))]
pub type LlmBackend = crate::llm_replay::ReplayLlmClient;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_llm_client_construction() {
        let client = LlmClient::new(
            "test-key",
            "https://api.anthropic.com",
            "claude-sonnet-5",
            WireFormat::Anthropic,
        );
        assert_eq!(client.model_id(), "claude-sonnet-5");
    }

    #[test]
    fn test_llm_client_trailing_slash() {
        let client = LlmClient::new(
            "key",
            "https://api.anthropic.com/",
            "model",
            WireFormat::Anthropic,
        );
        assert_eq!(client.base_url, "https://api.anthropic.com");
    }

    #[test]
    fn test_openai_client_construction() {
        let client = LlmClient::new(
            "key",
            "https://api.openai.com",
            "gpt-4o",
            WireFormat::OpenAI,
        );
        assert_eq!(client.model_id(), "gpt-4o");
        assert_eq!(client.wire_format, WireFormat::OpenAI);
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

    #[test]
    fn test_convert_skips_empty_assistant_message() {
        // An empty assistant message (blank text only) must not be emitted —
        // Anthropic rejects empty assistant content arrays.
        let msgs = vec![
            pi_core::AgentMessage::user("hello"),
            pi_core::AgentMessage::Assistant(pi_core::AssistantMessage::empty()),
            pi_core::AgentMessage::user("again"),
        ];
        let converted = convert_messages(&msgs);
        assert_eq!(converted.len(), 2, "empty assistant must be omitted");
        assert_eq!(converted[0]["role"], "user");
        assert_eq!(converted[1]["role"], "user");
    }

    #[test]
    fn test_convert_merges_consecutive_user_messages() {
        // Strict Anthropic-compatible endpoints reject adjacent same-role
        // messages ("roles must alternate"). Two consecutive user messages
        // must merge into one.
        let msgs = vec![
            pi_core::AgentMessage::user("first"),
            pi_core::AgentMessage::user("second"),
            pi_core::AgentMessage::assistant_text("reply"),
        ];
        let converted = convert_messages(&msgs);
        assert_eq!(converted.len(), 2, "consecutive users must merge");
        assert_eq!(converted[0]["role"], "user");
        assert_eq!(converted[0]["content"], "first\nsecond");
        assert_eq!(converted[1]["role"], "assistant");
    }

    #[test]
    fn test_convert_merges_consecutive_tool_results() {
        // Parallel tool_results must land in one user message, same as the
        // other wire layers.
        let mk = |id: &str, text: &str| {
            pi_core::AgentMessage::ToolResult(pi_core::message::ToolResultMessage {
                role: "tool_result".to_string(),
                tool_call_id: pi_core::ToolCallId::new(id),
                tool_name: pi_core::ToolName::new("t"),
                content: vec![pi_core::Content::Text(pi_core::message::TextContent {
                    text: text.to_string(),
                })],
                details: None,
                is_error: false,
                timestamp: 0,
            })
        };
        let msgs = vec![mk("c1", "r1"), mk("c2", "r2")];
        let converted = convert_messages(&msgs);
        assert_eq!(converted.len(), 1, "tool_results must coalesce");
        assert_eq!(converted[0]["role"], "user");
        let blocks = converted[0]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
    }

    #[test]
    fn test_convert_openai_messages() {
        let msgs = vec![
            pi_core::AgentMessage::user("hello"),
            pi_core::AgentMessage::assistant_text("hi there"),
        ];
        let converted = convert_messages_openai(&msgs, "You are a helper");
        // system + user + assistant = 3
        assert_eq!(converted.len(), 3);
        assert_eq!(converted[0]["role"], "system");
        assert_eq!(converted[0]["content"], "You are a helper");
        assert_eq!(converted[1]["role"], "user");
        assert_eq!(converted[2]["role"], "assistant");
        assert_eq!(converted[2]["content"], "hi there");
    }

    #[test]
    fn test_convert_openai_tool_results() {
        // OpenAI uses role: "tool" for tool results, not merged into user
        let mk = |id: &str, text: &str| {
            pi_core::AgentMessage::ToolResult(pi_core::message::ToolResultMessage {
                role: "tool_result".to_string(),
                tool_call_id: pi_core::ToolCallId::new(id),
                tool_name: pi_core::ToolName::new("t"),
                content: vec![pi_core::Content::Text(pi_core::message::TextContent {
                    text: text.to_string(),
                })],
                details: None,
                is_error: false,
                timestamp: 0,
            })
        };
        let msgs = vec![mk("c1", "r1"), mk("c2", "r2")];
        let converted = convert_messages_openai(&msgs, "");
        // Each tool result is its own message with role: "tool"
        assert_eq!(converted.len(), 2);
        assert_eq!(converted[0]["role"], "tool");
        assert_eq!(converted[0]["tool_call_id"], "c1");
        assert_eq!(converted[1]["role"], "tool");
        assert_eq!(converted[1]["tool_call_id"], "c2");
    }
}
