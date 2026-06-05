/* tslint:disable */
/* eslint-disable */
export interface AgentContext {
    system_prompt: string;
    messages: AgentMessage[];
    tools: ToolDefinition[];
}

export interface AgentOptions {
    system_prompt: string;
    model: Model;
    thinking_level?: ThinkingLevel;
    steering_mode?: QueueMode;
    follow_up_mode?: QueueMode;
    tool_execution_mode?: ExecutionMode;
    session_id?: SessionId;
}

export interface ArtifactSearchResult {
    id: string;
    snippet: string;
    match_count: number;
}

export interface ArtifactSearchResults {
    results: ArtifactSearchResult[];
}

export interface AssistantMessage {
    content: Content[];
    api: ApiName;
    provider: ProviderName;
    model: ModelId;
    stop_reason: StopReason;
    error_message?: string;
    timestamp: number;
    usage: TokenUsage;
}

export interface ContextProjectionBudget {
    max_tool_result_chars?: number;
    max_context_tokens?: number;
    microcompact_after_turns?: number;
    compaction_threshold?: number;
}

export interface CreateHostAgentOutput {
    handle: number;
}

export interface CreateHostAgentResult {
    ok: boolean;
    data?: CreateHostAgentOutput;
    error?: ErrorDto;
}

export interface CreateHostStateOutput {
    handle: number;
}

export interface CreateHostStateResult {
    ok: boolean;
    data?: CreateHostStateOutput;
    error?: ErrorDto;
}

export interface EmptyResult {
    ok: boolean;
    data?: null;
    error?: ErrorDto;
}

export interface ErrorDto {
    code: string;
    message: string;
}

export interface EstimateTokensInput {
    messages: AgentMessage[];
}

export interface EstimateTokensOutput {
    tokens: number;
}

export interface EstimateTokensResult {
    ok: boolean;
    data?: EstimateTokensOutput;
    error?: ErrorDto;
}

export interface HostStatePersistDataOutput {
    state: PersistData;
}

export interface HostStatePersistDataResult {
    ok: boolean;
    data?: HostStatePersistDataOutput;
    error?: ErrorDto;
}

export interface ImageContent {
    media_type: string;
    data: string;
}

export interface LlmContext {
    system_prompt: string;
    messages: AgentMessage[];
    tools: ToolDefinition[];
}

export interface LlmError {
    code: string;
    message: string;
    details?: unknown;
}

export interface Model {
    id: ModelId;
    name: ModelName;
    api: ApiName;
    provider: ProviderName;
    base_url?: string;
    reasoning: boolean;
    context_window: number;
    max_tokens: number;
    capabilities?: ModelCapabilities;
    cost?: ModelCost;
}

export interface ModelCapabilities {
    vision: boolean;
    json_mode: boolean;
    function_calling: boolean;
    streaming: boolean;
}

export interface ModelCost {
    input: number;
    output: number;
    cache_read: number;
    cache_write: number;
}

export interface PersistData {
    T?: unknown[];
    A?: Record<string, unknown>;
    turn_number?: number;
    host_artifacts?: [string, string][];
    budget?: ContextProjectionBudget;
    system_prompt?: string;
    compaction_prompt?: string;
}

export interface StartTurnInput {
    prompt: AgentMessage;
    tools?: ToolDefinition[];
}

export interface TextContent {
    text: string;
}

export interface TokenUsage {
    input: number;
    output: number;
    cache_read: number;
    cache_write: number;
    total_tokens: number;
}

export interface ToolCall {
    id: ToolCallId;
    name: ToolName;
    arguments: ToolArguments;
}

export interface ToolCallPreparation {
    tool_call_id: ToolCallId;
    transform: ToolCallTransform;
    permission: ToolCallPermission;
}

export interface ToolDefinition {
    name: ToolName;
    label: string;
    description: string;
    parameters: JsonSchema;
    execution_mode?: ExecutionMode;
    tool_run_mode?: ToolRunMode;
}

export interface ToolError {
    code: string;
    message: string;
    details?: ToolDetails;
}

export interface ToolExecutionUpdate {
    tool_call_id: ToolCallId;
    stream: ToolOutputStream;
    chunk: string;
    sequence: number;
    timestamp: number;
}

