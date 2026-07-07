use super::{LlmClient, ModelInfo, ModelDiscovery, WireFormat};

impl LlmClient {
    fn list_models_openai(&self) -> Result<Vec<ModelInfo>, Box<dyn std::error::Error>> {
        let url = format!("{}/v1/models", self.base_url);

        let resp = self
            .client
            .get(&url)
            .header("authorization", format!("Bearer {}", self.api_key))
            .send()?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            return Err(format!("API error {status}: {text}").into());
        }

        let body: serde_json::Value = resp.json()?;

        let models: Vec<ModelInfo> = body
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

        Ok(models)
    }

    fn list_models_anthropic(&self) -> Vec<ModelInfo> {
        // Anthropic has no /v1/models endpoint. Return known current models.
        [
            "claude-sonnet-5",
            "claude-opus-4",
            "claude-haiku-4",
            "claude-sonnet-4-20250514",
            "claude-opus-4-20250514",
            "claude-haiku-4-20250711",
        ]
        .into_iter()
        .map(|id| ModelInfo { id: id.to_string() })
        .collect()
    }
}

impl ModelDiscovery for LlmClient {
    fn list_models(&self) -> Result<Vec<ModelInfo>, Box<dyn std::error::Error>> {
        match self.wire_format {
            WireFormat::OpenAI => self.list_models_openai(),
            WireFormat::Anthropic => Ok(self.list_models_anthropic()),
        }
    }
}
