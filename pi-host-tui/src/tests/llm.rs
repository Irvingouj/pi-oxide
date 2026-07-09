use crate::llm::messages::{convert_messages, convert_messages_openai};
use crate::llm::*;

// ---------------------------------------------------------------------------
// Serialization regression tests — ensure content block types serialize as
// lowercase (e.g., "text", "tool_use", "tool_result") not PascalCase.
// Deepseek-anthropic rejects "Text" and returns 400.
// ---------------------------------------------------------------------------

#[test]
fn test_anthropic_text_block_serializes_as_lowercase() {
    // Regression: Text variant serialized as "Text" instead of "text",
    // causing deepseek-anthropic to reject the request with 400.
    let msgs = vec![pi_core::AgentMessage::assistant_text("hello")];
    let converted = convert_messages(&msgs);
    assert_eq!(converted.len(), 1);
    let content = converted[0]["content"].as_array().unwrap();
    assert_eq!(
        content[0]["type"], "text",
        "Text block must serialize as lowercase 'text'"
    );
}

#[test]
fn test_anthropic_tool_use_block_serializes_as_lowercase() {
    let tc = pi_core::ToolCall {
        id: pi_core::ToolCallId::new("call-1"),
        name: pi_core::ToolName::new("read"),
        arguments: pi_core::ToolArguments::new(serde_json::json!({"path": "foo.rs"})),
    };
    let msg = pi_core::AssistantMessage {
        content: vec![
            pi_core::Content::Text(pi_core::message::TextContent {
                text: "reading...".to_string(),
            }),
            pi_core::Content::ToolCall(tc),
        ],
        ..pi_core::AssistantMessage::empty()
    };
    let msgs = vec![pi_core::AgentMessage::Assistant(msg)];
    let converted = convert_messages(&msgs);
    assert_eq!(converted.len(), 1);
    let content = converted[0]["content"].as_array().unwrap();
    assert_eq!(content[0]["type"], "text", "Text must be lowercase");
    assert_eq!(
        content[1]["type"], "tool_use",
        "ToolUse must serialize as 'tool_use'"
    );
}

#[test]
fn test_anthropic_tool_result_block_serializes_as_lowercase() {
    let tr = pi_core::message::ToolResultMessage {
        role: "tool_result".to_string(),
        tool_call_id: pi_core::ToolCallId::new("call-1"),
        tool_name: pi_core::ToolName::new("read"),
        content: vec![pi_core::Content::Text(pi_core::message::TextContent {
            text: "file contents".to_string(),
        })],
        details: None,
        is_error: false,
        timestamp: 0,
    };
    let msgs = vec![pi_core::AgentMessage::ToolResult(tr)];
    let converted = convert_messages(&msgs);
    assert_eq!(converted.len(), 1);
    let content = converted[0]["content"].as_array().unwrap();
    assert_eq!(
        content[0]["type"], "tool_result",
        "ToolResult must serialize as 'tool_result'"
    );
}

