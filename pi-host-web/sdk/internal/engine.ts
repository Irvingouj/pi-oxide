// Engine orchestration — the only layer that knows about HostAgent, PersistData, and directives.
// Wires all AgentConfig fields (instructions, context, artifacts, model).
// Uses the raw WASM API directly (createHostAgent, startTurn, hostFeedLlmChunk, etc.).

import {
	type AgentEvent as RawAgentEvent,
	type AgentMessage as WasmAgentMessage,
	type Content,
	type LlmChunk,
	type LlmResult,
	type PersistData,
	type LlmContext,
	type ToolCall,
	type ToolError,
	type ToolResult,
	type ToolDefinition,
	type CancelReason,
	createHostAgent,
	destroyHostAgent,
	startTurn,
	hostFeedLlmChunk,
	hostLlmDone,
	hostToolDone,
	hostToolFailed,
	hostToolCancelled,
	hostContinueTurn,
	hostAbort,
	hostAcceptCompaction,
	hostSteer,
	hostReset,
	hostReadArtifact,
	getHostAgentPersistData,
	restoreHostAgent,
} from "../../pi_host_web.js";
import { ensureInit, HostError } from "../init.ts";
import { EventMapper } from "./events.ts";
import { SnapshotSerializer } from "../snapshot.ts";
import { ToolRegistryBuilder } from "./tools/registry.ts";
import { getLogger } from "./logger.ts";
import type {
	AgentConfig,
	AgentInput,
	AgentRunOptions,
	AgentRunResult,
	AgentStatus,
	AgentMessage,
	ModelResponse,
	ModelEvent,
	TokenUsage,
	Logger,
} from "../types.ts";
import { createAgentError } from "../errors.ts";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface ArtifactStore {
	save(sessionId: string, artifactId: string, content: string): Promise<void>;
	load(sessionId: string, artifactId: string): Promise<string | null>;
	search(sessionId: string, query: string): Promise<Array<{ id: string; snippet: string; match_count: number }>>;
}

export interface AgentRunConfig {
	llm: {
		call(context: LlmContext, signal?: AbortSignal): Promise<LlmStream> | LlmStream;
		summarize?(messages: WasmAgentMessage[], signal?: AbortSignal): Promise<string>;
	};
	tools: Record<string, (call: ToolCall) => Promise<ToolResult> | ToolResult>;
	llmTools?: ToolDefinition[];
	onEvent?: (event: RawAgentEvent) => void;
	onMarkers?: (markers: Array<{ type: string; entry_ids?: string[] }>) => void;
	signal?: AbortSignal;
	onPersist?: (data: PersistData) => Promise<void>;
	artifactStore?: ArtifactStore;
	logger?: Logger;
}

export interface LlmStream {
	chunks: AsyncIterable<LlmChunk>;
	result: Promise<LlmResult>;
}

export interface TurnResult {
	aborted: boolean;
}

// ---------------------------------------------------------------------------
// HostAgent — thin wrapper around the WASM handle
// ---------------------------------------------------------------------------

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
		const result = unwrap(hostSteer(this.handle, message));
		return { events: result.events };
	}

	getPersistData(): PersistData {
		const result = unwrap(getHostAgentPersistData(this.handle));
		return result.state;
	}

	destroy() {
		destroyHostAgent(this.handle);
	}
}

// ---------------------------------------------------------------------------
// Host agent lifecycle
// ---------------------------------------------------------------------------

export async function createHostAgentInstance(
	config: AgentConfig,
	sessionState?: PersistData,
): Promise<HostAgent> {
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
		const restored = unwrap(restoreHostAgent(options, sessionState));
		handle = restored.handle;
	} else {
		logger.info("Creating new host agent", { sessionId: config.sessionId });
		const result = unwrap(createHostAgent(options, buildContextBudget(config.context)));
		handle = result.handle;
	}
	return new HostAgent(handle, config.sessionId);
}

