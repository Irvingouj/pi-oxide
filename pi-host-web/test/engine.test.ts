import assert from "node:assert";
import { describe, it } from "node:test";
import type { AgentConfig, Content, LlmContext, ToolCall, ToolResult } from "../pi_host_web.js";
import { hostLlmDone, hostPrepareToolCalls, startTurn } from "../pi_host_web.js";
import { ensureInit } from "../sdk/init.ts";
import {
	type AgentRunConfig,
	createHostAgentInstance,
	destroyEngineAgent,
	type LlmStream,
	runTurnWithHostAgent,
} from "../sdk/internal/engine.ts";

await ensureInit();

type MockModelResponse = {
	text?: string;
	toolCalls?: Array<{ id: string; name: string; arguments: object }>;
	stopReason?: "end_turn" | "tool_use";
};

function makeMockModel(returnResult: MockModelResponse | MockModelResponse[]) {
	const responses = Array.isArray(returnResult) ? returnResult : [returnResult];
	let callIndex = 0;

	return {
		call: async (_context: LlmContext, _signal?: AbortSignal): Promise<LlmStream> => {
			const response = responses[Math.min(callIndex++, responses.length - 1)];
			const content: Content[] = [];
			if (response.text) {
				content.push({ type: "text", text: response.text });
			}
			if (response.toolCalls) {
				for (const tc of response.toolCalls) {
					content.push({ type: "tool_call", id: tc.id, name: tc.name, arguments: tc.arguments });
				}
			}
			return {
				chunks: {
					async *[Symbol.asyncIterator]() {
						yield {
							kind: "start",
							content,
							api: "test",
							provider: "test",
							model: "test",
							stop_reason: response.stopReason ?? "end_turn",
							error_message: undefined,
							timestamp: Date.now(),
							usage: { input: 0, output: 0, cache_read: 0, cache_write: 0, total_tokens: 0 },
						};
					},
				},
				result: Promise.resolve({
					Ok: {
						content,
						api: "test",
						provider: "test",
						model: "test",
						stop_reason: response.stopReason ?? "end_turn",
						error_message: undefined,
						timestamp: Date.now(),
						usage: { input: 0, output: 0, cache_read: 0, cache_write: 0, total_tokens: 0 },
					},
				}),
			};
		},
		summarize: async () => "summary",
	};
}

function makeUserMessage(text: string): import("../pi_host_web.js").AgentMessage {
	return {
		role: "user",
		content: [{ type: "text", text }],
		timestamp: Date.now(),
	};
}

function makeToolConfig(overrides: Partial<AgentRunConfig> = {}): AgentRunConfig {
	return {
		llm: makeMockModel({ text: "Hello" }),
		tools: {
			test_tool: async (call: ToolCall): Promise<ToolResult> => ({
				content: [{ type: "text", text: `ran ${call.name}` }],
			}),
		},
		llmTools: [
			{
				name: "test_tool",
				label: "Test",
				description: "A test tool",
				parameters: {},
				execution_mode: "parallel",
				tool_run_mode: "immediate",
			},
		],
		onEvent: () => {},
		...overrides,
	};
}

function makeAgentConfig(): AgentConfig {
	return {
		instructions: "test",
		model: {
			id: "test-model",
			generate: async () => ({ content: [{ type: "text", text: "hi" }], stopReason: "end" }),
		},
	};
}

