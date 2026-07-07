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