export async function runTurnWithHostAgent(
	hostAgent: HostAgent,
	message: WasmAgentMessage,
	config: AgentRunConfig,
): Promise<TurnResult> {
	const signal = config.signal;
	const logger = config.logger ?? getLogger("engine");

	const checkAbort = () => {
		if (signal?.aborted) {
			try {
				unwrap(hostAbort(hostAgent.handle));
			} catch (e) {
				// ignore wrong_phase errors
			}
			throw new HostError("user_aborted", "Turn stopped by user");
		}
	};

	try {
		logger.info("Starting turn", { sessionId: hostAgent.sessionId });
		let step = unwrap(startTurn(hostAgent.handle, { prompt: message, tools: config.llmTools }));
		for (const event of step.events) {
			config.onEvent?.(event);
		}
		await processStepMarkers(step, hostAgent, config);

		while (true) {
			checkAbort();
			const actions = step.directives ?? [];
			if (actions.length === 0) {
				logger.info("Turn completed with no directives");
				return { aborted: false };
			}

			logger.debug("Processing directives", { count: actions.length, types: actions.map((a) => a.type) });

			let stateAdvanced = false;
			let turnFinished = false;

			for (const action of actions) {
				checkAbort();
				switch (action.type) {
					case "stream_llm": {
						logger.info("Streaming LLM", { messageCount: action.context.messages.length, toolCount: action.context.tools.length });
						const stream = await config.llm.call(action.context, signal);
						for await (const chunk of stream.chunks) {
							checkAbort();
							const ev = unwrap(hostFeedLlmChunk(hostAgent.handle, chunk));
							for (const e of ev.events) config.onEvent?.(e);
						}
						checkAbort();
						const result = await stream.result;
						step = unwrap(hostLlmDone(hostAgent.handle, result));
						for (const e of step.events) config.onEvent?.(e);
						await processStepMarkers(step, hostAgent, config);
						stateAdvanced = true;
						break;
					}

					case "execute_tools": {
						logger.info("Executing tools", { count: action.calls.length, names: action.calls.map((c) => c.name) });
						for (const call of action.calls) {
							checkAbort();
							const handler = config.tools[call.name];
							if (!handler) {
								logger.warn("Tool handler not found", { toolName: call.name });
								step = unwrap(hostToolFailed(hostAgent.handle, call.id, {
									code: "tool_not_found",
									message: `No handler for ${call.name}`,
								}));
							} else {
								try {
									const result = await handler(call);
									logger.debug("Tool completed", { toolName: call.name });
									step = unwrap(hostToolDone(hostAgent.handle, call.id, result));
								} catch (e) {
									logger.warn("Tool failed", { toolName: call.name, error: e instanceof Error ? e.message : String(e) });
									step = unwrap(hostToolFailed(hostAgent.handle, call.id, toolErrorFromUnknown(e)));
								}
							}
							for (const e of step.events) config.onEvent?.(e);
							await processStepMarkers(step, hostAgent, config);
						}
						if ((step.directives ?? []).length === 0) {
							step = unwrap(hostContinueTurn(hostAgent.handle));
							for (const e of step.events) config.onEvent?.(e);
							await processStepMarkers(step, hostAgent, config);
						}
						stateAdvanced = true;
						break;
					}

					case "cancel_tools": {
						logger.info("Cancelling tools", { count: action.tool_call_ids.length, reason: action.reason });
						for (const id of action.tool_call_ids) {
							step = unwrap(hostToolCancelled(hostAgent.handle, id, action.reason));
							for (const e of step.events) config.onEvent?.(e);
							await processStepMarkers(step, hostAgent, config);
						}
						stateAdvanced = true;
						break;
					}

					case "summarize": {
						logger.info("Summarizing context");
						const summary = await config.llm.summarize!(action.context.messages, signal);
						step = unwrap(hostAcceptCompaction(hostAgent.handle, summary, []));
						for (const e of step.events) config.onEvent?.(e);
						await processStepMarkers(step, hostAgent, config);
						stateAdvanced = true;
						break;
					}

					case "persist": {
						logger.info("Persisting state");
						const persistData = hostAgent.getPersistData();
						await config.onPersist?.(persistData);
						break;
					}

					case "finished":
						turnFinished = true;
						break;

					case "wait_for_input":
						step = unwrap(hostContinueTurn(hostAgent.handle));
						for (const e of step.events) config.onEvent?.(e);
						await processStepMarkers(step, hostAgent, config);
						stateAdvanced = true;
						break;

					default:
						logger.warn("Unknown directive type", { type: (action as { type: string }).type });
						break;
				}
			}

			// If the turn is finished, return after processing all directives in this batch
			if (turnFinished) {
				logger.info("Turn finished");
				return { aborted: false };
			}

			// If no state was advanced and there are no new directives, continue the turn
			if (!stateAdvanced && (step.directives ?? []).length === 0) {
				logger.debug("No state advanced, continuing turn");
				step = unwrap(hostContinueTurn(hostAgent.handle));
				for (const e of step.events) config.onEvent?.(e);
				await processStepMarkers(step, hostAgent, config);
			}
		}
	} catch (e: unknown) {
		const isUserAbort =
			(e instanceof HostError && e.code === "user_aborted") ||
			(e instanceof DOMException && e.name === "AbortError");
		if (isUserAbort) {
			logger.info("Turn aborted by user");
			return { aborted: true };
		}
		logger.error("Turn failed", { error: e instanceof Error ? e.message : String(e) });
		throw e;
	}
}

