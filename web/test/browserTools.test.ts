import assert from "node:assert";
import { describe, it } from "node:test";
import type {
	BrowserConsoleEntry,
	BrowserElementSnapshot,
	BrowserPageSnapshot,
	BrowserRuntime,
	BrowserToolResult as RuntimeToolResult,
} from "../src/browser/browserRuntime.ts";
import {
	BROWSER_TOOLS,
	type BrowserToolResult,
	executeBrowserTool,
} from "../src/browser/browserTools.ts";

function mockRuntime(overrides: Partial<BrowserRuntime> = {}): BrowserRuntime {
	return {
		getPage: () => ({
			url: "http://localhost/",
			title: "Test",
			readyState: "complete",
			focusedElement: null,
		}),
		evalJs: () => "ok",
		querySelector: () => null,
		querySelectorAll: () => [],
		click: () => ({ ok: true }),
		type: () => ({ ok: true }),
		getConsole: () => [],
		...overrides,
	} as BrowserRuntime;
}

function isError(
	result: BrowserToolResult,
): result is { error: { code: string; message: string } } {
	return "error" in result;
}

function isContent(
	result: BrowserToolResult,
): result is { content: Array<{ type: "text"; text: string }> } {
	return "content" in result;
}

describe("executeBrowserTool", () => {
	it("get_page returns page state", () => {
		const runtime = mockRuntime();
		const result = executeBrowserTool(
			{ name: "browser_get_page", arguments: {}, id: "1" },
			runtime,
		);
		assert.ok(isContent(result));
		assert.ok(result.content[0].text.includes("http://localhost/"));
	});

	it("eval_js with invalid source returns error", () => {
		const runtime = mockRuntime({
			evalJs: () => {
				throw new Error("Syntax error");
			},
		});
		const result = executeBrowserTool(
			{ name: "browser_eval_js", arguments: { source: "bad" }, id: "1" },
			runtime,
		);
		assert.ok(isError(result));
		assert.equal(result.error.code, "eval_error");
	});

	it("query_selector with invalid selector returns error", () => {
		const runtime = mockRuntime({
			querySelector: () => {
				throw new Error("invalid selector");
			},
		});
		const result = executeBrowserTool(
			{
				name: "browser_query_selector",
				arguments: { selector: "bad[" },
				id: "1",
			},
			runtime,
		);
		assert.ok(isError(result));
		assert.equal(result.error.code, "selector_error");
	});

	it("click missing element returns error", () => {
		const runtime = mockRuntime({
			click: () => ({
				ok: false,
				error: { code: "element_not_found", message: "not found" },
			}),
		});
		const result = executeBrowserTool(
			{ name: "browser_click", arguments: { selector: "#missing" }, id: "1" },
			runtime,
		);
		assert.ok(isError(result));
		assert.equal(result.error.code, "click_error");
	});

	it("type missing element returns error", () => {
		const runtime = mockRuntime({
			type: () => ({
				ok: false,
				error: { code: "element_not_found", message: "not found" },
			}),
		});
		const result = executeBrowserTool(
			{
				name: "browser_type",
				arguments: { selector: "#missing", text: "hi" },
				id: "1",
			},
			runtime,
		);
		assert.ok(isError(result));
		assert.equal(result.error.code, "type_error");
	});

	it("console returns empty array when no logs", () => {
		const runtime = mockRuntime();
		const result = executeBrowserTool(
			{ name: "browser_console", arguments: {}, id: "1" },
			runtime,
		);
		assert.ok(isContent(result));
		assert.ok(result.content[0].text.includes('"count": 0'));
	});

	it("unknown tool returns error", () => {
		const runtime = mockRuntime();
		const result = executeBrowserTool(
			{ name: "browser_unknown", arguments: {}, id: "1" },
			runtime,
		);
		assert.ok(isError(result));
		assert.equal(result.error.code, "unknown_tool");
	});
});
