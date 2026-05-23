/**
 * Typed wrapper around the browser WASM exports.
 *
 * Same API surface as ../wasmBinding.ts, but backed by the browser ESM WASM target.
 * All WASM functions take and return JSON strings using the envelope shape:
 *   success: { ok: true, data: ... }
 *   failure: { ok: false, error: { code, message } }
 */

import { raw } from "./wasm.ts";

// --- Types derived from Rust serde shapes ---

export interface ErrorBody {
  code: string;
  message: string;
}

export interface Envelope<T> {
  ok: boolean;
  data?: T;
  error?: ErrorBody;
}

export class HostError extends Error {
  readonly code: string;
  constructor(body: ErrorBody) {
    super(body.message);
    this.name = "HostError";
    this.code = body.code;
  }
}

function unwrap<T>(json: string): T {
  const env: Envelope<T> = JSON.parse(json);
  if (!env.ok) {
    throw new HostError(env.error!);
  }
  return env.data as T;
}

// --- Agent action / event types ---

export interface StreamLlmAction {
  type: "stream_llm";
  context: {
    system_prompt: string;
    messages: unknown[];
    tools: unknown[];
  };
  session_id: string | null;
}

export interface ExecuteToolsAction {
  type: "execute_tools";
  calls: ToolCall[];
}

export interface CancelToolsAction {
  type: "cancel_tools";
  tool_call_ids: string[];
  reason: CancelReason;
}

export interface WaitForInputAction {
  type: "wait_for_input";
  mode: "steering" | "follow_up" | "any";
}

export interface FinishedAction {
  type: "finished";
  messages: unknown[];
}

export type AgentAction =
  | StreamLlmAction
  | ExecuteToolsAction
  | CancelToolsAction
  | WaitForInputAction
  | FinishedAction;

export type AgentEvent = Record<string, unknown> & { type: string };

export interface ToolCall {
  id: string;
  name: string;
  arguments: Record<string, unknown>;
}

export interface StepOutput {
  events: AgentEvent[];
  actions: AgentAction[];
}

export interface EventsOutput {
  events: AgentEvent[];
}

export interface HandleOutput {
  handle: number;
}

export interface AgentOptions {
  system_prompt: string;
  model: {
    id: string;
    name: string;
    api: string;
    provider: string;
    base_url?: string | null;
    reasoning: boolean;
    context_window: number;
    max_tokens: number;
  };
  thinking_level?: "off" | "minimal" | "low" | "medium" | "high" | "xhigh";
  tools?: unknown[];
  messages?: unknown[];
}

// --- Public API ---

export function createAgent(options: AgentOptions): number {
  const data = unwrap<HandleOutput>(raw.createAgent(JSON.stringify(options)));
  return data.handle;
}

export function destroyAgent(handle: number): void {
  unwrap<{}>(raw.destroyAgent(handle));
}

export function prompt(handle: number, text: string): StepOutput {
  return unwrap<StepOutput>(raw.prompt(handle, JSON.stringify({ text })));
}

export function feedLlmChunk(handle: number, chunk: unknown): EventsOutput {
  return unwrap<EventsOutput>(raw.feedLlmChunk(handle, JSON.stringify(chunk)));
}

export function onLlmDone(handle: number, result: unknown): StepOutput {
  return unwrap<StepOutput>(raw.onLlmDone(handle, JSON.stringify(result)));
}

export function onToolDone(
  handle: number,
  toolCallId: string,
  result: unknown,
): StepOutput {
  return unwrap<StepOutput>(
    raw.onToolDone(handle, toolCallId, JSON.stringify(result)),
  );
}

export function onToolStarted(handle: number, toolCallId: string): EventsOutput {
  return unwrap<EventsOutput>(raw.onToolStarted(handle, toolCallId));
}

export function projectContext(inputJson: string): string {
  return raw.projectContext(inputJson);
}

export interface ProjectionOutput {
  projected_messages: unknown[];
  updated_state: { replacements: Record<string, unknown> };
  report: { estimated_tokens: number; replacements: unknown[]; dropped_messages: number };
}

export function projectContextTyped(input: {
  system_prompt: string;
  messages: unknown[];
  budget: { max_tool_result_chars: number; max_context_tokens: number; default_preview_chars: number };
  state: { replacements: Record<string, unknown> };
}): ProjectionOutput {
  return unwrap<ProjectionOutput>(raw.projectContext(JSON.stringify(input)));
}

export function drainTraceLog(): string {
  return raw.drainTraceLog();
}
