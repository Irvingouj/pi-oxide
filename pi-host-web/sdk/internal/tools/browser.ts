/**
 * Browser-native tool schemas and tool registry.
 *
 * Defines the six browser tools that the browser host exposes to the agent.
 * Tool execution goes through a BrowserRuntime adapter — fake for tests,
 * real DOM later.
 *
 * Host-owned — no browser APIs in pi-core.
 */

import type {
	ToolCall,
	ToolDefinition,
	ToolResult,
} from "../../../pi_host_web.js";
import type {
	BrowserConsoleEntry,
	BrowserElementSnapshot,
	BrowserRuntime,
} from "./browserRuntime.ts";
import { LiveBrowserRuntime } from "./liveRuntime.ts";
import type { AgentTools, AgentToolDefinition } from "../../types.ts";

// ========================================================================
// Tool schemas
// ========================================================================

const browserGetPageSchema: object = {
	type: "object",
	properties: {},
	additionalProperties: false,
};

const browserEvalJsSchema: object = {
	type: "object",
	properties: {
		source: {
			type: "string",
			description: "JavaScript source code to evaluate in the page context.",
		},
	},
	required: ["source"],
	additionalProperties: false,
};

const browserQuerySelectorSchema: object = {
	type: "object",
	properties: {
		selector: {
			type: "string",
			description: "CSS selector to match elements.",
		},
		all: {
			type: "boolean",
			description:
				"If true, return all matching elements. Default: false (first match only).",
		},
	},
	required: ["selector"],
	additionalProperties: false,
};

const browserClickSchema: object = {
	type: "object",
	properties: {
		selector: {
			type: "string",
			description: "CSS selector of the element to click.",
		},
	},
	required: ["selector"],
	additionalProperties: false,
};

const browserTypeSchema: object = {
	type: "object",
	properties: {
		selector: {
			type: "string",
			description: "CSS selector of the input element to type into.",
		},
		text: {
			type: "string",
			description: "Text to type into the element.",
		},
	},
	required: ["selector", "text"],
	additionalProperties: false,
};

const browserConsoleSchema: object = {
	type: "object",
	properties: {
		level: {
			type: "string",
			description:
				"Filter by level: 'log', 'warn', 'error', 'info'. Omit for all.",
		},
		limit: {
			type: "number",
			description: "Maximum number of entries to return. Default: 50.",
		},
	},
	additionalProperties: false,
};

// ========================================================================
// Tool definitions
// ========================================================================

const BROWSER_GET_PAGE: ToolDefinition = {
	name: "browser_get_page",
	label: "Get Page",
	description:
		"Get the current page state: URL, title, ready state, and focused element summary.",
	parameters: browserGetPageSchema,
	execution_mode: "parallel",
};

const BROWSER_EVAL_JS: ToolDefinition = {
	name: "browser_eval_js",
	label: "Eval JS",
	description:
		"Evaluate JavaScript in the page context and return the JSON-serializable result. " +
		"Returns a typed error if the code throws.",
	parameters: browserEvalJsSchema,
	execution_mode: "sequential",
};

const BROWSER_QUERY_SELECTOR: ToolDefinition = {
	name: "browser_query_selector",
	label: "Query Selector",
	description:
		"Query elements by CSS selector. Returns tag, text preview, attributes, and visibility " +
		"for each matched element. Use 'all: true' to return all matches.",
	parameters: browserQuerySelectorSchema,
	execution_mode: "parallel",
};

const BROWSER_CLICK: ToolDefinition = {
	name: "browser_click",
	label: "Click",
	description: "Click an element by CSS selector.",
	parameters: browserClickSchema,
	execution_mode: "sequential",
};

const BROWSER_TYPE: ToolDefinition = {
	name: "browser_type",
	label: "Type",
	description: "Type text into an input element by CSS selector.",
	parameters: browserTypeSchema,
	execution_mode: "sequential",
};

