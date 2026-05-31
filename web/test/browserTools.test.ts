import assert from "node:assert";
import { describe, it } from "node:test";
import type { BrowserRuntime } from "../src/browser/browserRuntime.ts";
import { executeBrowserTool } from "../src/browser/browserTools.ts";

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

describe("executeBrowserTool", () => {
	it("get_page returns page state", () => {
		const runtime = mockRuntime();
		const result = executeBrowserTool(
			{ name: "browser_get_page", arguments: {}, id: "1" },
			runtime,
		);
		assert.ok("content" in result);
		const first = result.content[0] as { type: "text"; text: string };
		assert.ok(first.text.includes("http://localhost/"));
	});

	it("eval_js returns dynamic strategy in details", () => {
		const runtime = mockRuntime();
		const result = executeBrowserTool(
			{ name: "browser_eval_js", arguments: { source: "1+1" }, id: "1" },
			runtime,
		);
		assert.ok("content" in result);
		assert.equal(
			(result.details?.strategy as { type?: string })?.type,
			"dynamic",
		);
	});

	it("eval_js with invalid source throws", () => {
		const runtime = mockRuntime({
			evalJs: () => {
				throw new Error("Syntax error");
			},
		});
		assert.throws(() => {
			executeBrowserTool(
				{ name: "browser_eval_js", arguments: { source: "bad" }, id: "1" },
				runtime,
			);
		}, /Syntax error/);
	});

	it("query_selector with invalid selector throws", () => {
		const runtime = mockRuntime({
			querySelector: () => {
				throw new Error("invalid selector");
			},
		});
		assert.throws(() => {
			executeBrowserTool(
				{
					name: "browser_query_selector",
					arguments: { selector: "bad[" },
					id: "1",
				},
				runtime,
			);
		}, /invalid selector/);
	});

	it("click missing element throws", () => {
		const runtime = mockRuntime({
			click: () => ({
				ok: false,
				error: { code: "element_not_found", message: "not found" },
			}),
		});
		assert.throws(() => {
			executeBrowserTool(
				{ name: "browser_click", arguments: { selector: "#missing" }, id: "1" },
				runtime,
			);
		}, /not found/);
	});

	it("type missing element throws", () => {
		const runtime = mockRuntime({
			type: () => ({
				ok: false,
				error: { code: "element_not_found", message: "not found" },
			}),
		});
		assert.throws(() => {
			executeBrowserTool(
				{
					name: "browser_type",
					arguments: { selector: "#missing", text: "hi" },
					id: "1",
				},
				runtime,
			);
		}, /not found/);
	});

	it("console returns empty array when no logs", () => {
		const runtime = mockRuntime();
		const result = executeBrowserTool(
			{ name: "browser_console", arguments: {}, id: "1" },
			runtime,
		);
		assert.ok("content" in result);
		const first = result.content[0] as { type: "text"; text: string };
		assert.ok(first.text.includes('"count": 0'));
	});

	it("unknown tool throws", () => {
		const runtime = mockRuntime();
		assert.throws(() => {
			executeBrowserTool(
				{ name: "browser_unknown", arguments: {}, id: "1" },
				runtime,
			);
		}, /no browser tool handler/);
	});
});