// ---------------------------------------------------------------------------
// SDK engine facade
// ---------------------------------------------------------------------------

export async function createEngineAgent(
	config: AgentConfig,
	callbacks: {
		onEvent: (event: { type: string; payload: unknown }) => void;
		onStatus: (status: AgentStatus) => void;
	},
): Promise<HostAgent> {
	const logger = config.logger ?? getLogger("engine");

	// Load snapshot from store if available
	let sessionState: PersistData | undefined;
	if (config.store) {
		const snapshot = await config.store.loadSession(config.sessionId);
		if (snapshot) {
			const serializer = new SnapshotSerializer();
			sessionState = serializer.deserialize(snapshot) as PersistData | undefined;
			if (sessionState) {
				logger.info("Session snapshot loaded", { sessionId: config.sessionId });
			} else {
				logger.warn("Session snapshot version mismatch, starting fresh", { sessionId: config.sessionId });
			}
		}
	}

	const hostAgent = await createHostAgentInstance(config, sessionState);
	return hostAgent;
}

export function destroyEngineAgent(hostAgent: HostAgent): void {
	hostAgent.destroy();
}

export async function runAgentTurn(
	hostAgent: HostAgent,
	config: AgentConfig,
	input: string | AgentInput,
	options: AgentRunOptions | undefined,
	signal: AbortSignal,
	callbacks: {
		onEvent: (event: { type: string; payload: unknown }) => void;
		onStatus: (status: AgentStatus) => void;
	},
): Promise<AgentRunResult> {
	const logger = config.logger ?? getLogger("engine");

	// Build user message with attachments if present
	const userMessage = buildUserMessage(input);

	const eventMapper = new EventMapper();
	const runState = eventMapper.createRunState();

	// Build tool registry — supports AgentTools | AgentTools[]
	const toolRegistry = new ToolRegistryBuilder();
	const allTools = normalizeTools(config.tools);
	const artifactStore = buildArtifactStore(config);
	const toolMap = toolRegistry.build(allTools, artifactStore, config.sessionId);
	const llmTools = toolRegistry.getLlmTools(allTools);

	logger.info("Running agent turn", {
		sessionId: config.sessionId,
		toolCount: llmTools.length,
		messageLength: typeof input === "string" ? input.length : input.text.length,
	});

	// Build LLM provider adapter that returns LlmStream with chunks
	const llmProvider: AgentRunConfig["llm"] = {
		call: async (context: LlmContext, s?: AbortSignal): Promise<LlmStream> => {
			const effectiveSignal = s || signal;
			callbacks.onStatus({ state: "calling_model", message: "Calling model..." });
			logger.info("Calling model", { model: config.model.id ?? "custom" });

			const modelRequest = {
				instructions: context.system_prompt,
				messages: convertWasmMessagesToAgentMessages(context.messages),
				tools: llmTools.map((t) => ({
					name: t.name,
					description: t.description,
					inputSchema: t.parameters,
					run: () => null, // Dummy run — LLM tools only need schema, not handler
				})),
				signal: effectiveSignal,
				metadata: mergeMetadata(input, options?.metadata),
			};

			// Prefer real streaming when the model supports it
			if (config.model.generateStream) {
				logger.debug("Using streaming model");
				return modelStreamToLlmStream(
					config.model.generateStream(modelRequest, effectiveSignal),
					effectiveSignal,
					runState,
				);
			}

			logger.debug("Using non-streaming model");
			const response = await config.model.generate(modelRequest);

			if (response.usage) {
				runState.usage = response.usage;
				logger.info("Model usage", { usage: response.usage });
			}

			return modelResponseToLlmStream(response, effectiveSignal);
		},
		summarize: config.model.summarize
			? async (wasmMessages: WasmAgentMessage[], sig?: AbortSignal) => {
					logger.info("Calling model summarizer");
					const sdkMessages = convertWasmMessagesToAgentMessages(wasmMessages);
					return config.model.summarize!(sdkMessages, sig);
				}
			: async (wasmMessages: WasmAgentMessage[], sig?: AbortSignal) => {
					logger.info("Using default summarizer");
					return defaultSummarizer(config.model, wasmMessages, sig);
				},
	};

	try {
		const result = await runTurnWithHostAgent(hostAgent, userMessage, {
			llm: llmProvider,
			tools: toolMap,
			llmTools,
			logger,
			onEvent: (rawEvent: RawAgentEvent) => {
				const semanticEvents = eventMapper.map(rawEvent, runState);
				for (const ev of semanticEvents) {
					callbacks.onEvent(ev);
				}
			},
			onMarkers: (markers) => {
				const artifactEvents = eventMapper.processMarkers(markers, runState);
				for (const ev of artifactEvents) {
					callbacks.onEvent(ev);
				}
			},
			onPersist: async (data: PersistData) => {
				callbacks.onStatus({ state: "saving", message: "Saving session..." });
				logger.info("Persisting session", { sessionId: config.sessionId });
				if (config.store) {
					const serializer = new SnapshotSerializer();
					const snapshot = serializer.serialize(data);
					await config.store.saveSession(config.sessionId, snapshot);
					logger.info("Session saved", { sessionId: config.sessionId });
				}
				callbacks.onStatus({ state: "completed" });
			},
			artifactStore,
			signal,
		});

		if (result.aborted) {
			logger.info("Turn aborted");
			callbacks.onStatus({ state: "aborted", message: "Stopped by user" });
		} else {
			logger.info("Turn completed", { status: "completed" });
		}
		return eventMapper.buildRunResult(runState, result);
	} catch (e) {
		logger.error("Turn failed", { error: e instanceof Error ? e.message : String(e) });
		throw e;
	}
}

