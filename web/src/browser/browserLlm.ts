/**
 * Browser LLM provider — calls Anthropic/Fireworks API from the browser.
 *
 * Handles message format conversion and dual auth headers
 * (Fireworks Bearer vs Anthropic x-api-key).
 */

import { getApiBaseUrl, getApiKey, getModel } from "./config.ts";

// --- Message conversion ---

interface AgentMessage {
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

function convertMessages(messages: AgentMessage[]): unknown[] {
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
								b.type === "text" && !!b.text,
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

// --- LLM call ---

export interface LlmResponse {
	content: Array<{
		type: string;
		text?: string;
		id?: string;
		name?: string;
		input?: unknown;
	}>;
	stop_reason: string;
}

export async function callLlm(
	systemPrompt: string,
	messages: AgentMessage[],
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
