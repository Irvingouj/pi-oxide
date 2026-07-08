use super::messages::{convert_messages, convert_messages_openai};
use super::{LlmClient, WireFormat};
use pi_core::ToolDefinition;
use serde::Serialize;

// ---------------------------------------------------------------------------
// Wire-format request types
// ---------------------------------------------------------------------------

/// Anthropic Messages API request body.
#[derive(Debug, Serialize)]
struct AnthropicRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    system: &'a str,
    messages: Vec<serde_json::Value>,
    stream: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<AnthropicTool<'a>>,
}

#[derive(Debug, Serialize)]
struct AnthropicTool<'a> {
    name: &'a str,
    description: &'a str,
    input_schema: &'a serde_json::Value,
}

/// OpenAI Chat Completions API request body.
#[derive(Debug, Serialize)]
struct OpenAIRequest<'a> {
    model: &'a str,
    max_completion_tokens: u32,
    messages: Vec<serde_json::Value>,
    stream: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<OpenAITool<'a>>,
}

#[derive(Debug, Serialize)]
struct OpenAITool<'a> {
    #[serde(rename = "type")]
    tool_type: &'a str,
    function: OpenAIFunction<'a>,
}

#[derive(Debug, Serialize)]
struct OpenAIFunction<'a> {
    name: &'a str,
    description: &'a str,
    parameters: &'a serde_json::Value,
}

impl<'a> From<&'a ToolDefinition> for AnthropicTool<'a> {
    fn from(t: &'a ToolDefinition) -> Self {
        AnthropicTool {
            name: t.name.as_str(),
            description: &t.description,
            input_schema: &t.parameters.0,
        }
    }
}

impl<'a> From<&'a ToolDefinition> for OpenAITool<'a> {
    fn from(t: &'a ToolDefinition) -> Self {
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
// LlmClient::build_body
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Standalone body builder — used by both sync and async clients
// ---------------------------------------------------------------------------

/// Build the JSON body for an LLM streaming request.
pub fn build_body(
    model: &str,
    system_prompt: &str,
    messages: &[pi_core::AgentMessage],
    tools: &[ToolDefinition],
    wire_format: WireFormat,
) -> serde_json::Value {
    match wire_format {
        WireFormat::Anthropic => {
            let body = AnthropicRequest {
                model,
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
                model,
                max_completion_tokens: 16384,
                messages: convert_messages_openai(messages, system_prompt),
                stream: true,
                tools: tools.iter().map(OpenAITool::from).collect(),
            };
            serde_json::to_value(&body).expect("serialize openai request")
        }
    }
}

// ---------------------------------------------------------------------------
// LlmClient::build_body (backward compat)
// ---------------------------------------------------------------------------

impl LlmClient {
    #[allow(dead_code)] // used by LlmClient::stream_sync
    pub(crate) fn build_body(
        &self,
        system_prompt: &str,
        messages: &[pi_core::AgentMessage],
        tools: &[ToolDefinition],
    ) -> serde_json::Value {
        build_body(
            &self.model,
            system_prompt,
            messages,
            tools,
            self.wire_format,
        )
    }
}
