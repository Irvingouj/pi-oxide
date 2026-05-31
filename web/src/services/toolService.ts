/**
 * Tool service — creates a tool registry for the agent run config.
 *
 * Pure JS, no React. Wraps browser tool execution into the SDK ToolMap shape.
 */

import {
	hostReadArtifact,
	hostSearchArtifacts,
	type ToolCall,
	type ToolDefinition,
} from "@pi-oxide/pi-host-web";
import type { BrowserRuntime } from "../browser/browserRuntime.ts";
import type { ArtifactStore } from "./agentService.ts";

// Local type — not exported by the SDK
export type ToolMap = Record<string, (call: ToolCall) => Promise<import("@pi-oxide/pi-host-web").ToolResult> | import("@pi-oxide/pi-host-web").ToolResult>;
import {
	BROWSER_TOOLS,
	executeBrowserTool,
	wrapToolHandler,
} from "../browser/browserTools.ts";

// ========================================================================
// Artifact tool schemas
// ========================================================================

const artifactReadSchema: object = {
	type: "object",
	properties: {
		artifact_id: {
			type: "string",
			description: "The artifact id to retrieve (e.g. tool-result-abc123).",
		},
	},
	required: ["artifact_id"],
	additionalProperties: false,
};

const MAX_SEARCH_RESULTS = 50;

const artifactSearchSchema: object = {
	type: "object",
	properties: {
		pattern: {
			type: "string",
			description: "Text pattern to search for inside stored artifacts.",
		},
	},
	required: ["pattern"],
	additionalProperties: false,
};

// ========================================================================
// Artifact tool definitions
// ========================================================================

const ARTIFACT_READ: ToolDefinition = {
	name: "artifact_read",
	label: "Read Artifact",
	description:
		"Read the full original text of a previously stored artifact by its id. " +
		"Use this when a projected tool result references an artifact you need to inspect.",
	parameters: artifactReadSchema,
	execution_mode: "parallel",
};

const ARTIFACT_SEARCH: ToolDefinition = {
	name: "artifact_search",
	label: "Search Artifacts",
	description:
		"Search all stored artifacts for a text pattern. Returns up to 50 matching artifact ids, a short snippet around the first match, and the match count. Use artifact_read to retrieve the full text.",
	parameters: artifactSearchSchema,
	execution_mode: "parallel",
};

/** All artifact tools exposed by the host. */
export const ARTIFACT_TOOLS: ToolDefinition[] = [
	ARTIFACT_READ,
	ARTIFACT_SEARCH,
];

// ========================================================================
// Tool registry
// ========================================================================

export function createToolRegistry(runtime: BrowserRuntime): ToolMap {
	return Object.fromEntries(
		BROWSER_TOOLS.map((t) => [
			t.name,
			wrapToolHandler((call: ToolCall) => executeBrowserTool(call, runtime)),
		]),
	);
}

/**
 * Create artifact tool handlers that read from the host agent artifact store.
 */
export function createArtifactToolRegistry(
	getHandle: () => number,
	artifactStore?: ArtifactStore,
	getSessionId?: () => string | undefined,
): ToolMap {
	return {
		artifact_read: wrapToolHandler(async (call: ToolCall) => {
			const artifactId = call.arguments.artifact_id as string;
			if (typeof artifactId !== "string" || artifactId.length === 0) {
				throw new Error("artifact_id must be a non-empty string");
			}
			let text: string;
			if (artifactStore && getSessionId) {
				const sessionId = getSessionId();
				if (sessionId) {
					const stored = await artifactStore.load(sessionId, artifactId);
					text = stored ?? "";
				} else {
					text = "";
				}
			} else {
				text = await Promise.resolve(hostReadArtifact(getHandle(), artifactId));
			}
			if (text === "") {
				throw new Error(`artifact not found: ${artifactId}`);
			}
			return {
				content: [{ type: "text", text }],
			};
		}),
		artifact_search: wrapToolHandler(async (call: ToolCall) => {
			const pattern = call.arguments.pattern as string;
			if (typeof pattern !== "string" || pattern.length === 0) {
				throw new Error("pattern must be a non-empty string");
			}
			let results: Array<{ id: string; snippet: string; match_count: number }>;
			if (artifactStore && getSessionId) {
				const sessionId = getSessionId();
				if (sessionId) {
					results = await artifactStore.search(sessionId, pattern);
				} else {
					results = [];
				}
			} else {
				const result = await Promise.resolve(
					hostSearchArtifacts(getHandle(), pattern),
				);
				results = result.results;
			}
			const capped = results.slice(0, MAX_SEARCH_RESULTS);
			const text = JSON.stringify(
				capped.map((r) => ({
					id: r.id,
					snippet: r.snippet,
					match_count: r.match_count,
				})),
				null,
				2,
			);
			return {
				content: [{ type: "text", text }],
			};
		}),
	};
}

export { BROWSER_TOOLS };