#[test]
fn test_anthropic_full_multi_turn_flow_serializes_correctly() {
    // Simulate the full transcript after: user -> assistant(tool+text) -> tool_result
    // This is the exact pattern that caused the deepseek 400 regression.
    // Turn 1: assistant returns text + tool_use
    let tc = pi_core::ToolCall {
        id: pi_core::ToolCallId::new("call-1"),
        name: pi_core::ToolName::new("read"),
        arguments: pi_core::ToolArguments::new(serde_json::json!({"path": "Cargo.toml"})),
    };
    let assistant = pi_core::AssistantMessage {
        content: vec![
            pi_core::Content::Text(pi_core::message::TextContent {
                text: "Let me read the file.".to_string(),
            }),
            pi_core::Content::ToolCall(tc),
        ],
        ..pi_core::AssistantMessage::empty()
    };
    // Turn 2: tool result
    let tr = pi_core::message::ToolResultMessage {
        role: "tool_result".to_string(),
        tool_call_id: pi_core::ToolCallId::new("call-1"),
        tool_name: pi_core::ToolName::new("read"),
        content: vec![pi_core::Content::Text(pi_core::message::TextContent {
            text: "[workspace]\nmembers = [...]".to_string(),
        })],
        details: None,
        is_error: false,
        timestamp: 0,
    };

    let msgs = vec![
        pi_core::AgentMessage::user("read Cargo.toml"),
        pi_core::AgentMessage::Assistant(assistant),
        pi_core::AgentMessage::ToolResult(tr),
    ];
    let converted = convert_messages(&msgs);

    // Should be: user, assistant, user(tool_result)
    assert_eq!(converted.len(), 3);

    // Verify assistant message has correct content types
    let assistant_content = converted[1]["content"].as_array().unwrap();
    assert_eq!(assistant_content[0]["type"], "text");
    assert_eq!(assistant_content[1]["type"], "tool_use");

    // Verify tool result has correct type
    let tool_result_content = converted[2]["content"].as_array().unwrap();
    assert_eq!(tool_result_content[0]["type"], "tool_result");

    // Full JSON should not contain "Text" or "ToolUse" or "ToolResult" as type values
    let json = serde_json::to_string(&converted).unwrap();
    assert!(
        !json.contains("\"type\":\"Text\""),
        "JSON must not contain PascalCase 'Text' — use lowercase 'text'"
    );
    assert!(
        !json.contains("\"type\":\"ToolUse\""),
        "JSON must not contain PascalCase 'ToolUse' — use 'tool_use'"
    );
    assert!(
        !json.contains("\"type\":\"ToolResult\""),
        "JSON must not contain PascalCase 'ToolResult' — use 'tool_result'"
    );
}

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

// -----------------------------------------------------------------------
// Model discovery tests
// -----------------------------------------------------------------------

/// Mock discovery for testing trait abstraction
struct MockDiscovery {
    models: Vec<ModelInfo>,
}
impl ModelDiscovery for MockDiscovery {
    fn list_models(&self) -> Result<Vec<ModelInfo>, Box<dyn std::error::Error>> {
        Ok(self.models.clone())
    }
}

fn discover_via_trait<D: ModelDiscovery>(discovery: &D) -> Vec<String> {
    discovery
        .list_models()
        .map(|m| m.into_iter().map(|m| m.id).collect())
        .unwrap_or_default()
}

#[test]
fn test_trait_accepts_mock_discovery() {
    let mock = MockDiscovery {
        models: vec![
            ModelInfo {
                id: "mock-1".into(),
            },
            ModelInfo {
                id: "mock-2".into(),
            },
        ],
    };
    let ids = discover_via_trait(&mock);
    assert_eq!(ids, vec!["mock-1", "mock-2"]);
}

#[test]
fn test_trait_accepts_llm_client() {
    let client = LlmClient::new(
        "test-key",
        "https://api.anthropic.com",
        "claude-sonnet-5",
        WireFormat::Anthropic,
    );
    let ids = discover_via_trait(&client);
    assert!(ids.iter().any(|id| id == "claude-sonnet-5"));
    assert!(ids.iter().any(|id| id == "claude-opus-4"));
}

#[test]
fn test_list_models_anthropic_returns_hardcoded_list() {
    let client = LlmClient::new(
        "test-key",
        "https://api.anthropic.com",
        "claude-sonnet-5",
        WireFormat::Anthropic,
    );
    let models = client.list_models().unwrap();
    assert!(!models.is_empty());
    assert!(models.iter().any(|m| m.id == "claude-sonnet-5"));
    assert!(models.iter().any(|m| m.id == "claude-opus-4"));
}

#[test]
fn test_list_models_openai_parses_response() {
    // Test the parsing logic by constructing a mock response
    let response = serde_json::json!({
        "object": "list",
        "data": [
            {"id": "gpt-5.5", "object": "model"},
            {"id": "gpt-4o", "object": "model"},
            {"id": "gpt-4o-mini", "object": "model"}
        ]
    });

    let models: Vec<ModelInfo> = response
        .get("data")
        .and_then(|d| d.as_array())
        .into_iter()
        .flat_map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    m.get("id")
                        .and_then(|i| i.as_str())
                        .map(|id| ModelInfo { id: id.to_string() })
                })
                .collect::<Vec<_>>()
        })
        .collect();

    assert_eq!(models.len(), 3);
    assert_eq!(models[0].id, "gpt-5.5");
    assert_eq!(models[1].id, "gpt-4o");
    assert_eq!(models[2].id, "gpt-4o-mini");
}