export async function steerAgent(hostAgent: HostAgent, input: string | AgentInput): Promise<void> {
	const text = typeof input === "string" ? input : input.text;
	const message: WasmAgentMessage = {
		role: "user",
		content: [{ type: "text", text }],
		timestamp: Date.now(),
	};
	hostAgent.steer(message);
}

export async function resetAgentState(hostAgent: HostAgent): Promise<void> {
	unwrap(hostReset(hostAgent.handle));
	// Caller must set engineAgent = null after this
}

// --- Helpers ---

function modelStreamToLlmStream(
	stream: AsyncIterable<ModelEvent>,
	signal: AbortSignal,
	runState: { usage?: import("../types.ts").TokenUsage },
): LlmStream {
	let textAccumulator = "";
	const toolCalls = new Map<string, { id: string; name: string; arguments: string }>();
	let stopReason: "end" | "tool_call" | "length" | "error" = "end";
	let modelId: string | undefined;
	let usage: import("../types.ts").TokenUsage | undefined;
	let streamError: unknown;

	// Promise that resolves when chunks iteration finishes
	let chunksDoneResolve: () => void;
	const chunksDone = new Promise<void>((r) => { chunksDoneResolve = r; });

	const chunks: AsyncIterable<LlmChunk> = {
		[Symbol.asyncIterator]: async function* () {
			try {
				for await (const event of stream) {
					if (signal.aborted) return;
					switch (event.type) {
						case "text_delta": {
							const text = event.payload as string;
							textAccumulator += text;
							yield { kind: "text_delta", text };
							break;
						}
						case "tool_call_delta": {
							const delta = event.payload as { id: string; name: string; arguments?: unknown; delta?: unknown };
							const existing = toolCalls.get(delta.id);
							const argumentFragment = stringifyToolArguments(delta.arguments ?? delta.delta ?? "");
							toolCalls.set(delta.id, {
								id: delta.id,
								name: delta.name || existing?.name || "",
								arguments: (existing?.arguments ?? "") + argumentFragment,
							});
							yield { kind: "tool_call_delta", tool_call_id: delta.id, delta: { type: "string", value: argumentFragment } };
							break;
						}
						case "done": {
							const response = event.payload as ModelResponse;
							modelId = response.model;
							usage = response.usage;
							if (response.usage) runState.usage = response.usage;
							stopReason = response.stopReason;
							for (const block of response.content) {
								if (block.type !== "tool_call") continue;
								toolCalls.set(block.id, {
									id: block.id,
									name: block.name,
									arguments: stringifyToolArguments(block.arguments),
								});
							}
							break;
						}
					}
				}
			} catch (e) {
				streamError = e;
			} finally {
				chunksDoneResolve();
			}
		},
	};

	const result: Promise<LlmResult> = (async () => {
		// Wait for chunks to be fully consumed by the engine
		await chunksDone;

		if (streamError) {
			return { Err: { error: { code: "stream_error", message: streamError instanceof Error ? streamError.message : String(streamError) }, aborted: false } } as LlmResult;
		}

		const content: Content[] = [];
		if (textAccumulator) {
			content.push({ type: "text" as const, text: textAccumulator });
		}
		for (const tc of toolCalls.values()) {
			content.push({ type: "tool_call" as const, id: tc.id, name: tc.name, arguments: parseToolArguments(tc.arguments) });
		}

		return {
			Ok: {
				content,
				api: "sdk",
				provider: "sdk",
				model: modelId ?? "sdk-model",
				stop_reason: toWasmStopReason(stopReason),
				error_message: toWasmStopReason(stopReason) === "error" ? "Model returned an error stop reason" : undefined,
				timestamp: Date.now(),
				usage: {
					input: usage?.input ?? 0,
					output: usage?.output ?? 0,
					cache_read: usage?.cache_read ?? 0,
					cache_write: usage?.cache_write ?? 0,
					total_tokens: usage?.total_tokens ?? 0,
				},
			},
		};
	})();

	return { chunks, result };
}