export interface ToolResult {
    content: Content[];
    details?: ToolDetails;
    terminate?: boolean;
}

export interface ToolResultMessage {
    tool_call_id: ToolCallId;
    tool_name: ToolName;
    content: Content[];
    details?: ToolDetails;
    is_error: boolean;
    timestamp: number;
}

export interface TurnResultOutput {
    events: AgentEvent[];
    directives: HostDirective[];
    markers?: ChangeMarkerDto[];
}

export interface TurnResultResult {
    ok: boolean;
    data?: TurnResultOutput;
    error?: ErrorDto;
}

export interface UserMessage {
    content: Content[];
    timestamp: number;
}

export type AgentEvent = { type: "agent_start" } | { type: "agent_end" } | { type: "turn_start" } | { type: "turn_end"; message: AgentMessage; tool_results: ToolResultMessage[] } | { type: "message_start"; message: AgentMessage } | { type: "message_update"; message: AgentMessage; delta: ContentDelta } | { type: "message_end"; message: AgentMessage } | { type: "tool_execution_start"; tool_call_id: ToolCallId; tool_name: ToolName; args?: ToolArguments } | { type: "tool_execution_update"; tool_call_id: ToolCallId; stream: ToolOutputStream; chunk: string; sequence: number; timestamp: number } | { type: "tool_execution_end"; tool_call_id: ToolCallId; tool_name: ToolName; result: ToolResult; args?: ToolArguments; is_error: boolean } | { type: "tool_execution_cancelled"; tool_call_id: ToolCallId; reason: CancelReason } | { type: "queue_update"; steer: AgentMessage[]; follow_up: AgentMessage[] } | { type: "save_point"; had_pending_writes: boolean } | { type: "settled" };

export type AgentMessage = ({ role: "user" } & UserMessage) | ({ role: "assistant" } & AssistantMessage) | ({ role: "tool_result" } & ToolResultMessage);

export type ApiName = string;

export type CancelReason = { type: "user_requested" } | { type: "timeout" } | { type: "agent_aborted" } | { type: "dependency_failed"; cause_tool_call_id: ToolCallId };

export type ChangeMarkerDto = { type: "compaction_applied" } | { type: "new_artifacts"; entry_ids: string[] };

export type Content = ({ type: "text" } & TextContent) | ({ type: "image" } & ImageContent) | ({ type: "tool_call" } & ToolCall);

export type ContentDelta = { kind: "text_start" } | { kind: "text_delta"; text: string } | { kind: "text_end" } | { kind: "thinking_start" } | { kind: "thinking_delta"; text: string } | { kind: "thinking_end" } | { kind: "tool_call_start"; tool_call: ToolCall } | { kind: "tool_call_delta"; tool_call_id: ToolCallId; delta: Record<string, unknown> } | { kind: "tool_call_end"; tool_call_id: ToolCallId };

export type ExecutionMode = "parallel" | "sequential";

export type HostDirective = { type: "stream_llm"; context: LlmContext } | { type: "prepare_tool_calls"; calls: ToolCall[] } | { type: "execute_tools"; calls: ToolCall[] } | { type: "cancel_tools"; tool_call_ids: ToolCallId[]; reason: CancelReason } | { type: "persist" } | { type: "summarize"; context: LlmContext } | { type: "finished" } | { type: "wait_for_input"; mode: WaitMode };

export type JsonSchema = Record<string, unknown>;

export type LlmChunk = ({ kind: "start" } & {} & AssistantMessage) | { kind: "text_delta"; text: string } | { kind: "thinking_delta"; text: string } | { kind: "tool_call_delta"; tool_call_id: ToolCallId; delta: Record<string, unknown> } | { kind: "done" } | { kind: "error"; message: string };

export type LlmResult = { Ok: AssistantMessage } | { Err: { error: LlmError; aborted: boolean } };

export type ModelId = string;

export type ModelName = string;

export type ModelProvider = "open_ai" | "anthropic" | "google" | "ollama" | "custom";

export type ProviderName = string;

export type QueueMode = "one_at_a_time" | "all";

export type SessionId = string;

export type StopReason = "end_turn" | "max_tokens" | "tool_use" | "aborted" | "error";

export type ThinkingLevel = "off" | "minimal" | "low" | "medium" | "high" | "xhigh";

