/** @deprecated Import from `../orchestration/agent-engine.ts` instead. */

export type {
	AgentRunConfig,
	LlmStream,
	TurnResult,
} from "../bindings/types.ts";
export {
	createEngineAgent,
	createHostAgentInstance,
	destroyEngineAgent,
	HostAgent,
	resetAgentState,
	runAgentTurn,
	runTurnWithHostAgent,
	steerAgent,
} from "../orchestration/agent-engine.ts";
