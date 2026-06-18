import type {
	LlmChunk,
	LlmContext,
	LlmResult,
	PersistData,
	AgentEvent as RawAgentEvent,
	ToolCall,
	ToolDefinition,
	AgentMessage as WasmAgentMessage,
} from "../../pi_host_web.js";
import type { Logger } from "../types.ts";

export interface ArtifactStore {
	save(sessionId: string, artifactId: string, content: string): Promise<void>;
	load(sessionId: string, artifactId: string): Promise<string | null>;
	search(
		sessionId: string,
		query: string,
	): Promise<Array<{ id: string; snippet: string; match_count: number }>>;
}

export interface AgentRunConfig {
	llm: {
		call(
			context: LlmContext,
			signal?: AbortSignal,
		): Promise<LlmStream> | LlmStream;
		summarize?(
			messages: WasmAgentMessage[],
			signal?: AbortSignal,
		): Promise<string>;
	};
	tools: Record<
		string,
		(
			call: ToolCall,
		) =>
			| Promise<import("../../pi_host_web.js").ToolResult>
			| import("../../pi_host_web.js").ToolResult
	>;
	llmTools?: ToolDefinition[];
	onEvent?: (event: RawAgentEvent) => void;
	onMarkers?: (markers: Array<{ type: string; entry_ids?: string[] }>) => void;
	signal?: AbortSignal;
	onPersist?: (data: PersistData) => Promise<void>;
	artifactStore?: ArtifactStore;
	logger?: Logger;
	prepareToolCalls?: {
		transform?: (
			call: ToolCall,
		) =>
			| { type: "none" }
			| { type: "rewrite_args"; arguments: unknown }
			| Promise<
					{ type: "none" } | { type: "rewrite_args"; arguments: unknown }
			  >;
		permission?: (
			call: ToolCall,
		) =>
			| { type: "allow" }
			| { type: "block"; reason: string }
			| Promise<{ type: "allow" } | { type: "block"; reason: string }>;
	};
}

export interface LlmStream {
	chunks: AsyncIterable<LlmChunk>;
	result: Promise<LlmResult>;
}

export interface TurnResult {
	aborted: boolean;
}
