/**
 * Typed wrapper around the browser WASM exports.
 *
 * Same API surface as ../wasmBinding.ts, but backed by the browser ESM WASM target.
 * WASM functions return typed JS objects via tsify/wasm-bindgen.
 */

import { raw } from "./wasm.ts";
import type {
  AgentMessage,
  AgentOptions,
  CancelReason,
  CreateAgentOutput,
  EmptyResult,
  EventsOutput,
  EventsResult,
  LlmChunk,
  LlmResult,
  ProjectionInput,
  ProjectionOutput,
  StateOutput,
  StateResult,
  StepOutput,
  StepResult,
  ToolDonePayload,
  ToolExecutionUpdate,
} from "@pi-oxide/pi-host-web";

export type {
  AgentAction,
  AgentContext,
  AgentEvent,
  AgentMessage,
  AgentOptions,
  AgentState,
  AssistantMessage,
  CancelReason,
  Content,
  ContentDelta,
  ContextProjectionBudget,
  ContextProjectionReport,
  ContextProjectionState,
  ContextReplacement,
  ContextStrategy,
  CreateAgentOutput,
  ErrorDto,
  EventsOutput,
  EventsResult,
  HandleOutput,
  ImageContent,
  JsonSchema,
  LlmChunk,
  LlmContext,
  LlmError,
  LlmResult,
  Model,
  ModelCapabilities,
  ModelCost,
  ModelId,
  ModelName,
  ModelProvider,
  Phase,
  ProjectionInput,
  ProjectionOutput,
  ProjectionResult,
  ProviderName,
  QueueMode,
  SessionId,
  StateOutput,
  StateResult,
  StepOutput,
  StepResult,
  StopReason,
  TextContent,
  ThinkingLevel,
  TokenUsage,
  ToolArguments,
  ToolCall,
  ToolCallId,
  ToolDefinition,
  ToolDetails,
  ToolDonePayload,
  ToolError,
  ToolExecutionMode,
  ToolExecutionUpdate,
  ToolName,
  ToolOutputStream,
  ToolResult,
  ToolResultContext,
  ToolResultMessage,
  UserMessage,
  WaitMode,
} from "@pi-oxide/pi-host-web";

// --- Error handling ---

export interface ErrorBody {
  code: string;
  message: string;
}

export class HostError extends Error {
  readonly code: string;
  constructor(body: ErrorBody | undefined) {
    super(body?.message ?? "unknown error");
    this.name = "HostError";
    this.code = body?.code ?? "unknown";
  }
}

function unwrap<T>(result: { ok: boolean; data?: T | null; error?: ErrorBody }): T {
  if (!result.ok) {
    throw new HostError(result.error);
  }
  if (result.data === undefined || result.data === null) {
    return undefined as unknown as T;
  }
  return result.data;
}

// --- Public API ---

export function createAgent(options: AgentOptions): number {
  const data = unwrap<CreateAgentOutput>(raw.createAgent(options));
  return data.handle;
}

export function destroyAgent(handle: number): void {
  unwrap<void>(raw.destroyAgent(handle));
}

export function prompt(handle: number, text: string): StepOutput {
  return unwrap<StepOutput>(raw.prompt(handle, { text }));
}

export function feedLlmChunk(handle: number, chunk: LlmChunk): EventsOutput {
  return unwrap<EventsOutput>(raw.feedLlmChunk(handle, chunk));
}

export function onLlmDone(handle: number, result: LlmResult): StepOutput {
  return unwrap<StepOutput>(raw.onLlmDone(handle, result));
}

export function onToolDone(
  handle: number,
  toolCallId: string,
  payload: ToolDonePayload,
): StepOutput {
  return unwrap<StepOutput>(raw.onToolDone(handle, toolCallId, payload));
}

export function onToolStarted(handle: number, toolCallId: string): EventsOutput {
  return unwrap<EventsOutput>(raw.onToolStarted(handle, toolCallId));
}

export function onToolUpdate(
  handle: number,
  update: ToolExecutionUpdate,
): EventsOutput {
  return unwrap<EventsOutput>(raw.onToolUpdate(handle, update));
}

export function onToolCancelled(
  handle: number,
  toolCallId: string,
  reason: CancelReason,
): StepOutput {
  return unwrap<StepOutput>(raw.onToolCancelled(handle, toolCallId, reason));
}

export function steer(handle: number, message: AgentMessage): EventsOutput {
  return unwrap<EventsOutput>(raw.steer(handle, message));
}

export function followUp(handle: number, message: AgentMessage): void {
  unwrap<void>(raw.followUp(handle, message));
}

export function projectContext(input: ProjectionInput): ProjectionOutput {
  return unwrap<ProjectionOutput>(raw.projectContext(input));
}

export function drainTraceLog(): string[] {
  return raw.drainTraceLog();
}
