/**
 * Agent service — high-level agent lifecycle with the WASM SDK.
 *
 * Pure JS, no React. Encapsulates Agent.create, run, steer, stop, and session persistence.
 */

import {
	Agent,
	type AgentEvent,
	type AgentMessage,
	type AgentRunConfig,
	type SessionState,
} from "@pi-oxide/pi-host-web";

const SYSTEM_PROMPT =
	"You are a browser automation agent. You can see the current page, " +
	"query elements, click, type, evaluate JavaScript, and read console logs. " +
	"Help the user accomplish tasks in the browser.";

export interface AgentModelConfig {
	id: string;
	name: string;
	api: string;
	provider: string;
	reasoning: boolean;
	context_window: number;
	max_tokens: number;
	capabilities: {
		vision: boolean;
		json_mode: boolean;
		function_calling: boolean;
		streaming: boolean;
	};
	cost: {
		input: number;
		output: number;
		cache_read: number;
		cache_write: number;
	};
}

export const DEFAULT_MODEL: AgentModelConfig = {
	id: "browser-model",
	name: "browser",
	api: "anthropic",
	provider: "anthropic",
	reasoning: false,
	context_window: 100000,
	max_tokens: 1024,
	capabilities: {
		vision: false,
		json_mode: true,
		function_calling: true,
		streaming: true,
	},
	cost: { input: 0, output: 0, cache_read: 0, cache_write: 0 },
};

export async function createAgent(
	sessionId: string,
	sessionState?: SessionState,
): Promise<Agent> {
	return Agent.create({
		system_prompt: SYSTEM_PROMPT,
		model: DEFAULT_MODEL,
		session_id: sessionId,
		session_state: sessionState,
	});
}

export interface TurnResult {
	aborted: boolean;
	error: string | null;
}

export async function runTurn(
	agent: Agent,
	text: string,
	config: AgentRunConfig,
): Promise<TurnResult> {
	try {
		await agent.run(text, config);
		return { aborted: false, error: null };
	} catch (e: unknown) {
		const isUserAbort =
			(typeof e === "object" &&
				e !== null &&
				"code" in e &&
				(e as Record<string, unknown>).code === "user_aborted") ||
			(e instanceof DOMException && e.name === "AbortError");
		if (isUserAbort) {
			return { aborted: true, error: null };
		}
		const msg = e instanceof Error ? e.message : String(e);
		return { aborted: false, error: msg };
	}
}

export function stopAgent(abortController: AbortController | null): void {
	abortController?.abort("user-requested");
}

export function steerAgent(agent: Agent, text: string): AgentEvent[] {
	const message: AgentMessage = {
		role: "user",
		content: [{ type: "text", text }],
		timestamp: Date.now(),
	};
	return agent.steer(message);
}

export function getSessionState(agent: Agent): SessionState {
	return agent.getSessionState();
}
