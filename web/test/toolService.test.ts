import assert from "node:assert";
import { describe, it } from "node:test";
import {
	createHostAgent,
	destroyHostAgent,
	ensureInit,
	hostToolDone,
	startTurn,
	type ToolCall,
	type ToolResult,
} from "@pi-oxide/pi-host-web";

await ensureInit();

import type { BrowserRuntime } from "../src/browser/browserRuntime.ts";
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
	function makeAgentWithArtifacts(): { handle: number; cleanup: () => void } {
		const createResult = createHostAgent(
			{
				system_prompt: "test",
				model: {
					id: "test-model",
					name: "Test",
					api: "test",
					provider: "test",
					reasoning: false,
					context_window: 4096,
					max_tokens: 1024,
				},
				thinking_level: "off",
				steering_mode: "one_at_a_time",
				follow_up_mode: "one_at_a_time",
				tool_execution_mode: "parallel",
			},
			{
				max_tool_result_chars: 50000,
				max_context_tokens: 100000,
				microcompact_after_turns: 5,
				compaction_threshold: 0.75,
			},
		);
		assert.ok(createResult.ok);
		const handle = createResult.data!.handle;

		// Seed an artifact by running a tool turn that stores a result
		const turn = startTurn(handle, {
			prompt: {
				role: "user",
				content: [{ type: "text", text: "hello" }],
				timestamp: 1,
			},
			tools: [],
		});
		assert.ok(turn.ok);

		return {
			handle,
			cleanup: () => destroyHostAgent(handle),
		};
	}

	it("maps all artifact tools", () => {
		const { handle, cleanup } = makeAgentWithArtifacts();
		const registry = createArtifactToolRegistry(() => handle);
		for (const tool of ARTIFACT_TOOLS) {
			assert.ok(
				tool.name in registry,
				`expected registry to have handler for ${tool.name}`,
			);
			assert.equal(typeof registry[tool.name], "function");
		}
		cleanup();
	});

	it("artifact_read returns error for missing artifact", async () => {
		const { handle, cleanup } = makeAgentWithArtifacts();
		const registry = createArtifactToolRegistry(() => handle);

		const read = registry.artifact_read;
		const result = await read({
			name: "artifact_read",
			arguments: { artifact_id: "missing" },
			id: "1",
		});
		assert.ok("content" in result);
		const text = result.content[0].type === "text" ? result.content[0].text : "";
		assert.ok(text.includes("not found"));
		cleanup();
	});

	it("artifact_read returns error for invalid artifact_id", async () => {
		const { handle, cleanup } = makeAgentWithArtifacts();
		const registry = createArtifactToolRegistry(() => handle);
		const read = registry.artifact_read;
		const result = await read({
			name: "artifact_read",
			arguments: { artifact_id: "" },
			id: "1",
		});
		assert.ok("content" in result);
		const text = result.content[0].type === "text" ? result.content[0].text : "";
		assert.ok(text.includes("artifact_id"));
		cleanup();
	});

	it("artifact_read returns error for null artifact_id", async () => {
		const { handle, cleanup } = makeAgentWithArtifacts();
		const registry = createArtifactToolRegistry(() => handle);
		const read = registry.artifact_read;
		const result = await read({
			name: "artifact_read",
			arguments: { artifact_id: null },
			id: "1",
		});
		assert.ok("content" in result);
		const text = result.content[0].type === "text" ? result.content[0].text : "";
		assert.ok(text.includes("artifact_id"));
		cleanup();
	});

	it("artifact_search returns error for empty pattern", async () => {
		const { handle, cleanup } = makeAgentWithArtifacts();
		const registry = createArtifactToolRegistry(() => handle);
		const search = registry.artifact_search;
		const result = await search({
			name: "artifact_search",
			arguments: { pattern: "" },
			id: "1",
		});
		assert.ok("content" in result);
		const text = result.content[0].type === "text" ? result.content[0].text : "";
		assert.ok(text.includes("pattern"));
		cleanup();
	});

	it("artifact_search returns error for null pattern", async () => {
		const { handle, cleanup } = makeAgentWithArtifacts();
		const registry = createArtifactToolRegistry(() => handle);
		const search = registry.artifact_search;
		const result = await search({
			name: "artifact_search",
			arguments: { pattern: null },
			id: "1",
		});
		assert.ok("content" in result);
		const text = result.content[0].type === "text" ? result.content[0].text : "";
		assert.ok(text.includes("pattern"));
		cleanup();
	});
});