const BROWSER_CONSOLE: ToolDefinition = {
	name: "browser_console",
	label: "Console",
	description:
		"Read captured console logs, warnings, and errors from the page. " +
		"Optionally filter by level and limit count.",
	parameters: browserConsoleSchema,
	execution_mode: "parallel",
};

/** All browser-native tools for the browser host. */
export const BROWSER_TOOLS: ToolDefinition[] = [
	BROWSER_GET_PAGE,
	BROWSER_EVAL_JS,
	BROWSER_QUERY_SELECTOR,
	BROWSER_CLICK,
	BROWSER_TYPE,
	BROWSER_CONSOLE,
];

const BROWSER_GET_PAGE_SCRIPT = `#{ action: "project", text: head(text, 3000) }`;
const BROWSER_EVAL_JS_SCRIPT = `#{ action: "project", text: head(text, 5000) }`;
const BROWSER_CONSOLE_SCRIPT = `let all = lines(text);
let errs = [];
for line in all {
  if contains(line, "ERROR") || contains(line, "FATAL") {
    errs.push(line);
  }
}
#{ action: "project", text: join(errs, "\\n") }`;

const DEFAULT_SCRIPTS: Record<string, string> = {
	browser_get_page: BROWSER_GET_PAGE_SCRIPT,
	browser_console: BROWSER_CONSOLE_SCRIPT,
	browser_eval_js: BROWSER_EVAL_JS_SCRIPT,
};

// ========================================================================
// Tool execution
// ========================================================================

/** Max text preview length in element snapshots. */
const MAX_ELEMENT_TEXT = 500;

/** Wrap a handler so that thrown errors are normalized to ToolResult. */
export function wrapToolHandler(
	handler: (call: ToolCall) => ToolResult | Promise<ToolResult>,
): (call: ToolCall) => Promise<ToolResult> {
	return async (call: ToolCall) => {
		try {
			return await handler(call);
		} catch (err: unknown) {
			const message = err instanceof Error ? err.message : String(err);
			return {
				content: [{ type: "text", text: message }],
			};
		}
	};
}

function truncateText(
	text: string,
	max: number,
): { text: string; truncated: boolean } {
	if (text.length <= max) return { text, truncated: false };
	return { text: `${text.slice(0, max)}...`, truncated: true };
}

function makeDetails(
	toolName: string,
	text: string,
	truncatedByTool: boolean = false,
): Record<string, unknown> {
	return {
		content_kind: "generic_text",
		strategy: {
			type: "dynamic",
			script:
				DEFAULT_SCRIPTS[toolName] ||
				`#{ action: "project", text: head(text, 2000) }`,
		},
		original_chars: Array.from(text).length,
		truncated_by_tool: truncatedByTool,
	};
}

function formatElement(el: BrowserElementSnapshot): object {
	const { text, truncated } = truncateText(el.text, MAX_ELEMENT_TEXT);
	return {
		tag: el.tag,
		text,
		textTruncated: truncated,
		attributes: el.attributes,
		visible: el.visible,
		selector: el.selector,
	};
}

function formatConsoleEntries(
	entries: BrowserConsoleEntry[],
	level?: string,
	limit?: number,
): object {
	let filtered = entries;
	if (level) {
		filtered = filtered.filter((e) => e.level === level);
	}
	const effectiveLimit = limit ?? 50;
	const truncated = filtered.length > effectiveLimit;
	const sliced = filtered.slice(-effectiveLimit);
	return {
		entries: sliced.map((e) => ({
			level: e.level,
			args: e.args,
			timestamp: e.timestamp,
		})),
		count: sliced.length,
		totalAvailable: filtered.length,
		truncated,
	};
}

/** Wrap a tool function in a try-catch that throws on error. */
function tryTool<T>(
	fn: () => T,
	toolName: string,
): ToolResult {
	const text = JSON.stringify(fn(), null, 2);
	return {
		content: [{ type: "text", text }],
		details: makeDetails(toolName, text, false),
	};
}

/**
 * Execute a browser tool call against a BrowserRuntime.
 *
 * Returns a ToolResult suitable for hostToolDone.
 */
