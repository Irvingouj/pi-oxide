import type { Content, ContextProjectionBudget, Model, AgentMessage as WasmAgentMessage } from "../../pi_host_web.js";
import type { ArtifactStore } from "../bindings/types.ts";
import { createAgentError } from "../errors.ts";
import type {
	AgentArtifact,
	AgentConfig,
	AgentInput,
	AgentMessage,
	AgentModel,
	AgentTools,
	ArtifactSearchResult,
} from "../types.ts";

export function normalizeTools(tools: AgentTools | AgentTools[] | undefined): AgentTools[] {
	if (!tools) return [];
	if (Array.isArray(tools)) return tools;
	return [tools];
}

export function buildContextBudget(context?: import("../types.ts").AgentContextPolicy): ContextProjectionBudget {
	return {
		max_tool_result_chars: context?.toolResultLimit ?? 50000,
		max_context_tokens: context?.maxTokens ?? 100000,
		microcompact_after_turns: 5,
		compaction_threshold: 0.95,
	};
}

export function buildModelOptions(model: AgentModel): Model {
	return {
		id: model.id ?? "custom-model",
		name: model.id ?? "custom-model",
		api: "anthropic",
		provider: "anthropic",
		reasoning: false,
		context_window: model.contextWindow ?? 100000,
		max_tokens: model.maxTokens ?? 4096,
		capabilities: {
			vision: model.capabilities?.vision ?? false,
			json_mode: model.capabilities?.jsonMode ?? true,
			function_calling: model.capabilities?.functionCalling ?? true,
			streaming: model.capabilities?.streaming ?? true,
		},
		cost: { input: 0, output: 0, cache_read: 0, cache_write: 0 },
	};
}

export function buildArtifactStore(config: AgentConfig): ArtifactStore | undefined {
	if (config.artifacts?.mode === "external" && config.store) {
		const store = config.store;
		if (typeof store.saveArtifact !== "function" || typeof store.loadArtifact !== "function") {
			throw createAgentError(
				"store_artifact_unsupported",
				"Store does not support artifact operations but external artifact mode is configured",
				{ recoverable: false },
			);
		}
		const saveArtifact = store.saveArtifact;
		const loadArtifact = store.loadArtifact;
		return {
			save: (sessionId: string, artifactId: string, content: string) =>
				saveArtifact(sessionId, {
					id: artifactId,
					kind: "text",
					content,
					createdAt: Date.now(),
				}),
			load: (sessionId: string, artifactId: string) =>
				loadArtifact(sessionId, artifactId).then((a: AgentArtifact | null) =>
					a && typeof a.content === "string" ? a.content : null,
				),
			search: (sessionId: string, query: string) => {
				if (typeof store.searchArtifacts !== "function") {
					return Promise.resolve([]);
				}
				return store.searchArtifacts?.(sessionId, { text: query }).then((results: ArtifactSearchResult[]) =>
					results.map((r: ArtifactSearchResult) => ({
						id: r.artifact.id,
						snippet: r.snippet ?? "",
						match_count: r.matchCount ?? 0,
					})),
				);
			},
		};
	}
	return undefined;
}

export function mergeMetadata(
	input: string | AgentInput,
	runMetadata?: Record<string, unknown>,
): Record<string, unknown> | undefined {
	const inputMetadata = typeof input === "object" ? input.metadata : undefined;
	if (!inputMetadata && !runMetadata) return undefined;
	return { ...inputMetadata, ...runMetadata };
}

export function buildUserMessage(input: string | AgentInput): WasmAgentMessage {
	const text = typeof input === "string" ? input : input.text;
	const content: Content[] = [{ type: "text", text }];

	if (typeof input === "object" && input.attachments) {
		for (const attachment of input.attachments) {
			if (attachment.type === "image" || attachment.mimeType?.startsWith("image/")) {
				content.push({
					type: "image",
					media_type: attachment.mimeType ?? "image/png",
					data:
						typeof attachment.content === "string"
							? attachment.content
							: btoa(String.fromCharCode(...new Uint8Array(attachment.content))),
				});
			}
		}
	}

	return {
		role: "user",
		content,
		timestamp: Date.now(),
	};
}

export function convertWasmMessagesToAgentMessages(messages: WasmAgentMessage[]): AgentMessage[] {
	return messages.map((msg) => ({
		id: stableMessageId(msg),
		role: msg.role,
		content: msg.content.map((c) => {
			if (c.type === "text") return { type: "text" as const, text: c.text };
			if (c.type === "tool_call")
				return {
					type: "tool_call" as const,
					id: c.id,
					name: c.name,
					arguments: c.arguments,
				};
			if (c.type === "image") return { type: "image" as const, mimeType: c.media_type, data: c.data };
			return { type: "text" as const, text: "" };
		}),
		timestamp: Date.now(),
		tool_call_id: msg.role === "tool_result" ? (msg as unknown as { tool_call_id: string }).tool_call_id : undefined,
	}));
}

function stableMessageId(msg: WasmAgentMessage): string {
	const contentHash = msg.content
		.map((c) => {
			if (c.type === "text") return `t:${c.text?.slice(0, 64) ?? ""}`;
			if (c.type === "tool_call") return `tc:${c.id ?? ""}:${c.name ?? ""}`;
			if (c.type === "image") return `img:${c.media_type ?? ""}`;
			return (c as { type: string }).type;
		})
		.join("|");
	return `msg-${msg.role}-${msg.timestamp ?? 0}-${contentHash}`;
}

export { stableMessageId };