#[test]
fn test_list_models_openai_handles_empty_response() {
    let response = serde_json::json!({
        "object": "list",
        "data": []
    });

    let models: Vec<ModelInfo> = response
        .get("data")
        .and_then(|d| d.as_array())
        .into_iter()
        .flat_map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    m.get("id")
                        .and_then(|i| i.as_str())
                        .map(|id| ModelInfo { id: id.to_string() })
                })
                .collect::<Vec<_>>()
        })
        .collect();

    assert!(models.is_empty());
}

#[test]
fn test_list_models_openai_handles_missing_data() {
    let response = serde_json::json!({
        "object": "list"
    });

    let models: Vec<ModelInfo> = response
        .get("data")
        .and_then(|d| d.as_array())
        .into_iter()
        .flat_map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    m.get("id")
                        .and_then(|i| i.as_str())
                        .map(|id| ModelInfo { id: id.to_string() })
                })
                .collect::<Vec<_>>()
        })
        .collect();

    assert!(models.is_empty());
}

/// Regression: verify multi-turn conversations with tool calls serialize
/// correctly for OpenAI/DeepSeek endpoints. DeepSeek rejects messages where
/// "content" is a map/array instead of a string.
#[test]
fn test_openai_multi_turn_with_tool_call_content_is_string() {
    use crate::llm::messages::convert_messages_openai;

    let mut messages = vec![
        pi_core::AgentMessage::user("Say hello"),
        pi_core::AgentMessage::assistant_text("Hello!"),
        pi_core::AgentMessage::user("What is pi-core?"),
    ];

    let tc = pi_core::ToolCall {
        id: pi_core::ToolCallId::new("call-1"),
        name: pi_core::ToolName::new("glob"),
        arguments: pi_core::ToolArguments::new(serde_json::json!({"paths": ["**/*"]})),
    };
    messages.push(pi_core::AgentMessage::Assistant(
        pi_core::AssistantMessage {
            content: vec![
                pi_core::Content::Text(pi_core::message::TextContent {
                    text: "Let me explore.".to_string(),
                }),
                pi_core::Content::ToolCall(tc),
            ],
            ..pi_core::AssistantMessage::empty()
        },
    ));

    messages.push(pi_core::AgentMessage::ToolResult(
        pi_core::message::ToolResultMessage {
            role: "tool_result".to_string(),
            tool_call_id: pi_core::ToolCallId::new("call-1"),
            tool_name: pi_core::ToolName::new("glob"),
            content: vec![pi_core::Content::Text(pi_core::message::TextContent {
                text: "file1.rs".to_string(),
            })],
            details: None,
            is_error: false,
            timestamp: 0,
        },
    ));

    let converted = convert_messages_openai(&messages, "You are helpful.");

    for (i, msg) in converted.iter().enumerate() {
        let json_str = serde_json::to_string(msg).unwrap();
        eprintln!("messages[{}]: {}", i, json_str);
        if let Some(content) = msg.get("content") {
            // When tool_calls is present, content must be null (DeepSeek requirement)
            if content.is_null() {
                assert!(
                    msg.get("tool_calls").is_some_and(|tc| tc.is_array()),
                    "messages[{}].content is null but no tool_calls present",
                    i
                );
            } else {
                assert!(
                    content.is_string(),
                    "messages[{}].content must be string or null, got: {}",
                    i,
                    content
                );
            }
        }
    }

    assert_eq!(converted.len(), 6);
    assert_eq!(converted[4]["role"], "assistant");
    // When tool_calls is present, content must be null for DeepSeek compatibility
    assert!(
        converted[4]["content"].is_null(),
        "content must be null when tool_calls present, got: {}",
        converted[4]["content"]
    );
    assert!(converted[4]["tool_calls"].is_array());
    // arguments must be a JSON string, not an object
    let tc_args = &converted[4]["tool_calls"][0]["function"]["arguments"];
    assert!(
        tc_args.is_string(),
        "arguments must be a JSON string, got: {}",
        tc_args
    );
    assert_eq!(tc_args.as_str().unwrap(), "{\"paths\":[\"**/*\"]}");

    assert_eq!(converted[5]["role"], "tool");
    assert!(converted[5]["content"].is_string());
}