export type ToolArguments = unknown;

export type ToolCallId = string;

export type ToolCallPermission = { type: "allow" } | { type: "block"; reason: string };

export type ToolCallTransform = { type: "none" } | { type: "rewrite_args"; arguments: ToolArguments };

export type ToolDetails = Record<string, unknown>;

export type ToolName = string;

export type ToolOutputStream = "stdout" | "stderr" | "status";

export type ToolRunMode = "immediate" | "deferred";

export type WaitMode = "steering" | "follow_up" | "any";


export function createHostAgent(options: AgentOptions, budget: ContextProjectionBudget): CreateHostAgentResult;

export function createHostState(_budget: ContextProjectionBudget): CreateHostStateResult;

export function destroyHostAgent(handle: number): EmptyResult;

export function destroyHostState(handle: number): EmptyResult;

export function estimateTokens(input: EstimateTokensInput): EstimateTokensResult;

export function estimateTokensForText(text: string): EstimateTokensResult;

export function getHostAgentPersistData(handle: number): HostStatePersistDataResult;

export function getHostStatePersistData(handle: number): HostStatePersistDataResult;

export function hostAbort(handle: number): TurnResultResult;

export function hostAcceptCompaction(handle: number, summary: string, _compacted_entry_ids: string[]): TurnResultResult;

export function hostContinueTurn(handle: number): TurnResultResult;

export function hostFeedLlmChunk(handle: number, chunk: LlmChunk): TurnResultResult;

export function hostLlmDone(handle: number, result: LlmResult): TurnResultResult;

export function hostPrepareToolCalls(handle: number, preparations_json: string): TurnResultResult;

export function hostReadArtifact(handle: number, artifact_id: string): string;

export function hostReset(handle: number): EmptyResult;

export function hostSearchArtifacts(handle: number, query: string): ArtifactSearchResults;

export function hostSteer(handle: number, message: AgentMessage): TurnResultResult;

export function hostToolCancelled(handle: number, tool_call_id: string, reason: CancelReason): TurnResultResult;

export function hostToolDone(handle: number, id: ToolCallId, result: ToolResult): TurnResultResult;

export function hostToolFailed(handle: number, id: ToolCallId, error: ToolError): TurnResultResult;

export function restoreHostAgent(options: AgentOptions, data: PersistData): CreateHostAgentResult;

export function restoreHostState(data: PersistData): CreateHostStateResult;

export function restoreHostStateFromJson(json: string): CreateHostStateResult;

export function setLogLevel(level: string): void;

export function startTurn(handle: number, input: StartTurnInput): TurnResultResult;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly createHostState: (a: any) => any;
    readonly destroyHostState: (a: number) => any;
    readonly estimateTokens: (a: any) => any;
    readonly estimateTokensForText: (a: number, b: number) => any;
    readonly getHostStatePersistData: (a: number) => any;
    readonly hostReadArtifact: (a: number, b: number, c: number) => [number, number, number, number];
    readonly hostSearchArtifacts: (a: number, b: number, c: number) => [number, number, number];
    readonly restoreHostState: (a: any) => any;
    readonly restoreHostStateFromJson: (a: number, b: number) => any;
    readonly setLogLevel: (a: number, b: number) => void;
    readonly createHostAgent: (a: any, b: any) => any;
    readonly destroyHostAgent: (a: number) => any;
    readonly getHostAgentPersistData: (a: number) => any;
    readonly hostAbort: (a: number) => any;
    readonly hostAcceptCompaction: (a: number, b: number, c: number, d: number, e: number) => any;
    readonly hostContinueTurn: (a: number) => any;
    readonly hostFeedLlmChunk: (a: number, b: any) => any;
    readonly hostLlmDone: (a: number, b: any) => any;
    readonly hostPrepareToolCalls: (a: number, b: number, c: number) => any;
    readonly hostReset: (a: number) => any;
    readonly hostSteer: (a: number, b: any) => any;
    readonly hostToolCancelled: (a: number, b: number, c: number, d: any) => any;
    readonly hostToolDone: (a: number, b: any, c: any) => any;
    readonly hostToolFailed: (a: number, b: any, c: any) => any;
    readonly restoreHostAgent: (a: any, b: any) => any;
    readonly startTurn: (a: number, b: any) => any;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