function stringifyToolArguments(argumentsValue: unknown): string {
	if (typeof argumentsValue === "string") {
		return argumentsValue;
	}
	return JSON.stringify(argumentsValue) ?? "";
}

function parseToolArguments(argumentsText: string): unknown {
	try {
		return JSON.parse(argumentsText);
	} catch {
		return argumentsText;
	}
}

async function processStepMarkers(
	step: { markers?: Array<{ type: string; entry_ids?: string[] }> },
	hostAgent: HostAgent,
	config: AgentRunConfig,
): Promise<void> {
	if (!step.markers) return;
	for (const marker of step.markers) {
		if (marker.type === "new_artifacts") {
			for (const artifactId of marker.entry_ids ?? []) {
				const content = hostReadArtifact(hostAgent.handle, artifactId);
				await config.artifactStore?.save(hostAgent.sessionId ?? "default", artifactId, content);
			}
		}
	}
	config.onMarkers?.(step.markers);
}

function toolErrorFromUnknown(e: unknown): ToolError {
	if (e instanceof HostError) {
		return { code: e.code, message: e.message };
	}
	return {
		code: "tool_failed",
		message: e instanceof Error ? e.message : String(e),
	};
}

function unwrap<T>(result: { ok: boolean; data?: T; error?: { code: string; message: string } }): T {
	if (!result.ok) {
		throw new HostError(result.error!.code, result.error!.message);
	}
	return result.data!;
}

function normalizeTools(tools: import("../types.ts").AgentTools | import("../types.ts").AgentTools[] | undefined): import("../types.ts").AgentTools[] {
	if (!tools) return [];
	if (Array.isArray(tools)) return tools;
	return [tools];
}