export function executeBrowserTool(
	call: ToolCall,
	runtime: BrowserRuntime,
): ToolResult {
	switch (call.name) {
		case "browser_get_page": {
			const page = runtime.getPage();
			const text = JSON.stringify(
				{
					url: page.url,
					title: page.title,
					readyState: page.readyState,
					focusedElement: page.focusedElement
						? formatElement(page.focusedElement)
						: null,
				},
				null,
				2,
			);
			return {
				content: [{ type: "text", text }],
				details: makeDetails("browser_get_page", text, false),
			};
		}

		case "browser_eval_js": {
			const source = call.arguments.source as string;
			if (typeof source !== "string" || source.length === 0) {
				throw new Error("source must be a non-empty string");
			}
			return tryTool(
				() => ({ ok: true, result: runtime.evalJs(source) }),
				"browser_eval_js",
			);
		}

		case "browser_query_selector": {
			const selector = call.arguments.selector as string;
			const all = call.arguments.all as boolean | undefined;
			if (!selector) {
				throw new Error("selector is required");
			}
			return tryTool(
				() => {
					if (all) {
						const elements = runtime.querySelectorAll(selector);
						return {
							selector,
							matchCount: elements.length,
							elements: elements.map(formatElement),
						};
					}
					const el = runtime.querySelector(selector);
					return { selector, found: el ? formatElement(el) : null };
				},
				"browser_query_selector",
			);
		}

		case "browser_click": {
			const selector = call.arguments.selector as string;
			if (!selector) {
				throw new Error("selector is required");
			}
			return tryTool(
				() => {
					const result = runtime.click(selector);
					if (!result.ok) {
						throw new Error(result.error.message);
					}
					return { ok: true, action: "click", selector };
				},
				"browser_click",
			);
		}

		case "browser_type": {
			const selector = call.arguments.selector as string;
			const text = call.arguments.text as string;
			if (!selector) {
				throw new Error("selector is required");
			}
			if (typeof text !== "string") {
				throw new Error("text must be a string");
			}
			return tryTool(
				() => {
					const result = runtime.type(selector, text);
					if (!result.ok) {
						throw new Error(result.error.message);
					}
					return {
						ok: true,
						action: "type",
						selector,
						textLength: text.length,
					};
				},
				"browser_type",
			);
		}

		case "browser_console": {
			const level = call.arguments.level as string | undefined;
			const limit = call.arguments.limit as number | undefined;
			const entries = runtime.getConsole();
			const formatted = formatConsoleEntries(entries, level, limit);
			const text = JSON.stringify(formatted, null, 2);
			return {
				content: [{ type: "text", text }],
				details: makeDetails("browser_console", text, false),
			};
		}

		default:
			throw new Error(`no browser tool handler for: ${call.name}`);
	}
}

/**
 * Create an AgentTools pack for browser-native tools.
 * Auto-injects LiveBrowserRuntime in browser environments.
 */
export function browserTools(runtime?: BrowserRuntime): AgentTools {
	const rt = runtime ?? new LiveBrowserRuntime();

	// Build handlers map: each handler returns a ToolResult (preserves details)
	const handlers: Record<string, (call: ToolCall) => ToolResult | Promise<ToolResult>> = {};
	for (const def of BROWSER_TOOLS) {
		handlers[def.name] = (call: ToolCall) => executeBrowserTool(call, rt);
	}

	const definitions: AgentToolDefinition[] = BROWSER_TOOLS.map((t) => ({
		name: t.name,
		description: t.description,
		inputSchema: t.parameters,
		run: (input: unknown) => {
			const handler = handlers[t.name];
			if (!handler) return null;
			return handler({ id: "", name: t.name, arguments: input as Record<string, unknown> });
		},
	}));

	return {
		definitions,
		getHandler(name: string) {
			const handler = handlers[name];
			if (!handler) return null;
			return async (input: unknown) => {
				const result = await handler({ id: "", name, arguments: input as Record<string, unknown> });
				return result;
			};
		},
	};
}
