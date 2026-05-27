/**
 * Provider-neutral types for the LLM adapter interface.
 *
 * These types bridge the Rust agent core and any specific provider implementation.
 */

import type { ToolDefinition } from "../tools/schemas.ts";

/** Context from the Rust agent for a stream_llm action. */
export interface LlmRequest {
	system_prompt: string;
	messages: AgentMessageShape[];
	tools: ToolDefinition[];
}

/** The shape of a message as it flows through the Rust core. */
export type AgentMessageShape =
	| { role: "user"; content: ContentBlock[]; timestamp: number }
	| {
			role: "assistant";
			content: ContentBlock[];
			api: string;
			provider: string;
			model: string;
			stop_reason: string;
			error_message: string | null;
			timestamp: number;
			usage: TokenUsage;
	  }
	| {
			role: "tool_result";
			tool_call_id: string;
			tool_name: string;
			content: ContentBlock[];
			details: unknown;
			is_error: boolean;
			timestamp: number;
	  };

export interface ContentBlock {
	type: string;
	text?: string;
	id?: string;
	name?: string;
	arguments?: Record<string, unknown>;
	media_type?: string;
	data?: string;
}

export interface TokenUsage {
	input: number;
	output: number;
	cache_read: number;
	cache_write: number;
	total_tokens: number;
}

/** Result from the provider — fed back into Rust via onLlmDone. */
export interface ProviderResult {
	/** The LlmResult JSON to pass to onLlmDone. */
	llmResult: object;
	/** Chunks to feed via feedLlmChunk before calling onLlmDone. */
	chunks: object[];
	/** Log entries from the provider call. */
	log: string[];
}
