/**
 * Agent service — high-level agent lifecycle with the WASM SDK.
 *
 * Pure JS, no React. Encapsulates Agent.create, run, steer, stop, and session persistence.
 */

import {
	type AgentEvent,
	type AgentMessage,
	type CancelReason,
	createHostAgent,
	destroyHostAgent,
	ensureInit,
	getHostAgentPersistData,
	hostAbort,
	hostAcceptCompaction,
	hostContinueTurn,
	hostFeedLlmChunk,
	hostLlmDone,
	hostReadArtifact,
	hostSteer,
	hostToolCancelled,
	hostToolDone,
	HostError,
	type LlmChunk,
	type LlmResult,
	type PersistData,
	restoreHostAgent,
	startTurn,
	type ToolCallId,
	type ToolDefinition,
	type ToolResult,
	type TurnResultOutput,
	unwrap,
} from "@pi-oxide/pi-host-web";

export interface ArtifactSearchResult {
	id: string;
	snippet: string;
	match_count: number;
}

export interface ArtifactStore {
	save(sessionId: string, artifactId: string, content: string): Promise<void>;
	load(sessionId: string, artifactId: string): Promise<string | null>;
	search(sessionId: string, query: string): Promise<ArtifactSearchResult[]>;
}

export interface AgentRunConfig {
	llm: {
		call(
			context: LlmContext,
			signal?: AbortSignal,
		): Promise<LlmStream> | LlmStream;
		summarize?(messages: AgentMessage[], signal?: AbortSignal): Promise<string>;
	};
	tools: Record<string, (call: ToolCall) => Promise<ToolResult> | ToolResult>;
	llmTools?: ToolDefinition[];
	onEvent?: (event: AgentEvent) => void;
	signal?: AbortSignal;
	onPersist?: (data: PersistData) => Promise<void>;
	artifactStore?: ArtifactStore;
}

export interface LlmStream {
	chunks: AsyncIterable<LlmChunk>;
	result: Promise<LlmResult>;
}

export interface LlmContext {
	system_prompt: string;
	messages: AgentMessage[];
	tools: ToolDefinition[];
}

export interface ToolCall {
	id: string;
	name: string;
	arguments: Record<string, unknown>;
}

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

export interface TurnResult {
	aborted: boolean;
}

export function stopAgent(abortController: AbortController | null): void {
	abortController?.abort("user-requested");
}

// Local extension for markers until SDK rebuilds with wu-2
interface StepWithMarkers extends TurnResultOutput {
	markers?: Array<{ type: string; entry_ids: string[] }>;
}

export const stepProcessor = {
	async processStep(
		step: TurnResultOutput,
		hostAgent: HostAgent,
		config: AgentRunConfig,
	): Promise<TurnResultOutput> {
		const s = step as StepWithMarkers;
		for (const event of s.events) {
			config.onEvent?.(event);
		}

		if (config.artifactStore) {
			const sessionId = hostAgent.getSessionId();
			if (sessionId) {
				const synced = new Set<string>();
				for (const marker of s.markers ?? []) {
					if (marker.type === "new_artifacts") {
						for (const id of marker.entry_ids) {
							if (synced.has(id)) continue;
							synced.add(id);
							const content = hostReadArtifact(hostAgent.handle, id);
							if (content) {
								await config.artifactStore.save(sessionId, id, content);
							}
						}
					}
				}
			}
		}

		return step;
	},
};

// ---------------------------------------------------------------------------
// HostAgent — thin wrapper around the new WASM HostAgent handle
// ---------------------------------------------------------------------------

export class HostAgent {
	readonly handle: number;
	readonly sessionId: string | undefined;

	constructor(handle: number, sessionId?: string) {
		this.handle = handle;
		this.sessionId = sessionId;
	}

	getSessionId(): string | undefined {
		return this.sessionId;
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

	steer(message: AgentMessage): TurnResultOutput {
		return unwrap(hostSteer(this.handle, message));
	}

	getPersistData(): PersistData {
		const result = unwrap<{ state: PersistData }>(
			getHostAgentPersistData(this.handle),
		);
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
	await ensureInit();
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
		const result = unwrap<{ handle: number }>(
			restoreHostAgent(options, persistData),
		);
		return new HostAgent(result.handle, sessionId);
	}
	const result = unwrap<{ handle: number }>(createHostAgent(options, budget));
	return new HostAgent(result.handle, sessionId);
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
		let step: TurnResultOutput = await stepProcessor.processStep(
			hostAgent.startTurn(prompt, config.llmTools ?? []),
			hostAgent,
			config,
		);

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
						const stream = await config.llm.call(
							directive.context,
							config.signal,
						);
						for await (const chunk of stream.chunks) {
							checkAbort();
							const ev = hostAgent.feedLlmChunk(chunk);
							for (const e of ev.events) config.onEvent?.(e);
						}
						checkAbort();
						const result = await stream.result;
						step = await stepProcessor.processStep(
							hostAgent.llmDone(result),
							hostAgent,
							config,
						);
						break;
					}

					case "execute_tools": {
						for (const call of directive.calls) {
							stepChanged = true;
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
							step = await stepProcessor.processStep(
								hostAgent.toolDone(call.id, result),
								hostAgent,
								config,
							);
						}
						break;
					}

					case "persist": {
						const data = hostAgent.getPersistData();
						await config.onPersist?.(data);
						break;
					}

					case "summarize": {
						stepChanged = true;
						const context = directive.context;
						const summary =
							(await config.llm.summarize?.(
								context.messages,
								config.signal,
							)) ?? "Compacted by host";
						step = await stepProcessor.processStep(
							hostAgent.acceptCompaction(summary, []),
							hostAgent,
							config,
						);
						break;
					}

					case "finished": {
						shouldReturn = true;
						break;
					}

					case "wait_for_input": {
						stepChanged = true;
						step = await stepProcessor.processStep(
							hostAgent.continueTurn(),
							hostAgent,
							config,
						);
						break;
					}

					case "cancel_tools": {
						stepChanged = true;
						for (const id of directive.tool_call_ids) {
							step = await stepProcessor.processStep(
								hostAgent.toolCancelled(id, directive.reason),
								hostAgent,
								config,
							);
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

		return { aborted: false };
	} catch (e: unknown) {
		const isUserAbort =
			(e instanceof HostError && e.code === "user_aborted") ||
			(e instanceof DOMException && e.name === "AbortError");
		if (isUserAbort) {
			return { aborted: true };
		}
		throw e;
	}
}
