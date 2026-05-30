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
	type CancelReason,
	type LlmChunk,
	type LlmResult,
	type PersistData,
	type SessionState,
	type ToolCallId,
	type ToolDefinition,
	type ToolResult,
	type TurnResultOutput,
	hostAcceptCompaction,
	hostContinueTurn,
	hostFeedLlmChunk,
	hostLlmDone,
	hostToolDone,
	hostToolCancelled,
	hostAbort,
	startTurn,
	getHostAgentPersistData,
	destroyHostAgent,
	createHostAgent,
	restoreHostAgent,
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

// ---------------------------------------------------------------------------
// HostAgent — thin wrapper around the new WASM HostAgent handle
// ---------------------------------------------------------------------------

class HostError extends Error {
	code: string;
	constructor(code: string, message: string) {
		super(message);
		this.code = code;
		this.name = "HostError";
	}
}

function unwrap<T>(result: { ok: boolean; data?: T; error?: { code: string; message: string } }): T {
	if (!result.ok) {
		throw new HostError(result.error!.code, result.error!.message);
	}
	return result.data!;
}

export class HostAgent {
	readonly handle: number;
	constructor(handle: number) {
		this.handle = handle;
	}

	startTurn(prompt: AgentMessage, tools?: ToolDefinition[]): TurnResultOutput {
		return unwrap(startTurn(this.handle, { prompt, tools: tools ?? [] }));
	}

	feedLlmChunk(chunk: LlmChunk): TurnResultOutput {
		return unwrap(hostFeedLlmChunk(this.handle, chunk));
	}

	llmDone(result: LlmResult): TurnResultOutput {
		return unwrap(hostLlmDone(this.handle, result));
	}

	toolDone(id: string, result: ToolResult): TurnResultOutput {
		return unwrap(hostToolDone(this.handle, id as ToolCallId, result));
	}

	toolCancelled(id: string, reason: CancelReason): TurnResultOutput {
		return unwrap(hostToolCancelled(this.handle, id, reason));
	}

	acceptCompaction(summary: string, ids: string[]): TurnResultOutput {
		return unwrap(hostAcceptCompaction(this.handle, summary, ids));
	}

	continueTurn(): TurnResultOutput {
		return unwrap(hostContinueTurn(this.handle));
	}

	abort(): TurnResultOutput {
		return unwrap(hostAbort(this.handle));
	}

	getPersistData(): PersistData {
		const result = unwrap<{ state: PersistData }>(getHostAgentPersistData(this.handle));
		return result.state;
	}

	destroy() {
		destroyHostAgent(this.handle);
	}
}

export async function createHostAgentInstance(
	sessionId: string,
	persistData?: PersistData,
): Promise<HostAgent> {
	const options = {
		system_prompt: SYSTEM_PROMPT,
		model: DEFAULT_MODEL,
		session_id: sessionId,
	};
	const budget = {
		max_tool_result_chars: 50000,
		max_context_tokens: 100000,
		microcompact_after_turns: 5,
		compaction_threshold: 0.75,
	};
	if (persistData) {
		const result = unwrap<{ handle: number }>(restoreHostAgent(options, persistData));
		return new HostAgent(result.handle);
	}
	const result = unwrap<{ handle: number }>(createHostAgent(options, budget));
	return new HostAgent(result.handle);
}

export async function runTurnWithHostAgent(
	hostAgent: HostAgent,
	text: string,
	config: AgentRunConfig,
): Promise<TurnResult> {
	const checkAbort = () => {
		if (config.signal?.aborted) {
			try {
				hostAgent.abort();
			} catch {
				/* agent may already be idle; ignore */
			}
			throw new HostError("user_aborted", "Turn stopped by user");
		}
	};

	const prompt: AgentMessage = {
		role: "user",
		content: [{ type: "text", text }],
		timestamp: Date.now(),
	};

	try {
		let step = hostAgent.startTurn(prompt, config.llmTools ?? []);
		for (const event of step.events) {
			config.onEvent?.(event);
		}

		const pendingCompacts: Array<{ compact_up_to: string }> = [];

		while (true) {
			checkAbort();
			const directives = step.directives ?? [];

			let stepChanged = false;
			let shouldReturn = false;

			for (const directive of directives) {
				checkAbort();
				switch (directive.type) {
					case "stream_llm": {
						stepChanged = true;
						const stream = await config.llm.call(directive.context, config.signal);
						for await (const chunk of stream.chunks) {
							checkAbort();
							const ev = hostAgent.feedLlmChunk(chunk);
							for (const e of ev.events) config.onEvent?.(e);
						}
						checkAbort();
						const result = await stream.result;
						step = hostAgent.llmDone(result);
						for (const e of step.events) config.onEvent?.(e);
						break;
					}

					case "execute_tools": {
						stepChanged = true;
						for (const call of directive.calls) {
							checkAbort();
							const handler = config.tools[call.name];
							let result: ToolResult;
							if (handler) {
								result = await handler(call);
							} else {
								result = {
									content: [
										{ type: "text", text: `No handler for ${call.name}` },
									],
									};
							}
							step = hostAgent.toolDone(call.id, result);
							for (const e of step.events) config.onEvent?.(e);
						}
						break;
					}

					case "persist": {
						const data = hostAgent.getPersistData();
						await config.onPersist?.(data);
						break;
					}

					case "compact": {
						// Defer compaction until after the turn completes so it
						// does not overwrite step and lose directives like ExecuteTools.
						pendingCompacts.push(directive);
						break;
					}

					case "finished": {
						shouldReturn = true;
						break;
					}

					case "wait_for_input": {
						stepChanged = true;
						step = hostAgent.continueTurn();
						for (const e of step.events) config.onEvent?.(e);
						break;
					}

					case "cancel_tools": {
						stepChanged = true;
						for (const id of directive.tool_call_ids) {
							step = hostAgent.toolCancelled(id, directive.reason);
							for (const e of step.events) config.onEvent?.(e);
						}
						break;
					}
				}
			}

			if (shouldReturn) {
				break;
			}

			if (!stepChanged) {
				break;
			}
		}

		// Process any deferred compaction directives after the turn completes.
		for (const directive of pendingCompacts) {
			const data = hostAgent.getPersistData();
			const messagesToSummarize: AgentMessage[] = [];
			for (const entry of data.entries) {
				const kind = entry.kind as Record<string, unknown>;
				if (kind.type === "message") {
					messagesToSummarize.push(kind as unknown as AgentMessage);
				} else if (kind.type === "compaction") {
					messagesToSummarize.push({
						role: "user",
						content: [
							{
								type: "text",
								text: `Previous conversation summary: ${kind.summary}`,
							},
							],
						timestamp: entry.timestamp,
					});
				}
			}
			const summary =
				(await config.llm.summarize?.(messagesToSummarize, config.signal)) ??
				"Compacted by host";
			const result = hostAgent.acceptCompaction(summary, [
				directive.compact_up_to,
			]);
			for (const e of result.events) config.onEvent?.(e);
			for (const d of result.directives) {
				if (d.type === "persist") {
					const persistData = hostAgent.getPersistData();
					await config.onPersist?.(persistData);
				}
			}
		}

		return { aborted: false, error: null };
	} catch (e: unknown) {
		const isUserAbort =
			(e instanceof HostError && e.code === "user_aborted") ||
			(e instanceof DOMException && e.name === "AbortError");
		if (isUserAbort) {
			try {
				hostAgent.abort();
			} catch {
				/* agent may already be idle; ignore */
				}
			return { aborted: true, error: null };
		}
		throw e;
	}
}
