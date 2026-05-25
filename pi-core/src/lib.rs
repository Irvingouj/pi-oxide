//! pi-core: Pure synchronous agent-core state machine.
//!
//! No async runtime, no I/O, no network. All platform capabilities are injected
//! by the host at construction time. The host drives the event loop and calls
//! into core via synchronous callbacks after completing async operations.

pub mod agent;
pub mod context;
pub mod context_metadata;
pub mod context_projection;
pub mod events;
pub mod llm;
pub mod message;
pub mod session;
pub mod timestamp;
pub mod tool;
pub mod types;

pub use agent::{Agent, AgentOptions, AgentState, Phase};
pub use context::{AgentContext, LlmContext};
pub use context_metadata::{fallback_strategy, ContentKind, ContextStrategy, ToolResultContext};
pub use context_projection::{
    estimate_tokens, estimate_tokens_for_text, project, ApiUsageSnapshot, ContextProjectionBudget,
    ContextProjectionReport, ContextProjectionState, ContextReplacement, ProjectionInput,
    ProjectionOutput,
};
pub use events::{
    AgentAction, AgentEvent, CancelReason, ContentDelta, QueueMode,
    ThinkingLevel, ToolExecutionUpdate, ToolOutputStream, WaitMode,
};
pub use llm::{LlmChunk, LlmError, LlmResult, Model, ModelCapabilities, ModelCost, ModelProvider};
pub use message::StopReason;
pub use message::{
    AgentMessage, AssistantMessage, Content, ImageContent, TextContent, ToolCall,
    ToolResultMessage, UserMessage,
};
pub use session::{BranchSummary, EntryKind, SessionEntry, SessionError, SessionState, SessionStorage};
pub use tool::{ExecutionMode, ToolDefinition, ToolError, ToolExecutionMode, ToolResult};
pub use types::{
    ApiName, JsonSchema, ModelId, ModelName, ProviderName, SessionId, ToolArguments, ToolCallId,
    ToolDetails, ToolName,
};
