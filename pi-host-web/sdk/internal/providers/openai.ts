// openaiCompatible() and openai() factories — OpenAI-compatible provider adapters.
// openai() is a thin wrapper with baseUrl: "https://api.openai.com" (no /v1).
// openaiCompatible() appends /v1/chat/completions to the baseUrl.
// Correct message format: content: string | null, tool_calls: [...]
// Response parsing with tool_calls into AgentContentBlock[].

import { createAgentError } from "../../errors.ts";
import { getLogger } from "../../internal/logger.ts";
import { isRecord } from "../../internal/util/types.ts";
import type {
	AgentContentBlock,
	AgentModel,
	ModelRequest,
	ModelResponse,
} from "../../types.ts";

export function openaiCompatible(config: {
	apiKey: string;
	baseUrl: string;
	model: string;
	maxTokens?: number;
}): AgentModel {
	const logger = getLogger("openai");
	return {
		id: config.model,
		contextWindow: 128000,
		maxTokens: config.maxTokens ?? 4096,
		capabilities: {
			vision:
				config.model.includes("vision") || config.model.includes("gpt-4o"),
			jsonMode: true,
			functionCalling: true,
			streaming: true,
		},
		async generate(request: ModelRequest): Promise<ModelResponse> {
			logger.info("OpenAI generate", {
				model: config.model,
				messageCount: request.messages.length,
			});
			// Convert AgentMessage[] -> OpenAI Chat Completions message format
			const messages = request.messages.map((msg) => {
				switch (msg.role) {
					case "user": {
						const text = msg.content
							.filter(
								(c): c is { type: "text"; text: string } => c.type === "text",
							)
							.map((c) => c.text)
							.join("\n");
						return { role: "user" as const, content: text };
					}
					case "assistant": {
						const textBlocks = msg.content
							.filter(
								(c): c is { type: "text"; text: string } => c.type === "text",
							)
							.map((c) => c.text)
							.join("");
						const toolCalls = msg.content
							.filter(
								(
									c,
								): c is {
									type: "tool_call";
									id: string;
									name: string;
									arguments: unknown;
								} => c.type === "tool_call",
							)
							.map((c) => ({
								id: c.id,
								type: "function" as const,
								function: {
									name: c.name,
									arguments: JSON.stringify(c.arguments),
								},
							}));
						return {
							role: "assistant" as const,
							content: textBlocks || null,
							tool_calls: toolCalls.length > 0 ? toolCalls : undefined,
						};
					}
					case "tool_result": {
						const text = msg.content
							.filter(
								(c): c is { type: "text"; text: string } => c.type === "text",
							)
							.map((c) => c.text)
							.join("\n");
						return {
							role: "tool" as const,
							tool_call_id: msg.tool_call_id ?? "",
							content: text,
						};
					}
					default: {
						return { role: "user" as const, content: "" };
					}
				}
			});

			// Convert AgentToolDefinition[] -> OpenAI functions format
			const tools = request.tools.map((t) => ({
				type: "function" as const,
				function: {
					name: t.name,
					description: t.description,
					parameters: isRecord(t.inputSchema)
						? t.inputSchema
						: { type: "object", properties: {} },
				},
			}));

			const body = {
				model: config.model,
				messages,
				tools: tools.length > 0 ? tools : undefined,
				max_tokens: config.maxTokens,
			};

			try {
				const resp = await fetch(
					`${config.baseUrl.replace(/\/+$/, "")}/v1/chat/completions`,
					{
						method: "POST",
						headers: {
							"Content-Type": "application/json",
							Authorization: `Bearer ${config.apiKey}`,
						},
						body: JSON.stringify(body),
						signal: request.signal,
					},
				);

				if (!resp.ok) {
					const status = resp.status;
					const text = await resp.text();
					logger.warn("OpenAI API error", { status, body: text.slice(0, 500) });
					throw createAgentError(
						status === 401
							? "model_auth_failed"
							: status === 429
								? "model_rate_limited"
								: "model_unavailable",
						`HTTP ${status}: ${text}`,
						{ recoverable: status === 429 },
					);
				}

				const data = await resp.json();
				const choice = data.choices?.[0];
				const message = choice?.message;

				logger.info("OpenAI response", {
					finishReason: choice?.finish_reason,
					model: data.model,
				});

				// Parse content and tool_calls from OpenAI response
				const content: AgentContentBlock[] = [];

				if (message?.content && typeof message.content === "string") {
					content.push({ type: "text", text: message.content });
				}

				if (message?.tool_calls && Array.isArray(message.tool_calls)) {
					for (const tc of message.tool_calls) {
						if (tc.type === "function") {
							content.push({
								type: "tool_call",
								id: tc.id ?? "",
								name: tc.function?.name ?? "",
								arguments: (() => {
									try {
										return JSON.parse(tc.function?.arguments ?? "{}");
									} catch {
										return {};
									}
								})(),
							});
						}
					}
				}

				return {
					content,
					stopReason:
						choice?.finish_reason === "tool_calls"
							? "tool_call"
							: choice?.finish_reason === "stop"
								? "end"
								: choice?.finish_reason === "length"
									? "length"
									: "error",
					usage: data.usage
						? {
								input: data.usage.prompt_tokens,
								output: data.usage.completion_tokens,
								cache_read: 0,
								cache_write: 0,
								total_tokens: data.usage.total_tokens,
							}
						: undefined,
					model: config.model,
					raw: data,
				};
			} catch (e) {
				if (e && typeof e === "object" && "code" in e) throw e;
				logger.error("OpenAI request failed", {
					error: e instanceof Error ? e.message : String(e),
				});
				throw createAgentError(
					"model_unavailable",
					e instanceof Error ? e.message : String(e),
					{
						cause: e,
						recoverable: false,
					},
				);
			}
		},
	};
}

// openai() passes baseUrl WITHOUT /v1; openaiCompatible() appends /v1/chat/completions
export function openai(config: {
	apiKey: string;
	model: string;
	maxTokens?: number;
}): AgentModel {
	return openaiCompatible({
		apiKey: config.apiKey,
		baseUrl: "https://api.openai.com",
		model: config.model,
		maxTokens: config.maxTokens,
	});
}
