/**
 * LLM provider service — wraps Anthropic/Fireworks API calls and streaming chunk generation.
 *
 * Pure JS, no React. Produces an object compatible with the SDK's LlmProvider interface.
 */

import type {
	LlmChunk,
	LlmContext,
	LlmResult,
	LlmStream,
} from "@pi-oxide/pi-host-web";
import { getApiBaseUrl, getApiKey, getModel } from "../browser/config.ts";

// --- Internal types ---

interface LlmResponseBlock {
	type: string;
	text?: string;
	id?: string;
	name?: string;
	input?: unknown;
}

interface LlmResponse {
	content: LlmResponseBlock[];
	stop_reason: string;
}

interface AgentMessageShape {
	role: string;
	content: Array<{
		type: string;
		text?: string;
		id?: string;
		name?: string;
		arguments?: Record<string, unknown>;
		tool_call_id?: string;
		is_error?: boolean;
	}>;
}

// --- Message conversion (mirrors browserLlm.ts) ---

function convertMessages(messages: AgentMessageShape[]): unknown[] {
	const result: unknown[] = [];
	let i = 0;
	while (i < messages.length) {
		const msg = messages[i];
		if (msg.role === "user") {
			const text = msg.content
				.filter(
					(b): b is typeof b & { text: string } =>
						b.type === "text" && !!b.text,
				)
				.map((b) => b.text)
				.join("\n");
			result.push({ role: "user", content: text });
			i++;
		} else if (msg.role === "assistant") {
			const blocks: unknown[] = [];
			for (const b of msg.content) {
				if (b.type === "text" && b.text)
					blocks.push({ type: "text", text: b.text });
				else if (b.type === "tool_call" && b.id && b.name)
					blocks.push({
						type: "tool_use",
						id: b.id,
						name: b.name,
						input: b.arguments || {},
					});
			}
			result.push({ role: "assistant", content: blocks });
			i++;
		} else if (msg.role === "tool_result") {
			const trs: unknown[] = [];
			while (i < messages.length && messages[i].role === "tool_result") {
				const tr = messages[i];
				trs.push({
					type: "tool_result",
					tool_use_id: tr.tool_call_id,
					content: tr.content
						.filter(
							(b): b is typeof b & { text: string } =>
								b.type === "text" && b.text !== undefined,
						)
						.map((b) => b.text)
						.join("\n"),
					is_error: tr.is_error,
				});
				i++;
			}
			result.push({ role: "user", content: trs });
		} else {
			i++;
		}
	}
	return result;
}

function convertTools(
	tools: Array<{
		name: string;
		label: string;
		description: string;
		parameters: unknown;
	}>,
): unknown[] {
	return tools.map((t) => ({
		name: t.name,
		description: `${t.label}: ${t.description}`,
		input_schema: t.parameters,
	}));
}

// --- Fetch call ---

export async function callLlm(
	systemPrompt: string,
	messages: AgentMessageShape[],
	tools: Array<{
		name: string;
		label: string;
		description: string;
		parameters: unknown;
	}>,
	signal?: AbortSignal,
): Promise<LlmResponse> {
	const apiKey = getApiKey();
	const baseUrl = getApiBaseUrl();
	const model = getModel();

	if (!apiKey) throw new Error("API key is required");

	const body = {
		model,
		max_tokens: 1024,
		system: systemPrompt,
		messages: convertMessages(messages),
		tools: convertTools(tools),
	};

	const isFireworks = baseUrl.includes("fireworks.ai");
	const url = `${baseUrl.replace(/\/+$/, "")}/v1/messages`;

	const headers: Record<string, string> = {
		"Content-Type": "application/json",
		Authorization: `Bearer ${apiKey}`,
	};
	if (!isFireworks) {
		headers["x-api-key"] = apiKey;
		headers["anthropic-version"] = "2023-06-01";
		headers["anthropic-dangerous-direct-browser-access"] = "true";
	}

	const resp = await fetch(url, {
		method: "POST",
		headers,
		body: JSON.stringify(body),
		signal,
	});

	if (!resp.ok) {
		let errMsg = `HTTP ${resp.status}: ${resp.statusText}`;
		try {
			const j = await resp.json();
			errMsg = j.error?.message || errMsg;
		} catch {
			/* use default */
		}
		throw new Error(errMsg);
	}

	return await resp.json();
}

// --- Streaming chunk builder ---

export async function* buildStreamingChunks(
	data: LlmResponse,
	signal?: AbortSignal,
): AsyncGenerator<LlmChunk> {
	yield {
		kind: "start",
		content: [{ type: "text", text: "" }],
		api: "anthropic",
		provider: "anthropic",
		model: "browser-model",
		stop_reason: data.stop_reason,
		error_message: undefined,
		timestamp: Date.now(),
		usage: {
			input: 0,
			output: 0,
			cache_read: 0,
			cache_write: 0,
			total_tokens: 0,
		},
	};

	for (const block of data.content) {
		if (block.type === "text" && block.text) {
			const words = block.text.split(/(\s+)/);
			for (const word of words) {
				if (word) {
					if (signal?.aborted) return;
					yield { kind: "text_delta", text: word };
					await new Promise((r) => setTimeout(r, 15));
				}
			}
		}
	}
}

// --- Result builder ---

export function buildLlmResult(data: LlmResponse): LlmResult {
	const content = data.content
		.filter((b) => b.type === "text" || b.type === "tool_use")
		.map((b) => {
			if (b.type === "text") return { type: "text", text: b.text };
			return {
				type: "tool_call",
				id: b.id,
				name: b.name,
				arguments: b.input || {},
			};
		});

	if (content.length === 0) {
		content.push({ type: "text", text: "" });
	}

	const stopReason = data.stop_reason === "tool_use" ? "tool_use" : "end_turn";

	return {
		Ok: {
			content,
			api: "anthropic",
			provider: "anthropic",
			model: "browser-model",
			stop_reason: stopReason,
			error_message: undefined,
			timestamp: Date.now(),
			usage: {
				input: 0,
				output: 0,
				cache_read: 0,
				cache_write: 0,
				total_tokens: 0,
			},
		},
	};
}

// --- Provider factory ---

export function createLlmProvider(signal?: AbortSignal): {
	call(context: LlmContext, s?: AbortSignal): Promise<LlmStream>;
} {
	return {
		async call(context, s) {
			const effectiveSignal = s || signal;
			const data = await callLlm(
				context.system_prompt,
				context.messages as AgentMessageShape[],
				context.tools as Parameters<typeof callLlm>[2],
				effectiveSignal,
			);
			return {
				chunks: buildStreamingChunks(data, effectiveSignal),
				result: Promise.resolve(buildLlmResult(data)),
			};
		},
	};
}

export async function smartExtract(
	text: string,
	prompt: string,
	signal?: AbortSignal,
): Promise<string> {
	const data = await callLlm(
		prompt,
		[{ role: "user", content: [{ type: "text", text }] }],
		[],
		signal,
	);
	return data.content
		.filter((b): b is typeof b & { text: string } => b.type === "text" && !!b.text)
		.map((b) => b.text)
		.join("\n");
}
