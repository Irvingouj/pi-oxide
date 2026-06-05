use serde::{Deserialize, Serialize};
use tsify::Tsify;

// ---------------------------------------------------------------------------
// Serde roundtrip helper — avoids hand-written field-by-field conversions.
// Only used at the WASM boundary, not in hot loops.
// ---------------------------------------------------------------------------

fn to_dto<T, U>(v: T) -> Result<U, serde_json::Error>
where
    T: Serialize,
    U: for<'de> Deserialize<'de>,
{
    serde_json::from_value(serde_json::to_value(v)?)
}

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct ErrorDto {
    pub code: String,
    pub message: String,
}

// ---------------------------------------------------------------------------
// Newtype wrappers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[serde(transparent)]
pub struct ApiName(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[serde(transparent)]
pub struct ModelId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[serde(transparent)]
pub struct ModelName(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[serde(transparent)]
pub struct ProviderName(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[serde(transparent)]
pub struct SessionId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[serde(transparent)]
pub struct ToolCallId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[serde(transparent)]
pub struct ToolName(pub String);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[serde(transparent)]
pub struct JsonSchema(#[tsify(type = "Record<string, unknown>")] pub serde_json::Value);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[serde(transparent)]
pub struct ToolArguments(#[tsify(type = "unknown")] pub serde_json::Value);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[serde(transparent)]
pub struct ToolDetails(#[tsify(type = "Record<string, unknown>")] pub serde_json::Value);

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct Model {
    pub id: ModelId,
    pub name: ModelName,
    pub api: ApiName,
    pub provider: ProviderName,
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

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Tsify, Default)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct ModelCapabilities {
    pub vision: bool,
    pub json_mode: bool,
    pub function_calling: bool,
    pub streaming: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Tsify, Default)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct ModelCost {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[serde(rename_all = "snake_case")]
pub enum ModelProvider {
    OpenAi,
    Anthropic,
    Google,
    Ollama,
    Custom,
}

// ---------------------------------------------------------------------------
// Message types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct UserMessage {
    pub content: Vec<Content>,
    pub timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct AssistantMessage {
    pub content: Vec<Content>,
    pub api: ApiName,
    pub provider: ProviderName,
    pub model: ModelId,
    pub stop_reason: StopReason,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub timestamp: u64,
    pub usage: TokenUsage,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct ToolResultMessage {
    #[serde(skip)]
    pub role: String,
    pub tool_call_id: ToolCallId,
    pub tool_name: ToolName,
    pub content: Vec<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<ToolDetails>,
    pub is_error: bool,
    pub timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum AgentMessage {
    User(UserMessage),
    Assistant(AssistantMessage),
    ToolResult(ToolResultMessage),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Content {
    Text(TextContent),
    Image(ImageContent),
    ToolCall(ToolCall),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct TextContent {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct ImageContent {
    pub media_type: String,
    pub data: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct ToolCall {
    pub id: ToolCallId,
    pub name: ToolName,
    pub arguments: ToolArguments,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify, Default)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct TokenUsage {
    pub input: u32,
    pub output: u32,
    pub cache_read: u32,
    pub cache_write: u32,
    pub total_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    ToolUse,
    Aborted,
    Error,
}

// ---------------------------------------------------------------------------
// LLM types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LlmChunk {
    Start {
        #[serde(flatten)]
        partial: AssistantMessage,
    },
    TextDelta {
        text: String,
    },
    ThinkingDelta {
        text: String,
    },
    ToolCallDelta {
        tool_call_id: ToolCallId,
        #[tsify(type = "Record<string, unknown>")]
        delta: serde_json::Value,
    },
    Done,
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub enum LlmResult {
    Ok(AssistantMessage),
    Err { error: LlmError, aborted: bool },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct LlmError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[tsify(type = "unknown")]
    pub details: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Tool types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct ToolDefinition {
    pub name: ToolName,
    pub label: String,
    pub description: String,
    pub parameters: JsonSchema,
    #[serde(rename = "execution_mode", default)]
    pub execution_mode: ExecutionMode,
    #[serde(rename = "tool_run_mode", default)]
    pub tool_run_mode: ToolRunMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Tsify, Default)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    #[default]
    Parallel,
    Sequential,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Tsify, Default)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[serde(rename_all = "snake_case")]
pub enum ToolRunMode {
    #[default]
    Immediate,
    Deferred,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct ToolResult {
    pub content: Vec<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<ToolDetails>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminate: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct ToolError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<ToolDetails>,
}

// ---------------------------------------------------------------------------
// Event types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[serde(rename_all = "snake_case")]
pub enum ToolOutputStream {
    Stdout,
    Stderr,
    Status,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CancelReason {
    UserRequested,
    Timeout,
    AgentAborted,
    DependencyFailed { cause_tool_call_id: ToolCallId },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChangeMarkerDto {
    CompactionApplied,
    NewArtifacts { entry_ids: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct ToolExecutionUpdate {
    pub tool_call_id: ToolCallId,
    pub stream: ToolOutputStream,
    pub chunk: String,
    pub sequence: u64,
    pub timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct ToolCallPreparation {
    pub tool_call_id: ToolCallId,
    pub transform: ToolCallTransform,
    pub permission: ToolCallPermission,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolCallTransform {
    None,
    RewriteArgs { arguments: ToolArguments },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolCallPermission {
    Allow,
    Block { reason: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostDirective {
    StreamLlm {
        context: LlmContext,
    },
    PrepareToolCalls {
        calls: Vec<ToolCall>,
    },
    ExecuteTools {
        calls: Vec<ToolCall>,
    },
    CancelTools {
        tool_call_ids: Vec<ToolCallId>,
        reason: CancelReason,
    },
    Persist,
    Summarize {
        context: LlmContext,
    },
    Finished,
    WaitForInput {
        mode: WaitMode,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    AgentStart,
    AgentEnd,
    TurnStart,
    TurnEnd {
        message: AgentMessage,
        tool_results: Vec<ToolResultMessage>,
    },
    MessageStart {
        message: AgentMessage,
    },
    MessageUpdate {
        message: AgentMessage,
        delta: ContentDelta,
    },
    MessageEnd {
        message: AgentMessage,
    },
    ToolExecutionStart {
        tool_call_id: ToolCallId,
        tool_name: ToolName,
        #[serde(skip_serializing_if = "Option::is_none")]
        args: Option<ToolArguments>,
    },
    ToolExecutionUpdate {
        tool_call_id: ToolCallId,
        stream: ToolOutputStream,
        chunk: String,
        sequence: u64,
        timestamp: u64,
    },
    ToolExecutionEnd {
        tool_call_id: ToolCallId,
        tool_name: ToolName,
        result: ToolResult,
        #[serde(skip_serializing_if = "Option::is_none")]
        args: Option<ToolArguments>,
        is_error: bool,
    },
    ToolExecutionCancelled {
        tool_call_id: ToolCallId,
        reason: CancelReason,
    },
    QueueUpdate {
        steer: Vec<AgentMessage>,
        follow_up: Vec<AgentMessage>,
    },
    SavePoint {
        had_pending_writes: bool,
    },
    Settled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContentDelta {
    TextStart,
    TextDelta {
        text: String,
    },
    TextEnd,
    ThinkingStart,
    ThinkingDelta {
        text: String,
    },
    ThinkingEnd,
    ToolCallStart {
        tool_call: ToolCall,
    },
    ToolCallDelta {
        tool_call_id: ToolCallId,
        #[tsify(type = "Record<string, unknown>")]
        delta: serde_json::Value,
    },
    ToolCallEnd {
        tool_call_id: ToolCallId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Tsify, Default)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[serde(rename_all = "snake_case")]
pub enum QueueMode {
    #[default]
    OneAtATime,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Tsify, Default)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingLevel {
    #[default]
    Off,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[serde(rename_all = "snake_case")]
pub enum WaitMode {
    Steering,
    FollowUp,
    Any,
}

// ---------------------------------------------------------------------------
// Context types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct AgentContext {
    pub system_prompt: String,
    pub messages: Vec<AgentMessage>,
    pub tools: Vec<ToolDefinition>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct LlmContext {
    pub system_prompt: String,
    pub messages: Vec<AgentMessage>,
    pub tools: Vec<ToolDefinition>,
}

// ---------------------------------------------------------------------------
// Projection types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify, Default)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct ContextProjectionBudget {
    #[serde(default = "default_max_tool_result_chars")]
    pub max_tool_result_chars: usize,
    #[serde(default = "default_max_context_tokens")]
    pub max_context_tokens: usize,
    #[serde(default = "default_microcompact_after_turns")]
    pub microcompact_after_turns: u32,
    #[serde(default = "default_compaction_threshold")]
    pub compaction_threshold: f32,
}

fn default_max_tool_result_chars() -> usize {
    50000
}
fn default_max_context_tokens() -> usize {
    100000
}
fn default_microcompact_after_turns() -> u32 {
    5
}
fn default_compaction_threshold() -> f32 {
    0.75
}

// ---------------------------------------------------------------------------
// Estimate tokens types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct EstimateTokensInput {
    pub messages: Vec<AgentMessage>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct EstimateTokensOutput {
    pub tokens: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct EstimateTokensResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<EstimateTokensOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorDto>,
}

// ---------------------------------------------------------------------------
// Agent types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct AgentOptions {
    pub system_prompt: String,
    pub model: Model,
    #[serde(default)]
    pub thinking_level: ThinkingLevel,
    #[serde(default)]
    pub steering_mode: QueueMode,
    #[serde(default)]
    pub follow_up_mode: QueueMode,
    #[serde(default)]
    pub tool_execution_mode: ExecutionMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
}

// ---------------------------------------------------------------------------
// Flexible input types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct StartTurnInput {
    pub prompt: AgentMessage,
    #[serde(default)]
    pub tools: Vec<ToolDefinition>,
}

// ---------------------------------------------------------------------------
// New API output types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
#[derive(Default)]
pub struct TurnResultOutput {
    pub events: Vec<AgentEvent>,
    pub directives: Vec<HostDirective>,
    #[serde(default)]
    pub markers: Vec<ChangeMarkerDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct TurnResultResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<TurnResultOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct EmptyResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<()>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorDto>,
}

// ---------------------------------------------------------------------------
// HostState result types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct CreateHostStateOutput {
    pub handle: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct CreateHostStateResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<CreateHostStateOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct CreateHostAgentOutput {
    pub handle: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct CreateHostAgentResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<CreateHostAgentOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorDto>,
}

/// PersistData DTO. Uses serde_json::Value for transcript/artifacts since TrimmedMessage/Artifacts
/// are pi-core types without Tsify. The dto_conv! macro handles serde roundtrip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct PersistData {
    /// TrimmedMessage list as JSON value (serde roundtrip via dto_conv).
    #[serde(rename = "T", default)]
    #[tsify(type = "unknown[]")]
    pub transcript: serde_json::Value,
    /// Artifacts map as JSON value (serde roundtrip via dto_conv).
    #[serde(rename = "A", default)]
    #[tsify(type = "Record<string, unknown>")]
    pub artifacts: serde_json::Value,
    #[serde(default)]
    pub turn_number: u32,
    #[serde(default)]
    pub host_artifacts: Vec<(String, String)>,
    #[serde(default)]
    pub budget: ContextProjectionBudget,
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub compaction_prompt: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct HostStatePersistDataOutput {
    pub state: PersistData,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct HostStatePersistDataResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<HostStatePersistDataOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Tsify)]
#[tsify(into_wasm_abi, from_wasm_abi)]
pub struct ArtifactSearchResults {
    pub results: Vec<crate::host_state::ArtifactSearchResult>,
}

// ---------------------------------------------------------------------------
// From/Into — serde roundtrip for every DTO ↔ pi-core mapping
// ---------------------------------------------------------------------------

macro_rules! dto_conv {
    ($dto:ty, $core:ty) => {
        impl TryFrom<$core> for $dto {
            type Error = serde_json::Error;
            fn try_from(v: $core) -> Result<Self, serde_json::Error> {
                to_dto(v)
            }
        }
        impl TryFrom<$dto> for $core {
            type Error = serde_json::Error;
            fn try_from(v: $dto) -> Result<Self, serde_json::Error> {
                to_dto(v)
            }
        }
    };
}

dto_conv!(ApiName, pi_core::ApiName);
dto_conv!(ModelId, pi_core::ModelId);
dto_conv!(ModelName, pi_core::ModelName);
dto_conv!(ProviderName, pi_core::ProviderName);
dto_conv!(SessionId, pi_core::SessionId);
dto_conv!(ToolCallId, pi_core::ToolCallId);
dto_conv!(ToolName, pi_core::ToolName);
dto_conv!(JsonSchema, pi_core::JsonSchema);
dto_conv!(ToolArguments, pi_core::ToolArguments);
dto_conv!(ToolDetails, pi_core::ToolDetails);

dto_conv!(Model, pi_core::Model);
dto_conv!(ModelCapabilities, pi_core::ModelCapabilities);
dto_conv!(ModelCost, pi_core::ModelCost);
dto_conv!(ModelProvider, pi_core::ModelProvider);

dto_conv!(UserMessage, pi_core::UserMessage);
dto_conv!(AssistantMessage, pi_core::AssistantMessage);
dto_conv!(ToolResultMessage, pi_core::ToolResultMessage);
dto_conv!(AgentMessage, pi_core::AgentMessage);
dto_conv!(Content, pi_core::Content);
dto_conv!(TextContent, pi_core::TextContent);
dto_conv!(ImageContent, pi_core::ImageContent);
dto_conv!(ToolCall, pi_core::ToolCall);
dto_conv!(TokenUsage, pi_core::message::TokenUsage);
dto_conv!(StopReason, pi_core::StopReason);

dto_conv!(LlmChunk, pi_core::LlmChunk);
dto_conv!(LlmResult, pi_core::LlmResult);
dto_conv!(LlmError, pi_core::LlmError);

dto_conv!(ToolDefinition, pi_core::ToolDefinition);
dto_conv!(ExecutionMode, pi_core::ExecutionMode);
dto_conv!(ToolRunMode, pi_core::ToolRunMode);
dto_conv!(ToolResult, pi_core::ToolResult);
dto_conv!(ToolError, pi_core::ToolError);

dto_conv!(ToolCallPreparation, pi_core::ToolCallPreparation);
dto_conv!(ToolCallTransform, pi_core::ToolCallTransform);
dto_conv!(ToolCallPermission, pi_core::ToolCallPermission);

dto_conv!(ToolOutputStream, pi_core::ToolOutputStream);
dto_conv!(CancelReason, pi_core::CancelReason);
dto_conv!(ToolExecutionUpdate, pi_core::ToolExecutionUpdate);
dto_conv!(AgentEvent, pi_core::AgentEvent);
dto_conv!(ContentDelta, pi_core::ContentDelta);
dto_conv!(QueueMode, pi_core::QueueMode);
dto_conv!(ThinkingLevel, pi_core::ThinkingLevel);
dto_conv!(WaitMode, pi_core::WaitMode);

dto_conv!(AgentContext, pi_core::AgentContext);
dto_conv!(LlmContext, pi_core::LlmContext);

dto_conv!(ContextProjectionBudget, pi_core::ContextProjectionBudget);

dto_conv!(AgentOptions, pi_core::AgentOptions);

dto_conv!(ChangeMarkerDto, pi_core::ChangeMarker);

dto_conv!(PersistData, crate::host_state::PersistData);
