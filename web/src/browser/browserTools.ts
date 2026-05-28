/**
 * Browser-native tool schemas and tool registry.
 *
 * Defines the six browser tools that the browser host exposes to the agent.
 * Tool execution goes through a BrowserRuntime adapter — fake for tests,
 * real DOM later.
 *
 * Host-owned — no browser APIs in pi-core.
 */

import type { ToolCall, ToolDefinition } from "@pi-oxide/pi-host-web";
import type {
	BrowserConsoleEntry,
	BrowserElementSnapshot,
	BrowserRuntime,
} from "./browserRuntime.ts";

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

// ========================================================================
// Tool execution
// ========================================================================

/** Max text preview length in element snapshots. */
const MAX_ELEMENT_TEXT = 500;

/** Discriminated result returned by browser tool handlers. */
export type BrowserToolResult =
	| { content: Array<{ type: "text"; text: string }> }
	| { error: { code: string; message: string } };

function truncateText(
	text: string,
	max: number,
): { text: string; truncated: boolean } {
	if (text.length <= max) return { text, truncated: false };
	return { text: `${text.slice(0, max)}...`, truncated: true };
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

/** Wrap a tool function in a try-catch that produces a typed error result. */
function tryTool<T>(fn: () => T, errorCode: string): BrowserToolResult {
	try {
		return { content: [{ type: "text", text: JSON.stringify(fn(), null, 2) }] };
	} catch (err: unknown) {
		const message = err instanceof Error ? err.message : String(err);
		return { error: { code: errorCode, message } };
	}
}

/**
 * Execute a browser tool call against a BrowserRuntime.
 *
 * Returns a JSON payload suitable for `onToolDone`.
 */
export function executeBrowserTool(
	call: ToolCall,
	runtime: BrowserRuntime,
): BrowserToolResult {
	switch (call.name) {
		case "browser_get_page": {
			const page = runtime.getPage();
			return {
				content: [
					{
						type: "text",
						text: JSON.stringify(
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
						),
					},
				],
			};
		}

		case "browser_eval_js": {
			const source = call.arguments.source as string;
			if (typeof source !== "string" || source.length === 0) {
				return {
					error: {
						code: "invalid_argument",
						message: "source must be a non-empty string",
					},
				};
			}
			return tryTool(
				() => ({ ok: true, result: runtime.evalJs(source) }),
				"eval_error",
			);
		}

		case "browser_query_selector": {
			const selector = call.arguments.selector as string;
			const all = call.arguments.all as boolean | undefined;
			if (!selector) {
				return {
					error: { code: "invalid_argument", message: "selector is required" },
				};
			}
			return tryTool(() => {
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
			}, "selector_error");
		}

		case "browser_click": {
			const selector = call.arguments.selector as string;
			if (!selector) {
				return {
					error: { code: "invalid_argument", message: "selector is required" },
				};
			}
			return tryTool(() => {
				const result = runtime.click(selector);
				if (!result.ok) {
					throw new Error(result.error.message);
				}
				return { ok: true, action: "click", selector };
			}, "click_error");
		}

		case "browser_type": {
			const selector = call.arguments.selector as string;
			const text = call.arguments.text as string;
			if (!selector) {
				return {
					error: { code: "invalid_argument", message: "selector is required" },
				};
			}
			if (typeof text !== "string") {
				return {
					error: { code: "invalid_argument", message: "text must be a string" },
				};
			}
			return tryTool(() => {
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
			}, "type_error");
		}

		case "browser_console": {
			const level = call.arguments.level as string | undefined;
			const limit = call.arguments.limit as number | undefined;
			const entries = runtime.getConsole();
			const formatted = formatConsoleEntries(entries, level, limit);
			return {
				content: [
					{
						type: "text",
						text: JSON.stringify(formatted, null, 2),
					},
				],
			};
		}

		default:
			return {
				error: {
					code: "unknown_tool",
					message: `no browser tool handler for: ${call.name}`,
				},
			};
	}
}