function buildContextBudget(context?: import("../types.ts").AgentContextPolicy): import("../../pi_host_web.js").ContextProjectionBudget {
	return {
		max_tool_result_chars: context?.toolResultLimit ?? 50000,
		max_context_tokens: context?.maxTokens ?? 100000,
		microcompact_after_turns: 5,
		compaction_threshold: 0.75,
	};
}

function buildModelOptions(model: import("../types.ts").AgentModel): import("../../pi_host_web.js").Model {
	return {
		id: model.id ?? "custom-model",
		name: model.id ?? "custom-model",
		api: "anthropic",
		provider: "anthropic",
		reasoning: false,
		context_window: model.contextWindow ?? 100000,
		max_tokens: model.maxTokens ?? 4096,
		capabilities: {
			vision: model.capabilities?.vision ?? false,
			json_mode: model.capabilities?.jsonMode ?? true,
			function_calling: model.capabilities?.functionCalling ?? true,
			streaming: model.capabilities?.streaming ?? true,
		},
		cost: { input: 0, output: 0, cache_read: 0, cache_write: 0 },
	};
}

function buildArtifactStore(config: AgentConfig): ArtifactStore | undefined {
	if (config.artifacts?.mode === "external" && config.store) {
		const store = config.store;
		if (typeof store.saveArtifact !== "function" || typeof store.loadArtifact !== "function") {
			throw createAgentError(
				"store_artifact_unsupported",
				"Store does not support artifact operations but external artifact mode is configured",
				{ recoverable: false },
			);
		}
		return {
			save: (sessionId: string, artifactId: string, content: string) =>
				store.saveArtifact!(sessionId, {
					id: artifactId,
					kind: "text",
					content,
					createdAt: Date.now(),
				}),
			load: (sessionId: string, artifactId: string) =>
				store.loadArtifact!(sessionId, artifactId).then((a: import("../types.ts").AgentArtifact | null) =>
					a && typeof a.content === "string" ? a.content : null,
				),
			search: (sessionId: string, query: string) => {
				if (typeof store.searchArtifacts !== "function") {
					return Promise.resolve([]);
				}
				return store.searchArtifacts!(sessionId, { text: query }).then((results: import("../types.ts").ArtifactSearchResult[]) =>
					results.map((r: import("../types.ts").ArtifactSearchResult) => ({
						id: r.artifact.id,
						snippet: r.snippet ?? "",
						match_count: r.matchCount ?? 0,
					})),
				);
			},
		};
	}
	return undefined;
}

function mergeMetadata(
	input: string | AgentInput,
	runMetadata?: Record<string, unknown>,
): Record<string, unknown> | undefined {
	const inputMetadata = typeof input === "object" ? input.metadata : undefined;
	if (!inputMetadata && !runMetadata) return undefined;
	return { ...inputMetadata, ...runMetadata };
}

function buildUserMessage(input: string | AgentInput): WasmAgentMessage {
	const text = typeof input === "string" ? input : input.text;
	const content: Content[] = [{ type: "text", text }];

	if (typeof input === "object" && input.attachments) {
		for (const attachment of input.attachments) {
			if (attachment.type === "image" || attachment.mimeType?.startsWith("image/")) {
				content.push({
					type: "image",
					media_type: attachment.mimeType ?? "image/png",
					data: typeof attachment.content === "string" ? attachment.content : btoa(String.fromCharCode(...new Uint8Array(attachment.content))),
				});
			}
		}
	}

	return {
		role: "user",
		content,
		timestamp: Date.now(),
	};
}

function convertWasmMessagesToAgentMessages(
	messages: WasmAgentMessage[],
): AgentMessage[] {
	return messages.map((msg) => ({
		id: stableMessageId(msg),
		role: msg.role,
		content: msg.content.map((c) => {
			if (c.type === "text") return { type: "text" as const, text: c.text };
			if (c.type === "tool_call") return { type: "tool_call" as const, id: c.id, name: c.name, arguments: c.arguments };
			if (c.type === "image") return { type: "image" as const, mimeType: c.media_type, data: c.data };
			return { type: "text" as const, text: "" };
		}),
		timestamp: Date.now(),
		tool_call_id: msg.role === "tool_result" ? (msg as unknown as { tool_call_id: string }).tool_call_id : undefined,
	}));
}

