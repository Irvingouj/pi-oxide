/**
 * Typed wrapper around the raw pi-host-web WASM exports.
 *
 * All WASM functions take and return JSON strings using the envelope shape:
 *   success: { ok: true, data: ... }
 *   failure: { ok: false, error: { code, message } }
 *
 * This module parses envelopes and throws HostError on { ok: false }.
 */

import { raw } from "./rawBinding.ts";

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

// --- Agent action / event types matching Rust enums ---

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

export type ToolOutputStream = "stdout" | "stderr" | "status";

export type CancelReason =
  | { type: "user_requested" }
  | { type: "timeout" }
  | { type: "agent_aborted" }
  | { type: "dependency_failed"; cause_tool_call_id: string };

export interface ToolExecutionUpdate {
  tool_call_id: string;
  stream: ToolOutputStream;
  chunk: string;
  sequence: number;
  timestamp: number;
}

export interface StateOutput {
  state: {
    system_prompt: string;
    model: unknown;
    thinking_level: string;
    tools: unknown[];
    messages: unknown[];
    is_streaming: boolean;
    streaming_message: unknown | null;
    pending_tool_calls: string[];
    error_message: string | null;
  };
}

export interface HandleOutput {
  handle: number;
}

// --- Agent options ---

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
    capabilities?: { vision: boolean; json_mode: boolean; function_calling: boolean; streaming: boolean };
    cost?: { input: number; output: number; cache_read: number; cache_write: number };
  };
  thinking_level?: "off" | "minimal" | "low" | "medium" | "high" | "xhigh";
  tools?: unknown[];
  steering_mode?: "one_at_a_time" | "all";
  follow_up_mode?: "one_at_a_time" | "all";
  tool_execution_mode?: "parallel" | "sequential";
  session_id?: string | null;
  messages?: unknown[];
}

// --- Public API ---

export function createAgent(options: AgentOptions): number {
  const data = unwrap<HandleOutput>(raw.createAgent(JSON.stringify(options)));
  return data.handle;
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
  result: unknown
): StepOutput {
  return unwrap<StepOutput>(
    raw.onToolDone(handle, toolCallId, JSON.stringify(result))
  );
}

export function onToolStarted(handle: number, toolCallId: string): EventsOutput {
  return unwrap<EventsOutput>(raw.onToolStarted(handle, toolCallId));
}

export function onToolUpdate(
  handle: number,
  update: ToolExecutionUpdate
): EventsOutput {
  return unwrap<EventsOutput>(
    raw.onToolUpdate(handle, JSON.stringify(update))
  );
}

export function onToolCancelled(
  handle: number,
  toolCallId: string,
  reason: CancelReason
): StepOutput {
  return unwrap<StepOutput>(
    raw.onToolCancelled(handle, toolCallId, JSON.stringify(reason))
  );
}

export function steer(handle: number, message: unknown): EventsOutput {
  return unwrap<EventsOutput>(raw.steer(handle, JSON.stringify(message)));
}

export function followUp(handle: number, message: unknown): void {
  unwrap<{}>(raw.followUp(handle, JSON.stringify(message)));
}

export function state(handle: number): StateOutput {
  return unwrap<StateOutput>(raw.state(handle));
}

export function reset(handle: number): void {
  unwrap<{}>(raw.reset(handle));
}

export function destroyAgent(handle: number): void {
  unwrap<{}>(raw.destroyAgent(handle));
}
