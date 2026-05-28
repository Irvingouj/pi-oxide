import assert from "node:assert";
import { describe, it } from "node:test";
import type { BrowserRuntime } from "../src/browser/browserRuntime.ts";
import {
	BROWSER_TOOLS,
	createToolRegistry,
} from "../src/services/toolService.ts";

function mockRuntime(): BrowserRuntime {
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
	} as BrowserRuntime;
}

describe("createToolRegistry", () => {
	it("maps all browser tools", () => {
		const runtime = mockRuntime();
		const registry = createToolRegistry(runtime);
		for (const tool of BROWSER_TOOLS) {
			assert.ok(
				tool.name in registry,
				`expected registry to have handler for ${tool.name}`,
			);
			assert.equal(typeof registry[tool.name], "function");
		}
	});

	it("returns correct result shapes", async () => {
		const runtime = mockRuntime();
		const registry = createToolRegistry(runtime);

		const getPage = registry["browser_get_page"];
		const result = await getPage({
			name: "browser_get_page",
			arguments: {},
			id: "1",
		});
		assert.ok("content" in result || "error" in result);
	});
});
