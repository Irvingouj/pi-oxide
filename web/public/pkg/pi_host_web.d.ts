/* tslint:disable */
/* eslint-disable */

/**
 * Create a new agent from an `AgentOptions` JSON string.
 * Returns `{ ok: true, data: { handle } }` or an error envelope.
 */
export function createAgent(options_json: string): string;

/**
 * Destroy an agent and free its resources.
 */
export function destroyAgent(handle: number): string;

/**
 * Drain accumulated trace log messages and return them as a JSON array.
 */
export function drainTraceLog(): string;

/**
 * Feed a streaming LLM chunk.
 */
export function feedLlmChunk(handle: number, chunk_json: string): string;

/**
 * Queue a follow-up message for after the run would otherwise stop.
 */
export function followUp(handle: number, message_json: string): string;

/**
 * Notify the agent that the LLM stream has finished.
 */
export function onLlmDone(handle: number, result_json: string): string;

/**
 * Notify the agent that a tool was cancelled.
 */
export function onToolCancelled(handle: number, tool_call_id: string, reason_json: string): string;

/**
 * Notify the agent that a tool has finished executing.
 */
export function onToolDone(handle: number, tool_call_id: string, result_json: string): string;

/**
 * Notify the agent that a tool has started executing.
 */
export function onToolStarted(handle: number, tool_call_id: string): string;

/**
 * Send a streaming tool execution update to the agent.
 * Input JSON must match `ToolExecutionUpdate`.
 */
export function onToolUpdate(handle: number, update_json: string): string;

/**
 * Project context: run the Rust context projection engine.
 *
 * Input JSON must match `ProjectionInput`:
 * ```json
 * {
 *   "system_prompt": "...",
 *   "messages": [...],
 *   "budget": { "max_tool_result_chars": 50000, "max_context_tokens": 100000, "default_preview_chars": 2000 },
 *   "state": { "replacements": {} }
 * }
 * ```
 *
 * Returns:
 * ```json
 * { "ok": true, "data": { "projected_messages": [...], "updated_state": {...}, "report": {...} } }
 * ```
 */
export function projectContext(input_json: string): string;

/**
 * Start a new turn with a prompt.
 * `prompt_json` can be a full `AgentMessage` or `{ "text": "..." }`.
 */
export function prompt(handle: number, prompt_json: string): string;

/**
 * Reset the agent state.
 */
export function reset(handle: number): string;

/**
 * Get a read-only snapshot of the agent state.
 */
export function state(handle: number): string;

/**
 * Inject a steering message mid-run.
 */
export function steer(handle: number, message_json: string): string;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly createAgent: (a: number, b: number) => [number, number];
    readonly destroyAgent: (a: number) => [number, number];
    readonly drainTraceLog: () => [number, number];
    readonly feedLlmChunk: (a: number, b: number, c: number) => [number, number];
    readonly followUp: (a: number, b: number, c: number) => [number, number];
    readonly onLlmDone: (a: number, b: number, c: number) => [number, number];
    readonly onToolCancelled: (a: number, b: number, c: number, d: number, e: number) => [number, number];
    readonly onToolDone: (a: number, b: number, c: number, d: number, e: number) => [number, number];
    readonly onToolStarted: (a: number, b: number, c: number) => [number, number];
    readonly onToolUpdate: (a: number, b: number, c: number) => [number, number];
    readonly projectContext: (a: number, b: number) => [number, number];
    readonly prompt: (a: number, b: number, c: number) => [number, number];
    readonly reset: (a: number) => [number, number];
    readonly state: (a: number) => [number, number];
    readonly steer: (a: number, b: number, c: number) => [number, number];
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
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
