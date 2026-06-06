// Layer 2: Bindings over raw WASM state machine exports.

export {
	ensureInit,
	HostError,
	setLogLevel,
	toolResult,
	unwrap,
} from "./init.ts";
export { HostAgent, createHostAgentInstance } from "./host-agent.ts";
export { runTurnWithHostAgent } from "./turn-loop.ts";
export { buildToolCallPreparations, toolErrorFromUnknown } from "./tool-preparation.ts";
export { processStepMarkers } from "./markers.ts";
export type { AgentRunConfig, ArtifactStore, LlmStream, TurnResult } from "./types.ts";
