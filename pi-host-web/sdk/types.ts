// Public type definitions for the pi-oxide SDK.
// No WASM imports in this file — these are pure TypeScript contracts.

import type {
  AgentArtifact,
  AgentArtifactRef,
  ArtifactPolicy,
  ArtifactSearchQuery,
  ArtifactSearchResult,
} from "./artifacts.ts";

export type {
  AgentArtifact,
  AgentArtifactRef,
  ArtifactPolicy,
  ArtifactSearchQuery,
  ArtifactSearchResult,
} from "./artifacts.ts";

export interface AgentConfig {
  sessionId: string;
  model: AgentModel;
  tools?: AgentTools | AgentTools[];
  store?: AgentStore;
  instructions?: string;
  context?: AgentContextPolicy;
  artifacts?: ArtifactPolicy;
  telemetry?: AgentTelemetry;
}

export type AgentInput =
  | string
  | {
      text: string;
      attachments?: AgentAttachment[];
      metadata?: Record<string, unknown>;
    };

export interface AgentAttachment {
  type: string;
  content: string | Uint8Array;
  mimeType?: string;
}

export interface AgentRunOptions {
  signal?: AbortSignal;
  metadata?: Record<string, unknown>;
}

export interface AgentRunResult {
  status: "completed" | "aborted" | "failed";
  message?: AgentMessage;
  text: string;
  toolCalls: AgentToolRun[];
  artifacts: AgentArtifactRef[];
  usage?: TokenUsage;
  error?: AgentError;
}

export type AgentEventName =
  | "messageStart"
  | "text"
  | "messageEnd"
  | "toolStart"
  | "toolUpdate"
  | "toolEnd"
  | "artifact"
  | "status"
  | "done"
  | "error"
  | "debug";

export type AgentEventHandler<E extends AgentEventName> =
  E extends "messageStart" ? (message: AgentMessage) => void :
  E extends "text" ? (delta: string) => void :
  E extends "messageEnd" ? (message: AgentMessage) => void :
  E extends "toolStart" ? (tool: AgentToolRun) => void :
  E extends "toolUpdate" ? (tool: AgentToolRun) => void :
  E extends "toolEnd" ? (tool: AgentToolRun) => void :
  E extends "artifact" ? (artifact: AgentArtifactRef) => void :
  E extends "status" ? (status: AgentStatus) => void :
  E extends "done" ? (result: AgentRunResult) => void :
  E extends "error" ? (error: AgentError) => void :
  E extends "debug" ? (event: unknown) => void :
  never;

export interface AgentMessage {
  id: string;
  role: "user" | "assistant" | "tool_result";
  content: AgentContentBlock[];
  timestamp?: number;
  tool_call_id?: string;
}

export type AgentContentBlock =
  | { type: "text"; text: string }
  | { type: "tool_call"; id: string; name: string; arguments: unknown }
  | { type: "image"; mimeType: string; data: string }
  | { type: "file"; mimeType: string; data: string };

export interface AgentToolRun {
  id: string;
  name: string;
  title?: string;
  input: unknown;
  output?: unknown;
  status: "running" | "completed" | "failed" | "cancelled";
  startedAt: number;
  endedAt?: number;
  error?: AgentError;
}

export interface AgentStatus {
  state:
    | "idle"
    | "loading"
    | "thinking"
    | "calling_model"
    | "running_tool"
    | "saving"
    | "completed"
    | "aborted"
    | "failed";
  message?: string;
}

export interface AgentModel {
  id?: string;
  contextWindow?: number;
  maxTokens?: number;
  capabilities?: {
    vision?: boolean;
    jsonMode?: boolean;
    functionCalling?: boolean;
    streaming?: boolean;
  };
  generate(request: ModelRequest): Promise<ModelResponse>;
  summarize?(messages: AgentMessage[], signal?: AbortSignal): Promise<string>;
}

export interface ModelRequest {
  instructions: string;
  messages: AgentMessage[];
  tools: AgentToolDefinition[];
  signal?: AbortSignal;
  metadata?: Record<string, unknown>;
}

export interface ModelResponse {
  content: AgentContentBlock[];
  stopReason: "end" | "tool_call" | "length" | "error";
  usage?: TokenUsage;
  model?: string;
  raw?: unknown;
}

export interface ModelEvent {
  type: "text_delta" | "tool_call_delta" | "done";
  payload: unknown;
}

export interface AgentTools {
  definitions: AgentToolDefinition[];
  getHandler(name: string): ((input: unknown) => Promise<unknown> | unknown) | null;
}

export interface AgentToolDefinition {
  name: string;
  description: string;
  inputSchema: unknown; // ZodType, but we avoid importing zod in public types
  run: (input: unknown) => Promise<unknown> | unknown;
  details?: (output: unknown) => Record<string, unknown>;
}

export interface AgentStore {
  loadSession(sessionId: string): Promise<AgentSnapshot | null>;
  saveSession(sessionId: string, snapshot: AgentSnapshot): Promise<void>;
  saveArtifact?(sessionId: string, artifact: AgentArtifact): Promise<void>;
  loadArtifact?(sessionId: string, artifactId: string): Promise<AgentArtifact | null>;
  searchArtifacts?(sessionId: string, query: ArtifactSearchQuery): Promise<ArtifactSearchResult[]>;
}

export interface AgentSnapshot {
  version: number;
  data: unknown;
}

export interface AgentContextPolicy {
  maxTokens?: number;
  toolResultLimit?: number;
  summarize?: boolean | AgentSummarizer;
}

export interface AgentSummarizer {
  summarize(messages: AgentMessage[]): Promise<string>;
}

export interface AgentTelemetry {
  onEvent?(event: { type: string; payload: unknown }): void;
  onMetric?(name: string, value: number, metadata?: Record<string, unknown>): void;
}

export interface AgentError {
  code:
    | "model_auth_failed"
    | "model_rate_limited"
    | "model_unavailable"
    | "tool_input_invalid"
    | "tool_failed"
    | "tool_duplicate"
    | "store_load_failed"
    | "store_save_failed"
    | "store_artifact_unsupported"
    | "snapshot_invalid"
    | "aborted"
    | "internal_error"
    | "agent_disposed"
    | "agent_busy"
    | "agent_not_initialized";
  message: string;
  cause?: unknown;
  recoverable: boolean;
  metadata?: Record<string, unknown>;
}

export interface TokenUsage {
  input: number;
  output: number;
  cache_read: number;
  cache_write: number;
  total_tokens: number;
}

export interface UseAgentResult {
  send(input: string | AgentInput, options?: AgentRunOptions): Promise<AgentRunResult>;
  stop(reason?: string): void;
  steer(input: string | AgentInput): Promise<void>;
  reset(): Promise<void>;
  status: AgentStatus;
  messages: AgentMessage[];
  toolCalls: AgentToolRun[];
  artifacts: AgentArtifactRef[];
  error: AgentError | null;
}

export type Unsubscribe = () => void;
