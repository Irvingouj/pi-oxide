import {
	createHostAgent,
	destroyHostAgent,
	getHostAgentPersistData,
	hostSteer,
	type PersistData,
	type AgentEvent as RawAgentEvent,
	restoreHostAgent,
	type AgentMessage as WasmAgentMessage,
} from "../../pi_host_web.js";
import type { AgentConfig } from "../types.ts";
import { getLogger } from "../internal/logger.ts";
import { buildContextBudget, buildModelOptions } from "../orchestration/config-builders.ts";
import { ensureInit, unwrap } from "./init.ts";

export class HostAgent {
	/** @internal */
	readonly handle: number;
	readonly sessionId: string | undefined;

	constructor(handle: number, sessionId?: string) {
		this.handle = handle;
		this.sessionId = sessionId;
	}

	getSessionId(): string | undefined {
		return this.sessionId;
	}

	steer(message: WasmAgentMessage): { events: RawAgentEvent[] } {
		const result = unwrap(hostSteer(this.handle, message)) as { events: RawAgentEvent[] };
		return { events: result.events };
	}

	getPersistData(): PersistData {
		const result = unwrap(getHostAgentPersistData(this.handle)) as { state: PersistData };
		return result.state;
	}

	destroy() {
		destroyHostAgent(this.handle);
	}
}

export async function createHostAgentInstance(config: AgentConfig, sessionState?: PersistData): Promise<HostAgent> {
	await ensureInit();
	const logger = config.logger ?? getLogger("engine");
	const options = {
		system_prompt: config.instructions ?? "You are a helpful assistant.",
		model: buildModelOptions(config.model),
		session_id: config.sessionId,
	};

	let handle: number;
	if (sessionState) {
		logger.info("Restoring host agent from session state", { sessionId: config.sessionId });
		const restored = unwrap(restoreHostAgent(options, sessionState)) as { handle: number };
		handle = restored.handle;
	} else {
		logger.info("Creating new host agent", { sessionId: config.sessionId });
		const result = unwrap(createHostAgent(options, buildContextBudget(config.context))) as { handle: number };
		handle = result.handle;
	}
	return new HostAgent(handle, config.sessionId);
}
