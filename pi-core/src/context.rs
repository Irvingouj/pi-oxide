use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::message::AgentMessage;
use crate::tool::ToolDefinition;

/// Context snapshot passed into the agent loop for a single turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct AgentContext {
    pub system_prompt: String,
    pub messages: Vec<AgentMessage>,
    pub tools: Vec<ToolDefinition>,
}

/// Context sent to the LLM provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct LlmContext {
    pub system_prompt: String,
    pub messages: Vec<AgentMessage>,
    pub tools: Vec<ToolDefinition>,
}
