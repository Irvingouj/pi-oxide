import type {
	LlmContext,
	PersistData,
	AgentEvent as RawAgentEvent,
} from "../../pi_host_web.js";
import { hostReset } from "../../pi_host_web.js";
import {
	createHostAgentInstance,
	type HostAgent,
} from "../bindings/host-agent.ts";
import { unwrap } from "../bindings/init.ts";
import { runTurnWithHostAgent } from "../bindings/turn-loop.ts";
import type { AgentRunConfig, LlmStream } from "../bindings/types.ts";
import { EventMapper } from "../internal/events.ts";
import { getLogger } from "../internal/logger.ts";
import { ToolRegistryBuilder } from "../internal/tools/registry.ts";
import { SnapshotSerializer } from "../snapshot.ts";
import type {
	AgentConfig,
	AgentInput,
	AgentRunOptions,
	AgentRunResult,
	AgentStatus,
} from "../types.ts";
import {
	buildArtifactStore,
	buildUserMessage,
	convertWasmMessagesToAgentMessages,
	mergeMetadata,
	normalizeTools,
} from "./config-builders.ts";
import {
	defaultSummarizer,
	modelResponseToLlmStream,
	modelStreamToLlmStream,
} from "./model-adapter.ts";

export type { HostAgent } from "../bindings/host-agent.ts";
export { createHostAgentInstance } from "../bindings/host-agent.ts";
export { runTurnWithHostAgent } from "../bindings/turn-loop.ts";
export type {
	AgentRunConfig,
	LlmStream,
	TurnResult,
} from "../bindings/types.ts";

export async function createEngineAgent(
	config: AgentConfig,
	_callbacks: {
		onEvent: (event: { type: string; payload: unknown }) => void;
		onStatus: (status: AgentStatus) => void;
	},
): Promise<HostAgent> {
	const logger = config.logger ?? getLogger("engine");

	let sessionState: PersistData | undefined;
	if (config.store) {
		const snapshot = await config.store.loadSession(config.sessionId);
		if (snapshot) {
			const serializer = new SnapshotSerializer();
			sessionState = serializer.deserialize(snapshot) as
				| PersistData
				| undefined;
			if (sessionState) {
				logger.info("Session snapshot loaded", { sessionId: config.sessionId });
			} else {
				logger.warn("Session snapshot version mismatch, starting fresh", {
					sessionId: config.sessionId,
				});
			}
		}
	}

	return createHostAgentInstance(config, sessionState);
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
	const userMessage = buildUserMessage(input);
	const eventMapper = new EventMapper();
	const runState = eventMapper.createRunState();
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

	const llmProvider: AgentRunConfig["llm"] = {
		call: async (context: LlmContext, s?: AbortSignal): Promise<LlmStream> => {
			const effectiveSignal = s || signal;
			callbacks.onStatus({
				state: "calling_model",
				message: "Calling model...",
			});
			logger.info("Calling model", { model: config.model.id ?? "custom" });

			const modelRequest = {
				instructions: context.system_prompt,
				messages: convertWasmMessagesToAgentMessages(context.messages),
				tools: llmTools.map((t) => ({
					name: t.name,
					description: t.description,
					inputSchema: t.parameters,
					run: () => null,
				})),
				signal: effectiveSignal,
				metadata: mergeMetadata(input, options?.metadata),
			};

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
			? async (wasmMessages, sig) => {
					logger.info("Calling model summarizer");
					const sdkMessages = convertWasmMessagesToAgentMessages(wasmMessages);
					return config.model.summarize?.(sdkMessages, sig);
				}
			: async (wasmMessages, sig) => {
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
			prepareToolCalls: config.prepareToolCalls,
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
		logger.error("Turn failed", {
			error: e instanceof Error ? e.message : String(e),
		});
		throw e;
	}
}

export async function steerAgent(
	hostAgent: HostAgent,
	input: string | AgentInput,
): Promise<void> {
	const text = typeof input === "string" ? input : input.text;
	hostAgent.steer({
		role: "user",
		content: [{ type: "text", text }],
		timestamp: Date.now(),
	});
}

export async function resetAgentState(hostAgent: HostAgent): Promise<void> {
	unwrap(hostReset(hostAgent.handle));
}
