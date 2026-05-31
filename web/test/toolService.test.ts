import assert from "node:assert";
import { describe, it } from "node:test";
import {
	createHostAgent,
	destroyHostAgent,
	ensureInit,
	hostContinueTurn,
	hostLlmDone,
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

	function makeAgentWithProjectedArtifact(): {
		handle: number;
		artifactId: string;
		cleanup: () => void;
	} {
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

		// Turn 1: run a grep tool with a large result to trigger projection
		startTurn(handle, {
			prompt: {
				role: "user",
				content: [{ type: "text", text: "use tool" }],
				timestamp: 1,
			},
			tools: [
				{
					name: "grep",
					label: "Grep",
					description: "Search",
					parameters: { type: "object", properties: {} },
					execution_mode: "parallel",
				},
			],
		});

		hostLlmDone(handle, {
			Ok: {
				content: [
					{
						type: "tool_call",
						id: "tc-1",
						name: "grep",
						arguments: {},
					},
				],
				api: "test",
				provider: "test",
				model: "test-model",
				stop_reason: "tool_use",
				error_message: undefined,
				timestamp: 1,
				usage: {
					input: 0,
					output: 0,
					cache_read: 0,
					cache_write: 0,
					total_tokens: 0,
				},
			},
		});

		hostToolDone(handle, "tc-1", {
			content: [{ type: "text", text: "x".repeat(3001) }],
		});

		hostContinueTurn(handle);

		const llm2 = hostLlmDone(handle, {
			Ok: {
				content: [{ type: "text", text: "done" }],
				api: "test",
				provider: "test",
				model: "test-model",
				stop_reason: "end_turn",
				error_message: undefined,
				timestamp: 1,
				usage: {
					input: 0,
					output: 0,
					cache_read: 0,
					cache_write: 0,
					total_tokens: 0,
				},
			},
		});

		const markers = (llm2.data as any)?.markers ?? [];
		const artifactId = markers[0]?.entry_ids?.[0] ?? "entry-0";

		return {
			handle,
			artifactId,
			cleanup: () => destroyHostAgent(handle),
		};
	}

	it("artifact_read returns content after projection", async () => {
		const { handle, artifactId, cleanup } = makeAgentWithProjectedArtifact();
		const registry = createArtifactToolRegistry(() => handle);

		const read = registry.artifact_read;
		const result = await read({
			name: "artifact_read",
			arguments: { artifact_id: artifactId },
			id: "1",
		});
		assert.ok("content" in result);
		const text = result.content[0].type === "text" ? result.content[0].text : "";
		assert.ok(
			text.length > 3000,
			"artifact_read should return the full projected content",
		);
		assert.ok(
			!text.includes("not found"),
			"artifact_read should find the projected artifact",
		);
		cleanup();
	});

	it("artifact_search finds projected artifacts", async () => {
		const { handle, cleanup } = makeAgentWithProjectedArtifact();
		const registry = createArtifactToolRegistry(() => handle);

		const search = registry.artifact_search;
		const result = await search({
			name: "artifact_search",
			arguments: { pattern: "xxxx" },
			id: "1",
		});
		assert.ok("content" in result);
		const text = result.content[0].type === "text" ? result.content[0].text : "";
		const parsed = JSON.parse(text);
		assert.ok(Array.isArray(parsed));
		assert.ok(
			parsed.length > 0,
			"artifact_search should find at least one projected artifact",
		);
		assert.ok(
			parsed.some((r: { id: string }) => r.id === "entry-0"),
			"artifact_search results should contain the projected artifact id",
		);
		cleanup();
	});

	it("artifact_read is async-shaped without ArtifactStore", async () => {
		const { handle, cleanup } = makeAgentWithArtifacts();
		const registry = createArtifactToolRegistry(() => handle);

		const result = registry.artifact_read({
			name: "artifact_read",
			arguments: { artifact_id: "any" },
			id: "1",
		});
		assert.ok(
			result instanceof Promise,
			"artifact_read should return a Promise when no ArtifactStore is provided",
		);
		await result; // consume promise to avoid unhandled rejection
		cleanup();
	});

	it("artifact_search is async-shaped without ArtifactStore", async () => {
		const { handle, cleanup } = makeAgentWithArtifacts();
		const registry = createArtifactToolRegistry(() => handle);

		const result = registry.artifact_search({
			name: "artifact_search",
			arguments: { pattern: "test" },
			id: "1",
		});
		assert.ok(
			result instanceof Promise,
			"artifact_search should return a Promise when no ArtifactStore is provided",
		);
		await result; // consume promise to avoid unhandled rejection
		cleanup();
	});

	it("image content produces placeholder artifact", async () => {
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

		// Turn 1: run a grep tool with large text + image content
		startTurn(handle, {
			prompt: {
				role: "user",
				content: [{ type: "text", text: "use tool" }],
				timestamp: 1,
			},
			tools: [
				{
					name: "grep",
					label: "Grep",
					description: "Search",
					parameters: { type: "object", properties: {} },
					execution_mode: "parallel",
				},
			],
		});

		hostLlmDone(handle, {
			Ok: {
				content: [
					{
						type: "tool_call",
						id: "tc-1",
						name: "grep",
						arguments: {},
					},
				],
				api: "test",
				provider: "test",
				model: "test-model",
				stop_reason: "tool_use",
				error_message: undefined,
				timestamp: 1,
				usage: {
					input: 0,
					output: 0,
					cache_read: 0,
					cache_write: 0,
					total_tokens: 0,
				},
			},
		});

		hostToolDone(handle, "tc-1", {
			content: [
				{ type: "text", text: "x".repeat(3001) },
				{ type: "image", media_type: "image/png", data: "base64data" },
			],
		});

		hostContinueTurn(handle);

		const llm2 = hostLlmDone(handle, {
			Ok: {
				content: [{ type: "text", text: "done" }],
				api: "test",
				provider: "test",
				model: "test-model",
				stop_reason: "end_turn",
				error_message: undefined,
				timestamp: 1,
				usage: {
					input: 0,
					output: 0,
					cache_read: 0,
					cache_write: 0,
					total_tokens: 0,
				},
			},
		});

		const markers = (llm2.data as any)?.markers ?? [];
		const artifactId = markers[0]?.entry_ids?.[0] ?? "entry-0";

		const registry = createArtifactToolRegistry(() => handle);
		const read = registry.artifact_read;
		const result = await read({
			name: "artifact_read",
			arguments: { artifact_id: artifactId },
			id: "1",
		});
		assert.ok("content" in result);
		const text = result.content[0].type === "text" ? result.content[0].text : "";
		assert.ok(
			text.includes("[image: image/png]"),
			"artifact content should contain image placeholder",
		);

		destroyHostAgent(handle);
	});
});
