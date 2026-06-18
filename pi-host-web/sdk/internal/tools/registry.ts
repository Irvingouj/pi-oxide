// ToolRegistryBuilder — converts SDK AgentTools[] into WASM ToolMap and LLM ToolDefinition[].
// Uses zod-to-json-schema for schema conversion.
// Detects duplicate tool names and throws AgentError.
// Preserves details field in ToolResult.

import type { z } from "zod";
import { zodToJsonSchema } from "zod-to-json-schema";
import type {
	ToolCall,
	ToolDefinition,
	ToolResult,
} from "../../../pi_host_web.js";
import { createAgentError } from "../../errors.ts";
import { HostError } from "../../init.ts";
import { isRecord } from "../../internal/util/types.ts";
import type { AgentTools } from "../../types.ts";
import type { ArtifactStore } from "../engine.ts";
import { createArtifactToolRegistry } from "./service.ts";

export type ToolMap = Record<
	string,
	(call: ToolCall) => Promise<ToolResult> | ToolResult
>;

export class ToolRegistryBuilder {
	/**
	 * Build a WASM ToolMap from AgentTools packs.
	 * Artifact tools are wired with the store at build time.
	 */
	build(
		tools: AgentTools[],
		artifactStore?: ArtifactStore,
		sessionId?: string,
	): ToolMap {
		const toolMap: ToolMap = {};
		const seenNames = new Set<string>();

		for (const pack of tools) {
			for (const def of pack.definitions) {
				if (seenNames.has(def.name)) {
					throw createAgentError(
						"tool_duplicate",
						`Duplicate tool name: ${def.name}`,
						{ recoverable: false },
					);
				}
				seenNames.add(def.name);

				const handler = pack.getHandler(def.name);
				if (handler) {
					toolMap[def.name] = async (call: ToolCall) => {
						let parsedInput: unknown;
						if (def.inputSchema && isZodSchema(def.inputSchema)) {
							const schema = def.inputSchema as z.ZodTypeAny;
							const parseResult = schema.safeParse(call.arguments);
							if (!parseResult.success) {
								throw new HostError(
									"tool_input_invalid",
									`Invalid input: ${parseResult.error.message}`,
								);
							}
							parsedInput = parseResult.data;
						} else {
							parsedInput = call.arguments;
						}

						const output = await handler(parsedInput);

						// If output is already a ToolResult, preserve it (including details)
						if (isToolResult(output)) {
							return output;
						}

						// Otherwise wrap the output
						const text =
							typeof output === "string"
								? output
								: JSON.stringify(output, null, 2);
						const result: ToolResult = {
							content: [{ type: "text", text }],
						};

						// Preserve details if the definition provides a details function
						if (def.details) {
							result.details = def.details(output);
						}

						return result;
					};
				}
			}
		}

		// Wire artifact tools with store if any artifact pack was provided
		const hasArtifactTools = tools.some((p) =>
			p.definitions.some(
				(d) => d.name === "artifact_read" || d.name === "artifact_search",
			),
		);
		if (hasArtifactTools) {
			const artifactRegistry = createArtifactToolRegistry(
				() => 0,
				artifactStore,
				() => sessionId,
			);
			for (const [name, handler] of Object.entries(artifactRegistry)) {
				if (seenNames.has(name)) {
					throw createAgentError(
						"tool_duplicate",
						`Duplicate tool name: ${name}`,
						{ recoverable: false },
					);
				}
				toolMap[name] = handler;
			}
		}

		return toolMap;
	}

	/**
	 * Convert AgentToolDefinition[] to WASM ToolDefinition[] for the LLM.
	 * Uses zod-to-json-schema for Zod schemas; passes plain objects through.
	 */
	getLlmTools(tools: AgentTools[]): ToolDefinition[] {
		const llmTools: ToolDefinition[] = [];
		const seenNames = new Set<string>();

		for (const pack of tools) {
			for (const def of pack.definitions) {
				if (seenNames.has(def.name)) {
					throw createAgentError(
						"tool_duplicate",
						`Duplicate tool name: ${def.name}`,
						{ recoverable: false },
					);
				}
				seenNames.add(def.name);

				let parameters: Record<string, unknown>;
				if (isZodSchema(def.inputSchema)) {
					const schemaResult = zodToJsonSchema(
						def.inputSchema as z.ZodTypeAny,
						{ name: def.name },
					);
					parameters = isRecord(schemaResult)
						? schemaResult
						: { type: "object", properties: {} };
				} else if (isRecord(def.inputSchema)) {
					parameters = def.inputSchema;
				} else {
					parameters = { type: "object", properties: {} };
				}

				llmTools.push({
					name: def.name,
					label: def.name,
					description: def.description,
					parameters,
					execution_mode: "parallel",
				});
			}
		}

		return llmTools;
	}
}

function isToolResult(value: unknown): value is ToolResult {
	return (
		typeof value === "object" &&
		value !== null &&
		"content" in value &&
		Array.isArray((value as ToolResult).content)
	);
}

function isZodSchema(value: unknown): value is z.ZodTypeAny {
	return (
		typeof value === "object" &&
		value !== null &&
		"_def" in value &&
		typeof (value as { _def: unknown })._def === "object" &&
		!!(value as { _def: { typeName?: string } })._def?.typeName?.startsWith(
			"Zod",
		)
	);
}
