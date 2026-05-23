use serde::{Deserialize, Serialize};

/// Describes a concrete LLM model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub api: String,
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    pub reasoning: bool,
    pub context_window: u32,
    pub max_tokens: u32,
    #[serde(default)]
    pub capabilities: ModelCapabilities,
    #[serde(default)]
    pub cost: ModelCost,
}

/// Capabilities advertised by a model.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct ModelCapabilities {
    pub vision: bool,
    pub json_mode: bool,
    pub function_calling: bool,
    pub streaming: bool,
}

/// Per-token cost estimate.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct ModelCost {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

/// Supported LLM providers.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelProvider {
    OpenAi,
    Anthropic,
    Google,
    Ollama,
    Custom,
}
