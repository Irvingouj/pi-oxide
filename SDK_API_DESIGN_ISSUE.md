# SDK API Design: Ideal Agent Interface

## Position

The public SDK should expose an agent, not a host protocol.

JavaScript developers should be able to create an agent, give it a model, tools,
and storage, then call `run()`. They should not need to know that Rust, WASM,
host handles, directives, transcript arrays, artifact maps, or persistence
snapshots exist.

The perfect API is:

- Simple for the common case.
- Typed for advanced extension points.
- Opaque at internal boundaries.
- Declarative at construction time.
- Event-driven during execution.
- Runtime-neutral in core concepts.
- Explicit about model, tools, storage, and lifecycle.

Anything below the `Agent` abstraction is implementation detail.

## The Ideal First Experience

```ts
import {
  Agent,
  anthropic,
  browserTools,
  indexedDbStore,
} from "@pi-oxide/web";

const agent = new Agent({
  sessionId: "default",
  model: anthropic({
    apiKey,
    model: "provider-model-id",
  }),
  tools: browserTools(),
  store: indexedDbStore(),
});

agent.on("text", (text) => {
  ui.appendAssistantText(text);
});

agent.on("toolUpdate", (tool) => {
  ui.showTool(tool.name, tool.status);
});

const result = await agent.run("Find the pricing page and summarize it.");
```

This is the bar. If a normal app needs to touch `HostAgent`, `PersistData`,
`runTurnWithHostAgent`, raw host events, artifact sync, or transcript internals,
the public API has failed.

## Primary API

```ts
export class Agent {
  constructor(config: AgentConfig);

  run(input: string | AgentInput, options?: AgentRunOptions): Promise<AgentRunResult>;
  stop(reason?: string): void;
  steer(input: string | AgentInput): Promise<void>;
  reset(): Promise<void>;
  dispose(): void;

  on<E extends AgentEventName>(
    event: E,
    handler: AgentEventHandler<E>,
  ): Unsubscribe;
}
```

### AgentConfig

```ts
export interface AgentConfig {
  sessionId: string;
  model: AgentModel;
  tools?: AgentTools;
  store?: AgentStore;
  instructions?: string;
  context?: AgentContextPolicy;
  artifacts?: ArtifactPolicy;
  telemetry?: AgentTelemetry;
}
```

Rules:

- `sessionId` identifies durable conversation state.
- `model` is required. No hidden global model.
- `tools` are optional. A text-only agent should be valid.
- `store` is optional. Without a store, the agent is memory-only.
- `instructions` are the app-level system instructions.
- `context` configures portable context policy, not provider formatting.
- `artifacts` configures artifact behavior without exposing host internals.
- `telemetry` is structured observability, not user-facing event handling.

The constructor should not perform I/O. The first `run()` lazily initializes and
loads stored state. Apps may call `agent.load()` only if an explicit load method
is later added for prewarming, but the normal path must not require it.

## Running The Agent

```ts
const result = await agent.run("What changed on this page?");
```

```ts
export type AgentInput =
  | string
  | {
      text: string;
      attachments?: AgentAttachment[];
      metadata?: Record<string, unknown>;
    };

export interface AgentRunOptions {
  signal?: AbortSignal;
  metadata?: Record<string, unknown>;
}
```

`run()` owns the whole turn:

- Load session snapshot if needed.
- Add user input.
- Build model context.
- Call the model.
- Stream semantic events.
- Execute tools.
- Persist state.
- Sync artifacts.
- Return a useful result.

No caller should wire the host turn loop manually.

## Run Result

```ts
export interface AgentRunResult {
  status: "completed" | "aborted" | "failed";
  message?: AgentMessage;
  text: string;
  toolCalls: AgentToolRun[];
  artifacts: AgentArtifactRef[];
  usage?: TokenUsage;
  error?: AgentError;
}
```

Rules:

- `text` is the final assistant text for the turn.
- `message` is the structured final assistant message.
- `toolCalls` contains user-meaningful tool activity.
- `artifacts` contains created or referenced artifacts.
- `usage` is normalized across providers when available.
- `error` is structured and actionable.

A caller should not need to reconstruct the final answer by listening to events.
Events are for live UI. `AgentRunResult` is for durable application logic.

