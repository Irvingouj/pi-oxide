use serde::Serialize;

// ---------------------------------------------------------------------------
// Wire-format message types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicContentBlock<'a> {
    Text {
        text: &'a str,
    },
    ToolUse {
        id: &'a str,
        name: &'a str,
        input: &'a serde_json::Value,
    },
}

#[derive(Debug, Serialize)]
struct AnthropicUserMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Serialize)]
struct AnthropicAssistantMessage<'a> {
    role: &'a str,
    content: Vec<AnthropicContentBlock<'a>>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicToolResultBlock {
    ToolResult {
        #[serde(rename = "tool_use_id")]
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
}

#[derive(Debug, Serialize)]
struct AnthropicToolResultMessage<'a> {
    role: &'a str,
    content: Vec<AnthropicToolResultBlock>,
}

#[derive(Debug, Serialize)]
struct OpenAISystemMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Serialize)]
struct OpenAIUserMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Serialize)]
struct OpenAIFunctionCall<'a> {
    name: &'a str,
    /// DeepSeek requires arguments as a JSON-encoded string, not an object.
    arguments: &'a str,
}

#[derive(Debug, Serialize)]
struct OpenAIToolCall<'a> {
    id: &'a str,
    #[serde(rename = "type")]
    tool_type: &'a str,
    function: OpenAIFunctionCall<'a>,
}

#[derive(Debug, Serialize)]
struct OpenAIAssistantMessage<'a> {
    role: &'a str,
    /// DeepSeek requires `null` (not empty string) when tool_calls is present.
    content: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAIToolCall<'a>>>,
}

#[derive(Debug, Serialize)]
struct OpenAIToolResultMessage<'a> {
    role: &'a str,
    #[serde(rename = "tool_call_id")]
    tool_call_id: &'a str,
    content: &'a str,
}

// ---------------------------------------------------------------------------
// Message conversion
// ---------------------------------------------------------------------------

pub(crate) fn convert_messages(messages: &[pi_core::AgentMessage]) -> Vec<serde_json::Value> {
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
pub(crate) fn convert_messages_openai(
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
                let mut arguments_json_strings: Vec<String> = Vec::new();
                for c in &asst_msg.content {
                    match c {
                        pi_core::Content::Text(t) => {
                            content.push_str(&t.text);
                        }
                        pi_core::Content::ToolCall(tc) => {
                            // DeepSeek requires arguments as a JSON-encoded string
                            let args_str = serde_json::to_string(&tc.arguments.0)
                                .unwrap_or_else(|_| "{}".to_string());
                            arguments_json_strings.push(args_str);
                            tool_calls.push(OpenAIToolCall {
                                id: tc.id.as_str(),
                                tool_type: "function",
                                function: OpenAIFunctionCall {
                                    name: tc.name.as_str(),
                                    // Placeholder — will set below after strings are stable
                                    arguments: "",
                                },
                            });
                        }
                        _ => {}
                    }
                }
                // Patch in the pre-serialized argument strings
                for (tc, args_str) in tool_calls.iter_mut().zip(arguments_json_strings.iter()) {
                    tc.function.arguments = args_str.as_str();
                }

                let has_tools = !tool_calls.is_empty();
                // DeepSeek: content must be null when tool_calls is present
                let content_val: Option<&str> = if has_tools {
                    None
                } else if content.is_empty() {
                    Some(" ")
                } else {
                    Some(content.as_str())
                };
                result.push(
                    serde_json::to_value(OpenAIAssistantMessage {
                        role: "assistant",
                        content: content_val,
                        tool_calls: if has_tools { Some(tool_calls) } else { None },
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
