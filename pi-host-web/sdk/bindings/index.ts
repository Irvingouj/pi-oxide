// Layer 2: Bindings over raw WASM state machine exports.

export { createHostAgentInstance, HostAgent } from "./host-agent.ts";
export {
	ensureInit,
	HostError,
	setLogLevel,
	toolResult,
	unwrap,
} from "./init.ts";
export { processStepMarkers } from "./markers.ts";
export {
	buildToolCallPreparations,
	toolErrorFromUnknown,
} from "./tool-preparation.ts";
export { runTurnWithHostAgent } from "./turn-loop.ts";
export type {
	AgentRunConfig,
	ArtifactStore,
	LlmStream,
	TurnResult,
} from "./types.ts";