describe("runTurnWithHostAgent", () => {
	it("executes tool calls with default allow-all policy", async () => {
		const hostAgent = await createHostAgentInstance(makeAgentConfig());
		const events: unknown[] = [];
		const config = makeToolConfig({
			llm: makeMockModel([
				{ toolCalls: [{ id: "tc-1", name: "test_tool", arguments: {} }], stopReason: "tool_use" },
				{ text: "done", stopReason: "end_turn" },
			]),
			onEvent: (ev) => events.push(ev),
		});

		const result = await runTurnWithHostAgent(hostAgent, makeUserMessage("use tool"), config);

		assert.equal(result.aborted, false);
		assert.equal(
			events.filter((e) => e.type === "tool_execution_start" && e.tool_call_id === "tc-1").length,
			1,
			"allowed call should emit exactly one tool_execution_start",
		);
		assert.ok(
			events.some((e) => e.type === "tool_execution_end" && e.tool_call_id === "tc-1"),
			"should emit tool_execution_end",
		);
		destroyEngineAgent(hostAgent);
	});

	it("blocks tool calls via permission hook", async () => {
		const hostAgent = await createHostAgentInstance(makeAgentConfig());
		const events: unknown[] = [];
		const config = makeToolConfig({
			llm: makeMockModel([
				{ toolCalls: [{ id: "tc-1", name: "test_tool", arguments: {} }], stopReason: "tool_use" },
				{ text: "done", stopReason: "end_turn" },
			]),
			onEvent: (ev) => events.push(ev),
			prepareToolCalls: {
				permission: () => ({ type: "block", reason: "not allowed" }),
			},
		});

		const result = await runTurnWithHostAgent(hostAgent, makeUserMessage("use tool"), config);

		assert.equal(result.aborted, false);
		assert.equal(
			events.filter((e) => e.type === "tool_execution_start" && e.tool_call_id === "tc-1").length,
			0,
			"blocked calls should not emit tool_execution_start",
		);
		// Blocked calls should produce a tool_execution_end with is_error
		assert.ok(
			events.some((e) => e.type === "tool_execution_end" && e.tool_call_id === "tc-1" && e.is_error === true),
			"should emit tool_execution_end with is_error for blocked calls",
		);
		destroyEngineAgent(hostAgent);
	});

	it("blocks duplicate preparation entries at the WASM boundary", async () => {
		const hostAgent = await createHostAgentInstance(makeAgentConfig());
		const config = makeToolConfig();

		const start = startTurn(hostAgent.handle, {
			prompt: makeUserMessage("use tool"),
			tools: config.llmTools,
		});
		assert.equal(start.ok, true);

		const llmDone = hostLlmDone(hostAgent.handle, {
			Ok: {
				content: [{ type: "tool_call", id: "tc-1", name: "test_tool", arguments: {} }],
				api: "test",
				provider: "test",
				model: "test",
				stop_reason: "tool_use",
				error_message: undefined,
				timestamp: Date.now(),
				usage: { input: 0, output: 0, cache_read: 0, cache_write: 0, total_tokens: 0 },
			},
		});
		assert.equal(llmDone.ok, true);

		const prepared = hostPrepareToolCalls(
			hostAgent.handle,
			JSON.stringify([
				{ tool_call_id: "tc-1", transform: { type: "none" }, permission: { type: "allow" } },
				{ tool_call_id: "tc-1", transform: { type: "none" }, permission: { type: "allow" } },
			]),
		);
		assert.equal(prepared.ok, true);

		const events = prepared.data?.events ?? [];
		assert.equal(
			events.filter((e) => e.type === "tool_execution_start" && e.tool_call_id === "tc-1").length,
			0,
			"duplicate preparation should not emit tool_execution_start",
		);
		assert.equal(
			events.filter((e) => e.type === "tool_execution_end" && e.tool_call_id === "tc-1" && e.is_error).length,
			1,
			"duplicate preparation should emit one error result",
		);
		assert.equal(
			(prepared.data?.directives ?? []).some((d) => d.type === "execute_tools"),
			false,
			"duplicate preparation should not execute tools",
		);

		destroyEngineAgent(hostAgent);
	});

	it("transform hook rewrites arguments before permission", async () => {
		const hostAgent = await createHostAgentInstance(makeAgentConfig());
		const events: unknown[] = [];
		let permissionSeenArgs: unknown = null;

		const config = makeToolConfig({
			llm: makeMockModel([
				{ toolCalls: [{ id: "tc-1", name: "test_tool", arguments: { original: true } }], stopReason: "tool_use" },
				{ text: "done", stopReason: "end_turn" },
			]),
			onEvent: (ev) => events.push(ev),
			prepareToolCalls: {
				transform: () => ({ type: "rewrite_args", arguments: { rewritten: true } }),
				permission: (call: ToolCall) => {
					permissionSeenArgs = call.arguments;
					return { type: "allow" };
				},
			},
		});

		const result = await runTurnWithHostAgent(hostAgent, makeUserMessage("use tool"), config);

		assert.equal(result.aborted, false);
		assert.deepStrictEqual(permissionSeenArgs, { rewritten: true }, "permission hook should see transformed arguments");
		destroyEngineAgent(hostAgent);
	});

	it("mixed batch: allowed and blocked calls produce correct results", async () => {
		const hostAgent = await createHostAgentInstance(makeAgentConfig());
		const events: unknown[] = [];
		const toolRuns: string[] = [];

		const config = makeToolConfig({
			llm: makeMockModel([
				{
					toolCalls: [
						{ id: "tc-1", name: "test_tool", arguments: {} },
						{ id: "tc-2", name: "test_tool", arguments: {} },
					],
					stopReason: "tool_use",
				},
				{ text: "done", stopReason: "end_turn" },
			]),
			onEvent: (ev) => events.push(ev),
			tools: {
				test_tool: async (call: ToolCall): Promise<ToolResult> => {
					toolRuns.push(call.id);
					return { content: [{ type: "text", text: `ran ${call.id}` }] };
				},
			},
			prepareToolCalls: {
				permission: (call: ToolCall) => {
					if (call.id === "tc-1") return { type: "allow" };
					return { type: "block", reason: "blocked" };
				},
			},
		});

		const result = await runTurnWithHostAgent(hostAgent, makeUserMessage("use tools"), config);

		assert.equal(result.aborted, false);
		assert.deepStrictEqual(toolRuns, ["tc-1"], "only allowed call should run");
		assert.ok(
			events.some((e) => e.type === "tool_execution_end" && e.tool_call_id === "tc-2" && e.is_error === true),
			"blocked call should produce error tool_execution_end",
		);
		destroyEngineAgent(hostAgent);
	});

	it("all blocked calls finalize batch without executing tools", async () => {
		const hostAgent = await createHostAgentInstance(makeAgentConfig());
		const events: unknown[] = [];
		let toolRan = false;

		const config = makeToolConfig({
			llm: makeMockModel([
				{ toolCalls: [{ id: "tc-1", name: "test_tool", arguments: {} }], stopReason: "tool_use" },
				{ text: "done", stopReason: "end_turn" },
			]),
			onEvent: (ev) => events.push(ev),
			tools: {
				test_tool: async (): Promise<ToolResult> => {
					toolRan = true;
					return { content: [{ type: "text", text: "ok" }] };
				},
			},
			prepareToolCalls: {
				permission: () => ({ type: "block", reason: "all blocked" }),
			},
		});

		const result = await runTurnWithHostAgent(hostAgent, makeUserMessage("use tool"), config);

		assert.equal(result.aborted, false);
		assert.equal(toolRan, false, "tool handler should not run when all blocked");
		assert.ok(
			events.some((e) => e.type === "tool_execution_end" && e.tool_call_id === "tc-1" && e.is_error === true),
			"blocked call should still produce tool_execution_end",
		);
		destroyEngineAgent(hostAgent);
	});

	it("hook error synthesizes blocked results for remaining calls", async () => {
		const hostAgent = await createHostAgentInstance(makeAgentConfig());
		const events: unknown[] = [];
		const toolRuns: string[] = [];

		const config = makeToolConfig({
			llm: makeMockModel([
				{
					toolCalls: [
						{ id: "tc-1", name: "test_tool", arguments: {} },
						{ id: "tc-2", name: "test_tool", arguments: {} },
					],
					stopReason: "tool_use",
				},
				{ text: "done", stopReason: "end_turn" },
			]),
			onEvent: (ev) => events.push(ev),
			tools: {
				test_tool: async (call: ToolCall): Promise<ToolResult> => {
					toolRuns.push(call.id);
					return { content: [{ type: "text", text: "ok" }] };
				},
			},
			prepareToolCalls: {
				permission: (call: ToolCall) => {
					if (call.id === "tc-1") throw new Error("permission hook exploded");
					return { type: "allow" };
				},
			},
		});

		const result = await runTurnWithHostAgent(hostAgent, makeUserMessage("use tools"), config);

		assert.equal(result.aborted, false);
		// tc-1 should be blocked due to hook error
		assert.ok(
			events.some((e) => e.type === "tool_execution_end" && e.tool_call_id === "tc-1" && e.is_error === true),
			"hook error should block tc-1",
		);
		// tc-2 should also be blocked (all remaining calls blocked after hook error)
		assert.ok(
			events.some((e) => e.type === "tool_execution_end" && e.tool_call_id === "tc-2" && e.is_error === true),
			"tc-2 should also be blocked after hook error",
		);
		assert.deepStrictEqual(toolRuns, [], "no tools should run after hook error");
		destroyEngineAgent(hostAgent);
	});
});
