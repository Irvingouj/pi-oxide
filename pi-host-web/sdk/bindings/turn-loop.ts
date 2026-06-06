import {
	hostAbort,
	hostAcceptCompaction,
	hostContinueTurn,
	hostFeedLlmChunk,
	hostLlmDone,
	hostPrepareToolCalls,
	hostToolCancelled,
	hostToolDone,
	hostToolFailed,
	startTurn,
	type AgentMessage as WasmAgentMessage,
} from "../../pi_host_web.js";
import { getLogger } from "../internal/logger.ts";
import type { HostAgent } from "./host-agent.ts";
import { HostError, unwrap } from "./init.ts";
import { processStepMarkers } from "./markers.ts";
import { buildToolCallPreparations, toolErrorFromUnknown } from "./tool-preparation.ts";
import type { AgentRunConfig, TurnResult } from "./types.ts";

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
			} catch (_e) {
				// ignore wrong_phase errors
			}
			throw new HostError("user_aborted", "Turn stopped by user");
		}
	};

	try {
		logger.info("Starting turn", { sessionId: hostAgent.sessionId });
		let step = unwrap(startTurn(hostAgent.handle, { prompt: message, tools: config.llmTools })) as {
			events: unknown[];
			directives?: Array<{ type: string; [key: string]: unknown }>;
			markers?: Array<{ type: string; entry_ids?: string[] }>;
		};
		for (const event of step.events) {
			config.onEvent?.(event as import("../../pi_host_web.js").AgentEvent);
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
						const ctx = action.context as import("../../pi_host_web.js").LlmContext;
						logger.info("Streaming LLM", {
							messageCount: ctx.messages.length,
							toolCount: ctx.tools.length,
						});
						const stream = await config.llm.call(ctx, signal);
						for await (const chunk of stream.chunks) {
							checkAbort();
							const ev = unwrap(hostFeedLlmChunk(hostAgent.handle, chunk)) as { events: unknown[] };
							for (const e of ev.events) config.onEvent?.(e as import("../../pi_host_web.js").AgentEvent);
						}
						checkAbort();
						const result = await stream.result;
						step = unwrap(hostLlmDone(hostAgent.handle, result)) as typeof step;
						for (const e of step.events) config.onEvent?.(e as import("../../pi_host_web.js").AgentEvent);
						await processStepMarkers(step, hostAgent, config);
						stateAdvanced = true;
						break;
					}

					case "prepare_tool_calls": {
						const calls = action.calls as import("../../pi_host_web.js").ToolCall[];
						logger.info("Preparing tool calls", { count: calls.length, names: calls.map((c) => c.name) });
						const preparations = await buildToolCallPreparations(calls, config.prepareToolCalls, logger);
						if (preparations.hadError) {
							logger.warn("Tool call preparation failed, blocked remaining calls", {
								error: preparations.errorMessage,
							});
						}
						step = unwrap(hostPrepareToolCalls(hostAgent.handle, JSON.stringify(preparations.items))) as typeof step;
						for (const e of step.events) config.onEvent?.(e as import("../../pi_host_web.js").AgentEvent);
						await processStepMarkers(step, hostAgent, config);
						stateAdvanced = true;
						break;
					}

					case "execute_tools": {
						const calls = action.calls as import("../../pi_host_web.js").ToolCall[];
						logger.info("Executing tools", { count: calls.length, names: calls.map((c) => c.name) });
						for (const call of calls) {
							checkAbort();
							const handler = config.tools[call.name];
							if (!handler) {
								logger.warn("Tool handler not found", { toolName: call.name });
								step = unwrap(
									hostToolFailed(hostAgent.handle, call.id, {
										code: "tool_not_found",
										message: `No handler for ${call.name}`,
									}),
								) as typeof step;
							} else {
								try {
									const result = await handler(call);
									logger.debug("Tool completed", { toolName: call.name });
									step = unwrap(hostToolDone(hostAgent.handle, call.id, result)) as typeof step;
								} catch (e) {
									logger.warn("Tool failed", {
										toolName: call.name,
										error: e instanceof Error ? e.message : String(e),
									});
									step = unwrap(hostToolFailed(hostAgent.handle, call.id, toolErrorFromUnknown(e))) as typeof step;
								}
							}
							for (const e of step.events) config.onEvent?.(e as import("../../pi_host_web.js").AgentEvent);
							await processStepMarkers(step, hostAgent, config);
						}
						if ((step.directives ?? []).length === 0) {
							step = unwrap(hostContinueTurn(hostAgent.handle)) as typeof step;
							for (const e of step.events) config.onEvent?.(e as import("../../pi_host_web.js").AgentEvent);
							await processStepMarkers(step, hostAgent, config);
						}
						stateAdvanced = true;
						break;
					}

					case "cancel_tools": {
						const ids = action.tool_call_ids as string[];
						logger.info("Cancelling tools", { count: ids.length, reason: action.reason });
						for (const id of ids) {
							step = unwrap(hostToolCancelled(hostAgent.handle, id, action.reason as import("../../pi_host_web.js").CancelReason)) as typeof step;
							for (const e of step.events) config.onEvent?.(e as import("../../pi_host_web.js").AgentEvent);
							await processStepMarkers(step, hostAgent, config);
						}
						stateAdvanced = true;
						break;
					}

					case "summarize": {
						const ctx = action.context as import("../../pi_host_web.js").LlmContext;
						logger.info("Summarizing context");
						const summary = await config.llm.summarize?.(ctx.messages, signal);
						step = unwrap(hostAcceptCompaction(hostAgent.handle, summary, [])) as typeof step;
						for (const e of step.events) config.onEvent?.(e as import("../../pi_host_web.js").AgentEvent);
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
						step = unwrap(hostContinueTurn(hostAgent.handle)) as typeof step;
						for (const e of step.events) config.onEvent?.(e as import("../../pi_host_web.js").AgentEvent);
						await processStepMarkers(step, hostAgent, config);
						stateAdvanced = true;
						break;

					default:
						logger.warn("Unknown directive type", { type: action.type });
						break;
				}
			}

			if (turnFinished) {
				logger.info("Turn finished");
				return { aborted: false };
			}

			if (!stateAdvanced && (step.directives ?? []).length === 0) {
				logger.debug("No state advanced, continuing turn");
				step = unwrap(hostContinueTurn(hostAgent.handle)) as typeof step;
				for (const e of step.events) config.onEvent?.(e as import("../../pi_host_web.js").AgentEvent);
				await processStepMarkers(step, hostAgent, config);
			}
		}
	} catch (e: unknown) {
		const isUserAbort =
			(e instanceof HostError && e.code === "user_aborted") || (e instanceof DOMException && e.name === "AbortError");
		if (isUserAbort) {
			logger.info("Turn aborted by user");
			return { aborted: true };
		}
		logger.error("Turn failed", { error: e instanceof Error ? e.message : String(e) });
		throw e;
	}
}