function stableMessageId(msg: WasmAgentMessage): string {
	const contentHash = msg.content.map((c) => {
		if (c.type === "text") return `t:${c.text?.slice(0, 64) ?? ""}`;
		if (c.type === "tool_call") return `tc:${c.id ?? ""}:${c.name ?? ""}`;
		if (c.type === "image") return `img:${c.media_type ?? ""}`;
		return (c as { type: string }).type;
	}).join("|");
	return `msg-${msg.role}-${msg.timestamp ?? 0}-${contentHash}`;
}

function toWasmStopReason(reason: ModelResponse["stopReason"]): "end_turn" | "tool_use" | "max_tokens" | "error" {
	switch (reason) {
		case "tool_call":
			return "tool_use";
		case "length":
			return "max_tokens";
		case "error":
			return "error";
		case "end":
		default:
			return "end_turn";
	}
}

function modelResponseToLlmStream(
	response: ModelResponse,
	signal: AbortSignal,
): LlmStream {
	const chunks: AsyncIterable<LlmChunk> = {
		[Symbol.asyncIterator]: async function* () {
			if (signal.aborted) return;

			// Start chunk — stop_reason belongs on final result, not start
			yield {
				kind: "start",
				content: [{ type: "text", text: "" }],
				api: "sdk",
				provider: "sdk",
				model: response.model ?? "sdk-model",
				stop_reason: "end_turn" as const,
				error_message: undefined,
				timestamp: Date.now(),
				usage: {
					input: response.usage?.input ?? 0,
					output: response.usage?.output ?? 0,
					cache_read: response.usage?.cache_read ?? 0,
					cache_write: response.usage?.cache_write ?? 0,
					total_tokens: response.usage?.total_tokens ?? 0,
				},
			};

			// Text delta chunks for each text block
			for (const block of response.content) {
				if (signal.aborted) return;
				if (block.type === "text" && block.text) {
					// Split into words to simulate streaming
					const words = block.text.split(/(\s+)/);
					for (const word of words) {
						if (signal.aborted) return;
						if (word) {
							yield { kind: "text_delta", text: word };
							// Small artificial delay to simulate streaming
							await new Promise((r) => setTimeout(r, 10));
						}
					}
				}
			}
		},
	};

	const result: Promise<LlmResult> = Promise.resolve({
		Ok: {
			content: response.content.map((c: import("../types.ts").AgentContentBlock) => {
				if (c.type === "text") return { type: "text", text: c.text };
				if (c.type === "tool_call") return { type: "tool_call", id: c.id, name: c.name, arguments: c.arguments };
				return { type: "text", text: "" };
			}),
			api: "sdk",
			provider: "sdk",
			model: response.model ?? "sdk-model",
			stop_reason: toWasmStopReason(response.stopReason),
			error_message: response.stopReason === "error" ? "Model returned an error stop reason" : undefined,
			timestamp: Date.now(),
			usage: {
				input: response.usage?.input ?? 0,
				output: response.usage?.output ?? 0,
				cache_read: response.usage?.cache_read ?? 0,
				cache_write: response.usage?.cache_write ?? 0,
				total_tokens: response.usage?.total_tokens ?? 0,
			},
		},
	});

	return { chunks, result };
}

async function defaultSummarizer(
	model: import("../types.ts").AgentModel,
	messages: WasmAgentMessage[],
	signal?: AbortSignal,
): Promise<string> {
	const summaryRequest: import("../types.ts").ModelRequest = {
		instructions: "Summarize the following conversation context concisely. Preserve key facts, decisions, and action items. Omit redundant details.",
		messages: convertWasmMessagesToAgentMessages(messages),
		tools: [],
		signal,
	};
	const response = await model.generate(summaryRequest);
	const text = response.content
		.filter((c): c is { type: "text"; text: string } => c.type === "text")
		.map((c) => c.text)
		.join("\n");
	return text || "[Context summarized]";
}