## Events

The public event stream should be semantic. Raw host events are not the default
developer interface.

```ts
agent.on("messageStart", (message) => {});
agent.on("text", (delta) => {});
agent.on("messageEnd", (message) => {});
agent.on("toolStart", (tool) => {});
agent.on("toolUpdate", (tool) => {});
agent.on("toolEnd", (tool) => {});
agent.on("artifact", (artifact) => {});
agent.on("status", (status) => {});
agent.on("done", (result) => {});
agent.on("error", (error) => {});
```

```ts
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
  | "error";
```

Event payloads should be stable public types:

```ts
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
```

There may be an advanced debug channel:

```ts
agent.on("debug", (event) => {});
```

But debug events must be clearly marked unstable. They are not app contracts.

## Model API

Most developers should use provider factories:

```ts
const model = anthropic({
  apiKey,
  model: "provider-model-id",
});

const model = openai({
  apiKey,
  model: "provider-model-id",
});

const model = openaiCompatible({
  apiKey,
  baseUrl: "https://api.fireworks.ai/inference",
  model: "accounts/fireworks/models/...",
});
```

Provider factories hide:

- Provider message formatting.
- Tool schema formatting.
- Streaming protocol differences.
- Stop reason normalization.
- Usage normalization.
- Provider error parsing.

The public model interface is provider-neutral:

```ts
export interface AgentModel {
  generate(request: ModelRequest): Promise<ModelResponse> | AsyncIterable<ModelEvent>;
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
  raw?: unknown;
}
```

Advanced users can define their own model:

```ts
const model = defineModel({
  async generate(request) {
    const response = await myProvider.call(request);

    return {
      content: response.content,
      stopReason: response.stopReason,
      usage: response.usage,
      raw: response,
    };
  },
});
```

The model adapter is the only place where provider-specific message conversion
belongs. Application code should not convert `AgentMessage` into Anthropic,
OpenAI, or Fireworks wire formats.

## Tool API

Tools should be easy to declare and strongly typed at the boundary.

```ts
const tools = defineTools({
  getPageTitle: tool({
    description: "Read the current browser page title.",
    input: z.object({}),
    async run() {
      return {
        title: document.title,
      };
    },
  }),

  click: tool({
    description: "Click an element by selector.",
    input: z.object({
      selector: z.string(),
    }),
    async run({ selector }) {
      document.querySelector(selector)?.click();
      return { clicked: true };
    },
  }),
});
```

Built-in tool packs should be composable:

```ts
const agent = new Agent({
  sessionId: "browser",
  model,
  tools: [
    browserTools(),
    artifactTools(),
    customTools,
  ],
});
```

Tool rules:

- Tool input is parsed before `run()` is called.
- Invalid input becomes a structured tool error.
- Tool output is structured data, not only strings.
- The SDK handles conversion into model-specific tool result formats.
- Tools do not receive host handles.
- Tools do not manually read or write agent persistence.

## Store API

The store should be boring in the common case:

```ts
const store = indexedDbStore();
```

Other built-ins:

```ts
memoryStore();
localStorageStore();
indexedDbStore();
httpStore({ baseUrl: "/api/agent" });
```

Custom stores should receive opaque snapshots:

```ts
export interface AgentStore {
  loadSession(sessionId: string): Promise<AgentSnapshot | null>;
  saveSession(sessionId: string, snapshot: AgentSnapshot): Promise<void>;

  saveArtifact?(sessionId: string, artifact: AgentArtifact): Promise<void>;
  loadArtifact?(sessionId: string, artifactId: string): Promise<AgentArtifact | null>;
  searchArtifacts?(
    sessionId: string,
    query: ArtifactSearchQuery,
  ): Promise<ArtifactSearchResult[]>;
}
```

```ts
export interface AgentSnapshot {
  version: number;
  data: unknown;
}
```

Rules:

- `AgentSnapshot` is opaque to application code.
- The SDK owns snapshot schema migrations.
- The SDK validates snapshots before restoring them.
- Stores persist snapshots; they do not interpret transcript internals.
- Artifact support is optional.
- If artifact methods are absent, the SDK uses in-snapshot artifacts or memory
  artifacts according to `ArtifactPolicy`.

