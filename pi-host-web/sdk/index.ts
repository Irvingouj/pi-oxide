// Public SDK exports — the only surface normal apps should import.
// Internal engine details are hidden in sdk/internal/.

export { ensureInit, HostError, unwrap, toolResult } from "./init.ts";

// SDK exports
export { Agent } from "./agent.ts";
export { defineModel } from "./model.ts";
export { anthropic } from "./internal/providers/anthropic.ts";
export { openai, openaiCompatible } from "./internal/providers/openai.ts";
export { defineTools, tool } from "./tools.ts";
export { browserTools } from "./internal/tools/browser.ts";
export { artifactTools } from "./internal/tools/artifact.ts";
export { indexedDbStore, memoryStore, localStorageStore, httpStore } from "./stores.ts";
export { useAgent } from "./react/index.ts";

export type {
	AgentArtifact,
	AgentArtifactRef,
	ArtifactPolicy,
	ArtifactSearchQuery,
	ArtifactSearchResult,
} from "./artifacts.ts";

export type {
	AgentConfig,
	AgentInput,
	AgentRunOptions,
	AgentRunResult,
	AgentEventName,
	AgentEventHandler,
	AgentMessage,
	AgentContentBlock,
	AgentToolRun,
	AgentStatus,
	AgentModel,
	ModelRequest,
	ModelResponse,
	ModelEvent,
	AgentTools,
	AgentToolDefinition,
	AgentStore,
	AgentSnapshot,
	AgentContextPolicy,
	AgentSummarizer,
	AgentTelemetry,
	AgentError,
	Unsubscribe,
	TokenUsage,
	UseAgentResult,
} from "./types.ts";

export { createAgentError } from "./errors.ts";
