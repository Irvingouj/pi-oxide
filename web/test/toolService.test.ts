import assert from "node:assert";
import { describe, it } from "node:test";
import type { BrowserRuntime } from "../src/browser/browserRuntime.ts";
import {
	createProjectionService,
	createTestProjectionService,
} from "../src/services/projectionService.ts";
import {
	ARTIFACT_TOOLS,
	BROWSER_TOOLS,
	createArtifactToolRegistry,
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

		const getPage = registry.browser_get_page;
		const result = await getPage({
			name: "browser_get_page",
			arguments: {},
			id: "1",
		});
		assert.ok("content" in result || "error" in result);
	});

	it("preserves dynamic strategy in details", async () => {
		const runtime = mockRuntime();
		const registry = createToolRegistry(runtime);

		const evalJs = registry.browser_eval_js;
		const result = await evalJs({
			name: "browser_eval_js",
			arguments: { source: "1+1" },
			id: "1",
		});
		assert.ok("content" in result);
		assert.equal(result.details?.strategy?.type, "dynamic");
	});
});

describe("createArtifactToolRegistry", () => {
	it("maps all artifact tools", () => {
		const projectionService = createProjectionService();
		const registry = createArtifactToolRegistry(projectionService);
		for (const tool of ARTIFACT_TOOLS) {
			assert.ok(
				tool.name in registry,
				`expected registry to have handler for ${tool.name}`,
			);
			assert.equal(typeof registry[tool.name], "function");
		}
	});

	it("artifact_read returns full text for a stored artifact", async () => {
		const projectionService = createTestProjectionService();
		projectionService.__seedArtifactForTest(
			"tool-result-tc-1",
			"original full text",
		);
		const registry = createArtifactToolRegistry(projectionService);

		const read = registry.artifact_read;
		const result = await read({
			name: "artifact_read",
			arguments: { artifact_id: "tool-result-tc-1" },
			id: "1",
		});
		assert.ok("content" in result);
		assert.equal(result.content?.[0]?.text, "original full text");
	});

	it("artifact_read returns error for missing artifact", async () => {
		const projectionService = createTestProjectionService();
		const registry = createArtifactToolRegistry(projectionService);

		const read = registry.artifact_read;
		const result = await read({
			name: "artifact_read",
			arguments: { artifact_id: "missing" },
			id: "1",
		});
		assert.ok("error" in result);
		assert.equal(result.error?.code, "not_found");
	});

	it("artifact_search returns matching artifacts", async () => {
		const projectionService = createTestProjectionService();
		projectionService.__seedArtifactForTest("tool-result-tc-1", "hello world");
		const registry = createArtifactToolRegistry(projectionService);

		const search = registry.artifact_search;
		const result = await search({
			name: "artifact_search",
			arguments: { pattern: "world" },
			id: "1",
		});
		assert.ok("content" in result);
		const parsed = JSON.parse(result.content?.[0]?.text ?? "[]");
		assert.equal(parsed.length, 1);
		assert.equal(parsed[0].snippet, "hello world");
		assert.equal(parsed[0].match_count, 1);
	});

	it("artifact_read returns error for null artifact_id", async () => {
		const projectionService = createTestProjectionService();
		const registry = createArtifactToolRegistry(projectionService);
		const read = registry.artifact_read;
		const result = await read({
			name: "artifact_read",
			arguments: { artifact_id: null },
			id: "1",
		});
		assert.ok("error" in result);
		assert.equal(result.error?.code, "invalid_argument");
	});

	it("artifact_read returns error for empty artifact_id", async () => {
		const projectionService = createTestProjectionService();
		const registry = createArtifactToolRegistry(projectionService);
		const read = registry.artifact_read;
		const result = await read({
			name: "artifact_read",
			arguments: { artifact_id: "" },
			id: "1",
		});
		assert.ok("error" in result);
		assert.equal(result.error?.code, "invalid_argument");
	});

	it("artifact_search returns error for empty pattern", async () => {
		const projectionService = createTestProjectionService();
		const registry = createArtifactToolRegistry(projectionService);
		const search = registry.artifact_search;
		const result = await search({
			name: "artifact_search",
			arguments: { pattern: "" },
			id: "1",
		});
		assert.ok("error" in result);
		assert.equal(result.error?.code, "invalid_argument");
	});

	it("artifact_search returns error for null pattern", async () => {
		const projectionService = createTestProjectionService();
		const registry = createArtifactToolRegistry(projectionService);
		const search = registry.artifact_search;
		const result = await search({
			name: "artifact_search",
			arguments: { pattern: null },
			id: "1",
		});
		assert.ok("error" in result);
		assert.equal(result.error?.code, "invalid_argument");
	});
});