No public store interface should mention `PersistData`, `T`, `A`,
`host_artifacts`, context budgets, or Rust-internal state names.

## Artifact API

Artifacts are public objects, not leaked host internals.

```ts
export interface AgentArtifact {
  id: string;
  kind: "text" | "json" | "binary";
  content: string | Uint8Array | unknown;
  mimeType?: string;
  title?: string;
  metadata?: Record<string, unknown>;
  createdAt: number;
}

export interface AgentArtifactRef {
  id: string;
  kind: AgentArtifact["kind"];
  title?: string;
  mimeType?: string;
}
```

Artifact search is a store capability:

```ts
export interface ArtifactSearchQuery {
  text: string;
  limit?: number;
}

export interface ArtifactSearchResult {
  artifact: AgentArtifactRef;
  snippet?: string;
  score?: number;
  matchCount?: number;
}
```

Applications should not know whether an artifact currently lives in WASM memory,
IndexedDB, a server, or an object store.

## Context Policy

Context management should be configurable without exposing provider formatting
or raw Rust state.

```ts
export interface AgentContextPolicy {
  maxTokens?: number;
  toolResultLimit?: number;
  strategy?:
    | { type: "keep_full" }
    | { type: "head"; tokens: number }
    | { type: "tail"; tokens: number }
    | { type: "head_tail"; headTokens: number; tailTokens: number }
    | { type: "drop_if_old"; maxAgeMs: number };
  summarize?: boolean | AgentSummarizer;
}
```

Rules:

- Context policy is runtime-neutral.
- Provider-specific formatting remains inside model adapters.
- The core decides what context to project.
- Hosts may store artifacts, but the projection decision is portable and
  testable.

## React API

React should wrap the same `Agent` API. It should not duplicate host lifecycle
logic.

```ts
const agent = useAgent({
  sessionId: "default",
  model: anthropic({ apiKey, model }),
  tools: browserTools(),
  store: indexedDbStore(),
});

await agent.send("Summarize this page.");
```

```ts
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
```

The hook should expose app-ready state. It should not expose `HostAgent`,
`PersistData`, raw `AgentEvent`, host handles, or directive processing.

## Error API

Errors should be structured:

```ts
export interface AgentError {
  code:
    | "model_auth_failed"
    | "model_rate_limited"
    | "model_unavailable"
    | "tool_input_invalid"
    | "tool_failed"
    | "store_load_failed"
    | "store_save_failed"
    | "snapshot_invalid"
    | "aborted"
    | "internal_error";
  message: string;
  cause?: unknown;
  recoverable: boolean;
  metadata?: Record<string, unknown>;
}
```

String-only errors are not good enough once data has crossed an SDK boundary.

## What Must Disappear From The Public Surface

The ideal public SDK does not expose these as normal application concepts:

- `HostAgent`
- WASM handles
- `createHostAgentInstance`
- `runTurnWithHostAgent`
- `PersistData`
- `T`
- `A`
- `host_artifacts`
- raw host directives
- raw Rust event names
- manual artifact synchronization
- manual `onPersist`
- provider wire-format message conversion

These may exist internally or under an explicitly unstable advanced namespace,
but they are not the product API.

## Distance From Current Code

The current implementation already has most of the engine pieces:

- `web/src/services/agentService.ts` owns the host lifecycle and turn loop.
- `web/src/services/llmService.ts` adapts a provider into the current LLM shape.
- `web/src/services/toolService.ts` builds browser and artifact tool registries.
- `web/src/browser/persistence.ts` persists session state in IndexedDB.
- `web/src/hooks/useAgent.ts` proves the app flow works.

The missing layer is the SDK facade.

Current code is roughly 60-70% complete as an engine, but only 30-40% complete
as a pleasant public SDK. The hard protocol exists. The ideal developer
experience does not.

To reach the ideal API, build a new public layer that owns:

- `Agent`
- provider factories such as `anthropic()` and `openaiCompatible()`
- store factories such as `indexedDbStore()`
- `defineTools()` and `tool()`
- semantic event mapping
- opaque snapshot persistence
- structured run results
- React state derived from `Agent`, not from host internals

The goal is not to make the current low-level API nicer. The goal is to make it
irrelevant for normal users.
