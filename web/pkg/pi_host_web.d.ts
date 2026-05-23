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
 * Notify the agent that a tool has finished executing.
 */
export function onToolDone(handle: number, tool_call_id: string, result_json: string): string;

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
