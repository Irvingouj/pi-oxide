/**
 * Compaction executor — generates structured summaries of old conversation turns.
 *
 * Called by the host when the Rust projection engine signals `needs_compaction`.
 * Uses an LLM call to produce a summary, then rewrites the transcript.
 */

import type { AgentMessageShape } from "../providers/types.ts";

// --- Compaction prompt ---

const COMPACTION_PROMPT = `Summarize the following conversation turns concisely. Structure your summary as XML:

<goal>What the user asked for</goal>
<progress>What was done, what worked, what didn't</progress>
<decisions>Key decisions made and why</decisions>
<files>Files read or modified (list paths)</files>
<next>What was about to happen next</next>

Keep it brief. Focus on information needed to continue the task.`;

// --- File operation extraction ---

function extractFileOps(messages: AgentMessageShape[]): string {
	const files = new Map<string, Set<string>>();
	for (const msg of messages) {
		if (msg.role !== "assistant") continue;
		for (const block of msg.content) {
			if (block.type === "tool_call" && block.arguments) {
				const args = block.arguments as Record<string, unknown>;
				const path = (args.path || args.file_path || args.filePath) as
					| string
					| undefined;
				if (path) {
					const ops = files.get(path) || new Set<string>();
					ops.add(block.name);
					files.set(path, ops);
				}
			}
		}
	}
	if (files.size === 0) return "";
	return (
		"<files>\n" +
		Array.from(files.entries())
			.map(
				([path, ops]) => `  <file path="${path}" ops="${[...ops].join(",")}"/>`,
			)
			.join("\n") +
		"\n</files>"
	);
}

// --- Message serialization for compaction ---

function serializeForCompaction(messages: AgentMessageShape[]): string {
	return messages
		.map((msg) => {
			if (msg.role === "user") {
				const text = msg.content
					.filter(
						(b): b is typeof b & { text: string } =>
							b.type === "text" && !!b.text,
					)
					.map((b) => b.text)
					.join("\n");
				return `[User] ${text}`;
			}
			if (msg.role === "assistant") {
				const parts = msg.content.map((b) => {
					if (b.type === "text" && b.text) return b.text;
					if (b.type === "tool_call" && b.name)
						return `[tool: ${b.name}(${JSON.stringify(b.arguments || {}).slice(0, 200)})]`;
					return "";
				});
				return `[Assistant] ${parts.join(" ")}`;
			}
			if (msg.role === "tool_result") {
				const text = msg.content
					.filter(
						(b): b is typeof b & { text: string } =>
							b.type === "text" && !!b.text,
					)
					.map((b) => b.text)
					.join("\n")
					.slice(0, 500);
				return `[Tool Result] ${text}`;
			}
			return "";
		})
		.filter(Boolean)
		.join("\n\n");
}

// --- Compaction API ---

export interface CompactionRequest {
	messages: AgentMessageShape[];
	needsCompaction: boolean;
	droppedMessages: number;
	/** LLM call function (injected to avoid circular deps) */
	callLlm: (systemPrompt: string, userText: string) => Promise<string>;
}

export interface CompactionResult {
	messages: AgentMessageShape[];
	compacted: boolean;
}

/**
 * If compaction is needed, summarize old turns and rewrite the transcript.
 * Otherwise return messages unchanged.
 */
export async function compactIfNeeded(
	req: CompactionRequest,
): Promise<CompactionResult> {
	if (!req.needsCompaction || req.messages.length === 0) {
		return { messages: req.messages, compacted: false };
	}

	// Determine how many messages to compact (all except the last 2 turns)
	const dropCount =
		req.droppedMessages > 0
			? req.droppedMessages
			: estimateCompactCount(req.messages);

	if (dropCount <= 0) {
		return { messages: req.messages, compacted: false };
	}

	const toCompact = req.messages.slice(0, dropCount);
	const toKeep = req.messages.slice(dropCount);

	const serialized = serializeForCompaction(toCompact);
	const fileOps = extractFileOps(toCompact);

	const userText = `${COMPACTION_PROMPT}\n\n${fileOps ? `${fileOps}\n\n` : ""}${serialized}`;

	try {
		const summary = await req.callLlm(
			"You are a concise conversation summarizer.",
			userText,
		);

		// Prepend summary as a user message
		const summaryMsg: AgentMessageShape = {
			role: "user",
			content: [
				{
					type: "text",
					text: `<context-compaction>\n${summary}\n</context-compaction>`,
				},
			],
		};

		return {
			messages: [summaryMsg, ...toKeep],
			compacted: true,
		};
	} catch {
		// Compaction failed — return original messages, host will use hard trim
		return { messages: req.messages, compacted: false };
	}
}

/**
 * Estimate how many messages to compact: drop all but the last 2 turns.
 */
function estimateCompactCount(messages: AgentMessageShape[]): number {
	let turnCount = 0;
	let lastTurnStart = 0;
	for (let i = 0; i < messages.length; i++) {
		if (messages[i].role === "user" && i > 0) {
			turnCount++;
			if (turnCount >= 2) lastTurnStart = i;
		}
	}
	return lastTurnStart;
}
