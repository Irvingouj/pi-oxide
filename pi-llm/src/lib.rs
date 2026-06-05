//! pi-llm: LLM provider protocol definitions.
//!
//! Pure types and traits. No network implementation.
//! Hosts bring their own HTTP clients (reqwest, fetch(), etc.)

pub mod schema;
pub mod stream;

pub use pi_core::{Model, ModelCapabilities, ModelCost, ModelProvider};
pub use schema::json_schema_for;
pub use stream::{LlmEvent, LlmStream, StreamError, StreamOptions};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_serialization_roundtrip() {
        let model = Model {
            id: "gpt-4".into(),
            name: "GPT-4".into(),
            api: "openai".into(),
            provider: "openai".into(),
            base_url: Some("https://api.example.com".into()),
            reasoning: true,
            context_window: 128000,
            max_tokens: 4096,
            capabilities: ModelCapabilities {
                vision: true,
                json_mode: true,
                function_calling: true,
                streaming: true,
            },
            cost: ModelCost {
                input: 0.01,
                output: 0.03,
                cache_read: 0.005,
                cache_write: 0.01,
            },
        };
        let json = serde_json::to_string(&model).unwrap();
        let decoded: Model = serde_json::from_str(&json).unwrap();
        assert_eq!(model, decoded);
    }

    #[test]
    fn model_capabilities_default() {
        let caps: ModelCapabilities = Default::default();
        assert!(!caps.vision);
        assert!(!caps.json_mode);
        assert!(!caps.function_calling);
        assert!(!caps.streaming);
    }

    #[test]
    fn model_cost_default() {
        let cost: ModelCost = Default::default();
        assert_eq!(cost.input, 0.0);
        assert_eq!(cost.output, 0.0);
        assert_eq!(cost.cache_read, 0.0);
        assert_eq!(cost.cache_write, 0.0);
    }

    #[test]
    fn model_provider_serialization_roundtrip() {
        for provider in [
            ModelProvider::OpenAi,
            ModelProvider::Anthropic,
            ModelProvider::Google,
            ModelProvider::Ollama,
            ModelProvider::Custom,
        ] {
            let json = serde_json::to_string(&provider).unwrap();
            let decoded: ModelProvider = serde_json::from_str(&json).unwrap();
            assert_eq!(provider, decoded);
        }
    }

    #[test]
    fn model_provider_snake_case_tags() {
        assert_eq!(
            serde_json::to_string(&ModelProvider::OpenAi).unwrap(),
            "\"open_ai\""
        );
        assert_eq!(
            serde_json::to_string(&ModelProvider::Anthropic).unwrap(),
            "\"anthropic\""
        );
        assert_eq!(
            serde_json::to_string(&ModelProvider::Custom).unwrap(),
            "\"custom\""
        );
    }

    #[test]
    fn stream_options_serialization_roundtrip() {
        let options = StreamOptions {
            api_key: Some("sk-test".into()),
            session_id: Some("sess-1".into()),
            timeout_ms: Some(30000),
            max_retries: Some(3),
            headers: Some([("X-Custom".into(), "value".into())].into_iter().collect()),
            metadata: Some(serde_json::json!({"foo": "bar"})),
        };
        let json = serde_json::to_string(&options).unwrap();
        let decoded: StreamOptions = serde_json::from_str(&json).unwrap();
        assert_eq!(options, decoded);
    }

    #[test]
    fn stream_options_default_is_empty() {
        let options: StreamOptions = Default::default();
        let json = serde_json::to_string(&options).unwrap();
        // api_key does not skip_serializing_if, so it appears as null
        assert_eq!(json, "{\"api_key\":null}");
    }

    #[test]
    fn llm_event_serialization_roundtrip() {
        let events = vec![
            LlmEvent::Start {
                model: "gpt-4".into(),
            },
            LlmEvent::TextDelta {
                text: "hello".into(),
            },
            LlmEvent::ThinkingDelta {
                text: "thinking...".into(),
            },
            LlmEvent::ToolCallStart {
                tool_call_id: "tc-1".into(),
                name: "read".into(),
            },
            LlmEvent::ToolCallDelta {
                tool_call_id: "tc-1".into(),
                delta: serde_json::json!({"path": "/foo"}),
            },
            LlmEvent::ToolCallEnd {
                tool_call_id: "tc-1".into(),
            },
            LlmEvent::Usage {
                input: 10,
                output: 5,
            },
            LlmEvent::Done,
            LlmEvent::Error {
                message: "something went wrong".into(),
            },
        ];
        for event in events {
            let json = serde_json::to_string(&event).unwrap();
            let decoded: LlmEvent = serde_json::from_str(&json).unwrap();
            assert_eq!(event, decoded);
        }
    }

    #[test]
    fn llm_event_kind_tags() {
        assert!(serde_json::to_string(&LlmEvent::Done)
            .unwrap()
            .contains("\"kind\":\"done\""));
        assert!(
            serde_json::to_string(&LlmEvent::TextDelta { text: "".into() })
                .unwrap()
                .contains("\"kind\":\"text_delta\"")
        );
        assert!(serde_json::to_string(&LlmEvent::ToolCallStart {
            tool_call_id: "".into(),
            name: "".into()
        })
        .unwrap()
        .contains("\"kind\":\"tool_call_start\""));
    }

    #[test]
    fn stream_error_serialization_roundtrip() {
        let errors = vec![
            StreamError::Network("timeout".into()),
            StreamError::Auth("invalid key".into()),
            StreamError::RateLimited,
            StreamError::Model("overloaded".into()),
            StreamError::Aborted,
        ];
        for error in errors {
            let json = serde_json::to_string(&error).unwrap();
            let decoded: StreamError = serde_json::from_str(&json).unwrap();
            assert_eq!(error, decoded);
        }
    }

    #[test]
    fn stream_error_display_messages() {
        assert_eq!(
            StreamError::Network("timeout".into()).to_string(),
            "network error: timeout"
        );
        assert_eq!(
            StreamError::Auth("bad key".into()).to_string(),
            "auth error: bad key"
        );
        assert_eq!(StreamError::RateLimited.to_string(), "rate limited");
        assert_eq!(
            StreamError::Model("down".into()).to_string(),
            "model error: down"
        );
        assert_eq!(StreamError::Aborted.to_string(), "aborted");
    }

    #[test]
    fn json_schema_for_generates_valid_schema() {
        let schema = json_schema_for(
            "Test parameters",
            &["name"],
            &[("name".into(), serde_json::json!({"type": "string"}))],
        );
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["description"], "Test parameters");
        assert_eq!(schema["required"], serde_json::json!(["name"]));
        assert_eq!(schema["properties"]["name"]["type"], "string");
    }

    #[test]
    fn json_schema_for_empty_required() {
        let schema = json_schema_for("No params", &[], &[]);
        assert_eq!(schema["required"], serde_json::json!([]));
        assert_eq!(schema["properties"], serde_json::json!({}));
    }

    #[test]
    fn llm_stream_constructed_from_events() {
        let model = Model {
            id: "test".into(),
            name: "Test".into(),
            api: "test".into(),
            provider: "test".into(),
            base_url: None,
            reasoning: false,
            context_window: 1000,
            max_tokens: 100,
            capabilities: Default::default(),
            cost: Default::default(),
        };
        let stream = LlmStream {
            model: model.clone(),
            events: vec![
                LlmEvent::Start {
                    model: "test".into(),
                },
                LlmEvent::Done,
            ],
        };
        assert_eq!(stream.model.id, "test".into());
        assert_eq!(stream.events.len(), 2);
    }
}
