import type { Content, LlmChunk, LlmResult, AgentMessage as WasmAgentMessage } from "../../pi_host_web.js";
import type { LlmStream } from "../bindings/types.ts";
import type { AgentContentBlock, AgentModel, ModelEvent, ModelRequest, ModelResponse, TokenUsage } from "../types.ts";
import { convertWasmMessagesToAgentMessages } from "./config-builders.ts";

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

export function toWasmStopReason(
	reason: ModelResponse["stopReason"],
): "end_turn" | "tool_use" | "max_tokens" | "error" {
	switch (reason) {
		case "tool_call":
			return "tool_use";
		case "length":
			return "max_tokens";
		case "error":
			return "error";
		default:
			return "end_turn";
	}
}

export function modelStreamToLlmStream(
	stream: AsyncIterable<ModelEvent>,
	signal: AbortSignal,
	runState: { usage?: TokenUsage },
): LlmStream {
	let textAccumulator = "";
	const toolCalls = new Map<string, { id: string; name: string; arguments: string }>();
	const streamState: { stopReason: "end" | "tool_call" | "length" | "error" } = { stopReason: "end" };
	let modelId: string | undefined;
	let usage: TokenUsage | undefined;
	let streamError: unknown;

	let chunksDoneResolve: () => void;
	const chunksDone = new Promise<void>((r) => {
		chunksDoneResolve = r;
	});

	const chunks: AsyncIterable<LlmChunk> = {
		[Symbol.asyncIterator]: async function* () {
			try {
				for await (const event of stream) {
					if (signal.aborted) return;
					switch (event.type) {
						case "start": {
							const msg = event.payload as Record<string, unknown>;
							yield { kind: "start", ...msg } as LlmChunk;
							break;
						}
						case "text_delta": {
							const text = event.payload as string;
							textAccumulator += text;
							yield { kind: "text_delta", text };
							break;
						}
						case "tool_call_delta": {
							const delta = event.payload as {
								id: string;
								name: string;
								arguments?: unknown;
								delta?: unknown;
							};
							const existing = toolCalls.get(delta.id);
							const argumentFragment = stringifyToolArguments(delta.arguments ?? delta.delta ?? "");
							toolCalls.set(delta.id, {
								id: delta.id,
								name: delta.name || existing?.name || "",
								arguments: (existing?.arguments ?? "") + argumentFragment,
							});
							yield {
								kind: "tool_call_delta",
								tool_call_id: delta.id,
								delta: { type: "string", value: argumentFragment },
							};
							break;
						}
						case "done": {
							const response = event.payload as ModelResponse;
							modelId = response.model;
							usage = response.usage;
							if (response.usage) runState.usage = response.usage;
							streamState.stopReason = response.stopReason;
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
		await chunksDone;

		if (streamError) {
			return {
				Err: {
					error: {
						code: "stream_error",
						message: streamError instanceof Error ? streamError.message : String(streamError),
					},
					aborted: false,
				},
			} as LlmResult;
		}

		if (streamState.stopReason === "error") {
			return {
				Err: {
					error: {
						code: "model_error",
						message: "Model returned an error stop reason",
					},
					aborted: false,
				},
			} as LlmResult;
		}

		const content: Content[] = [];
		if (textAccumulator) {
			content.push({ type: "text" as const, text: textAccumulator });
		}
		for (const tc of toolCalls.values()) {
			content.push({
				type: "tool_call" as const,
				id: tc.id,
				name: tc.name,
				arguments: parseToolArguments(tc.arguments),
			});
		}

		return {
			Ok: {
				content,
				api: "sdk",
				provider: "sdk",
				model: modelId ?? "sdk-model",
				stop_reason: toWasmStopReason(streamState.stopReason),
				error_message:
					toWasmStopReason(streamState.stopReason) === "error" ? "Model returned an error stop reason" : undefined,
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

export function modelResponseToLlmStream(response: ModelResponse, signal: AbortSignal): LlmStream {
	const chunks: AsyncIterable<LlmChunk> = {
		[Symbol.asyncIterator]: async function* () {
			if (signal.aborted) return;

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

			for (const block of response.content) {
				if (signal.aborted) return;
				if (block.type === "text" && block.text) {
					const words = block.text.split(/(\s+)/);
					for (const word of words) {
						if (signal.aborted) return;
						if (word) {
							yield { kind: "text_delta", text: word };
							await new Promise((r) => setTimeout(r, 10));
						}
					}
				}
			}
		},
	};

	const result: Promise<LlmResult> =
		response.stopReason === "error"
			? Promise.resolve({
					Err: {
						error: {
							code: "model_error",
							message: "Model returned an error stop reason",
						},
						aborted: false,
					},
				} as LlmResult)
			: Promise.resolve({
					Ok: {
						content: response.content.map((c: AgentContentBlock) => {
							if (c.type === "text") return { type: "text", text: c.text };
							if (c.type === "tool_call")
								return {
									type: "tool_call",
									id: c.id,
									name: c.name,
									arguments: c.arguments,
								};
							return { type: "text", text: "" };
						}),
						api: "sdk",
						provider: "sdk",
						model: response.model ?? "sdk-model",
						stop_reason: toWasmStopReason(response.stopReason),
						error_message: undefined,
						timestamp: Date.now(),
						usage: {
							input: response.usage?.input ?? 0,
							output: response.usage?.output ?? 0,
							cache_read: response.usage?.cache_read ?? 0,
							cache_write: response.usage?.cache_write ?? 0,
							total_tokens: response.usage?.total_tokens ?? 0,
						},
					},
				} as LlmResult);

	return { chunks, result };
}

export async function defaultSummarizer(
	model: AgentModel,
	messages: WasmAgentMessage[],
	signal?: AbortSignal,
): Promise<string> {
	const summaryRequest: ModelRequest = {
		instructions:
			"Summarize the following conversation context concisely. Preserve key facts, decisions, and action items. Omit redundant details.",
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
