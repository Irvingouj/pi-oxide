//! pi-core: Pure synchronous agent-core state machine.
//!
//! No async runtime, no I/O, no network. All platform capabilities are injected
//! by the host at construction time. The host drives the event loop and calls
//! into core via synchronous callbacks after completing async operations.

pub mod agent;
#[allow(non_snake_case)]
pub mod agent_runtime;
pub mod context;
pub mod context_projection;
pub mod events;
pub mod llm;
pub mod message;
pub mod session;
pub mod timestamp;
pub mod tool;
pub mod types;

pub use agent::{Agent, AgentOptions, AgentState, Phase};
pub use agent_runtime::{
    AbortedAgent, AgentRuntime, CompactingAgent, ContinueTurnTransition, ExecutingToolsAgent,
    FinishLlmTransition, FinishedAgent, IdleAgent, PreToolCallAgent, ReadyAgent,
    StartTurnTransition, StreamingAgent, ToolTransition, Transition, UserInputDuringTools,
};
pub use context::{AgentContext, LlmContext};
pub use context_projection::{
    build_llm_context_from_trimmed, count_message_chars, default_compaction_threshold,
    default_microcompact_after_turns, estimate_tokens, estimate_tokens_for_text,
    estimate_tokens_for_trimmed, projection_scan, projection_strategy, ChangeMarker,
    ContextProjectionBudget, NewProjectionStrategy,
};
pub use events::{
    AgentAction, AgentEvent, CancelReason, ContentDelta, QueueMode, ThinkingLevel,
    ToolExecutionUpdate, ToolOutputStream, WaitMode,
};
pub use llm::{LlmChunk, LlmError, LlmResult, Model, ModelCapabilities, ModelCost, ModelProvider};
pub use message::StopReason;
pub use message::{
    AgentMessage, Artifacts, AssistantMessage, CompactionSummary, Content, ImageContent,
    OriginalToolResult, ProjectedToolResult, TextContent, ToolCall, ToolResultMessage,
    TrimmedMessage, UserMessage,
};
pub use session::{apply_compaction, build_summary_messages, plan_compaction, CompactionPlan};
pub use tool::{
    ExecutionMode, ToolCallPermission, ToolCallPreparation, ToolCallTransform, ToolDefinition,
    ToolError, ToolResult, ToolRunMode,
};
pub use types::{
    ApiName, JsonSchema, ModelId, ModelName, ProviderName, SessionId, ToolArguments, ToolCallId,
    ToolDetails, ToolName,
};
