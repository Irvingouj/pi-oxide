// Public tool API for the pi-oxide SDK.
// Tools are easy to declare and strongly typed at the boundary.

import type { AgentToolDefinition, AgentTools } from "./types.ts";

/** Structural interface for Zod-compatible schemas without exporting Zod types. */
export interface ToolInputSchema<Input> {
	parse(data: unknown): Input;
	safeParse(data: unknown):
		| { success: true; data: Input }
		| { success: false; error: unknown };
	_def: {
		typeName?: string;
	};
}

export interface ToolConfig<Input, Output> {
	description: string;
	input: ToolInputSchema<Input>;
	run: (input: Input) => Promise<Output> | Output;
	details?: (output: Output) => Record<string, unknown>;
}

/**
 * Declare a single tool definition with a Zod schema for input validation.
 * The `name` is filled in by `defineTools()`.
 */
export function tool<Input, Output>(config: ToolConfig<Input, Output>): AgentToolDefinition {
	return {
		name: "",
		description: config.description,
		inputSchema: config.input,
		run: config.run as (input: unknown) => Promise<unknown> | unknown,
		details: config.details ? (config.details as (output: unknown) => Record<string, unknown>) : undefined,
	};
}

/**
 * Build a mergeable AgentTools pack from a record of tool definitions.
 * Multiple packs can be composed as an array in AgentConfig.tools.
 */
export function defineTools(tools: Record<string, AgentToolDefinition>): AgentTools {
	const entries = Object.entries(tools).map(([name, def]) => ({
		...def,
		name,
	}));

	return {
		definitions: entries,
		getHandler(name: string) {
			const def = entries.find((d) => d.name === name);
			return def?.run ?? null;
		},
	};
}
