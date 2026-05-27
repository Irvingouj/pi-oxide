/**
 * Fake LLM provider for testing the host loop.
 *
 * Produces deterministic responses with streaming chunks:
 * - A Start chunk to begin the assistant message
 * - One or more TextDelta chunks for text content
 * - Then the final LlmResult via onLlmDone
 */

import type { ToolCall } from "./wasmBinding.ts";

export interface FakeLlmResponse {
	/** Assistant text content. If omitted and no toolCalls, produces an empty text response. */
	text?: string;
	/** Tool calls to include in the response. */
	toolCalls?: ToolCall[];
	/** If set, the LLM returns an error (no streaming chunks). */
	error?: { code: string; message: string };
}

function emptyAssistant(): object {
	return {
		content: [{ type: "text", text: "" }],
		api: "test",
		provider: "test",
		model: "test-model",
		stop_reason: "end_turn",
		error_message: null,
		timestamp: 0,
		usage: {
			input: 0,
			output: 0,
			cache_read: 0,
			cache_write: 0,
			total_tokens: 0,
		},
	};
}

/** A sequence of fake responses. The host loop pops one per `stream_llm` action. */
export class FakeLlm {
	private queue: FakeLlmResponse[];
	public readonly log: string[] = [];

	constructor(responses: FakeLlmResponse[]) {
		this.queue = [...responses];
	}

	/** Pop the next response from the queue. */
	next(): FakeLlmResponse {
		const resp = this.queue.shift();
		if (!resp) {
			throw new Error("FakeLlm: no more responses queued");
		}
		this.log.push(`llm_response: ${JSON.stringify(resp)}`);
		return resp;
	}

	/**
	 * Generate streaming chunks for a fake response.
	 *
	 * For text responses: emits a Start chunk, then one or more TextDelta chunks.
	 * For tool-call responses: emits a Start chunk, then the final result carries tool calls.
	 * For errors: returns no chunks (onLlmDone handles it directly).
	 *
	 * Each chunk is a plain object ready for JSON.stringify — matching the Rust
	 * LlmChunk serde shape: { kind: "start", ...flattened fields }
	 * or { kind: "text_delta", text: "..." }
	 */
	buildChunks(resp: FakeLlmResponse): object[] {
		if (resp.error) {
			return [];
		}

		const chunks: object[] = [];

		// Start chunk — begins the streaming assistant message.
		// Rust expects: { kind: "start", content: [...], api: ..., provider: ..., ... }
		// with #[serde(flatten)] on the partial field.
		const startPartial = emptyAssistant();
		chunks.push({ kind: "start", ...startPartial });

		// TextDelta chunks — split text into pieces to exercise streaming
		if (resp.text) {
			// Split into roughly 10-char chunks, or one chunk if short
			const pieceSize = Math.max(10, Math.ceil(resp.text.length / 3));
			for (let i = 0; i < resp.text.length; i += pieceSize) {
				chunks.push({
					kind: "text_delta",
					text: resp.text.slice(i, i + pieceSize),
				});
			}
		}

		return chunks;
	}

	/** Build an LlmResult JSON from a FakeLlmResponse (passed to onLlmDone). */
	buildLlmResult(resp: FakeLlmResponse): object {
		if (resp.error) {
			return {
				Err: {
					error: { code: resp.error.code, message: resp.error.message },
					aborted: false,
				},
			};
		}

		const content: object[] = [];
		if (resp.text !== undefined) {
			content.push({ type: "text", text: resp.text });
		}
		if (resp.toolCalls) {
			for (const tc of resp.toolCalls) {
				content.push({
					type: "tool_call",
					id: tc.id,
					name: tc.name,
					arguments: tc.arguments,
				});
			}
		}

		return {
			Ok: {
				content,
				api: "test",
				provider: "test",
				model: "test-model",
				stop_reason: resp.toolCalls?.length ? "tool_use" : "end_turn",
				timestamp: Date.now(),
				usage: {
					input: 10,
					output: 20,
					cache_read: 0,
					cache_write: 0,
					total_tokens: 30,
				},
			},
		};
	}
}
