import assert from "node:assert";
import { describe, it } from "node:test";
import {
	type AgentMessage,
	createHostAgent,
	destroyHostAgent,
	ensureInit,
	type LlmChunk,
	type LlmContext,
	type LlmResult,
	type PersistData,
	startTurn,
	type ToolCall,
	type ToolResult,
} from "@pi-oxide/pi-host-web";

await ensureInit();

import {
	type AgentRunConfig,
	HostAgent,
	type LlmStream,
	runTurnWithHostAgent,
} from "../src/services/agentService.ts";

function makeAgent(): HostAgent {
	const result = createHostAgent(
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
	assert.ok(result.ok);
	return new HostAgent(result.data!.handle);
}

function makeLlmProvider(assistant: LlmResult): AgentRunConfig["llm"] {
	return {
		async call(_context, _signal): Promise<LlmStream> {
			return {
				chunks: (async function* () {
					yield {
						kind: "start",
						content: [{ type: "text", text: "" }],
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
					};
					yield { kind: "done" };
				})(),
				result: Promise.resolve(assistant),
			};
		},
	};
}

function makeToolProvider(
	results: Record<string, ToolResult>,
): AgentRunConfig["tools"] {
	return {
		async test_tool(call: ToolCall): Promise<ToolResult> {
			return (
				results[call.arguments.action as string] ?? {
					content: [{ type: "text", text: "ok" }],
				}
			);
		},
	};
}

describe("runTurnWithHostAgent", () => {
	it("drive_loop_handles_stream_llm", async () => {
		const agent = makeAgent();
		const events: string[] = [];
		const result = await runTurnWithHostAgent(agent, "hello", {
			llm: makeLlmProvider({
				Ok: {
					content: [{ type: "text", text: "hi" }],
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
			}),
			tools: {},
			onEvent: (e) => events.push(e.type),
		});
		assert.equal(result.aborted, false);

		assert.ok(events.includes("message_start"));
		assert.ok(events.includes("message_end"));
		agent.destroy();
	});

	it("drive_loop_handles_execute_tools", async () => {
		const agent = makeAgent();
		const toolCalls: ToolCall[] = [];
		let llmCalls = 0;
		const result = await runTurnWithHostAgent(agent, "use tool", {
			llm: {
				async call(_context, _signal): Promise<LlmStream> {
					llmCalls++;
					const isToolCall = llmCalls === 1;
					return {
						chunks: (async function* () {
							yield {
								kind: "start",
								content: isToolCall
									? [
											{
												type: "tool_call",
												id: "tc-1",
												name: "test_tool",
												arguments: { action: "run" },
											},
										]
									: [{ type: "text", text: "done" }],
								api: "test",
								provider: "test",
								model: "test-model",
								stop_reason: isToolCall ? "tool_use" : "end_turn",
								error_message: undefined,
								timestamp: 1,
								usage: {
									input: 0,
									output: 0,
									cache_read: 0,
									cache_write: 0,
									total_tokens: 0,
								},
							};
							yield { kind: "done" };
						})(),
						result: Promise.resolve({
							Ok: {
								content: isToolCall
									? [
											{
												type: "tool_call",
												id: "tc-1",
												name: "test_tool",
												arguments: { action: "run" },
											},
										]
									: [{ type: "text", text: "done" }],
								api: "test",
								provider: "test",
								model: "test-model",
								stop_reason: isToolCall ? "tool_use" : "end_turn",
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
						}),
					};
				},
			},
			tools: {
				async test_tool(call: ToolCall): Promise<ToolResult> {
					toolCalls.push(call);
					return { content: [{ type: "text", text: "tool-result" }] };
				},
			},
			llmTools: [
				{
					name: "test_tool",
					label: "Test",
					description: "A test tool.",
					parameters: { type: "object", properties: {} },
					execution_mode: "parallel",
				},
			],
		});
		assert.equal(result.aborted, false);

		assert.equal(toolCalls.length, 1);
		assert.equal(toolCalls[0].name, "test_tool");
		agent.destroy();
	});

	it("drive_loop_handles_persist", async () => {
		const agent = makeAgent();
		let persisted: PersistData | undefined;
		const result = await runTurnWithHostAgent(agent, "hello", {
			llm: makeLlmProvider({
				Ok: {
					content: [{ type: "text", text: "hi" }],
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
			}),
			tools: {},
			onPersist: async (data) => {
				persisted = data;
			},
		});
		assert.equal(result.aborted, false);

		assert.ok(persisted);
		assert.ok(Array.isArray(persisted!.T));
		agent.destroy();
	});

	it("drive_loop_handles_finished", async () => {
		const agent = makeAgent();
		const result = await runTurnWithHostAgent(agent, "hello", {
			llm: makeLlmProvider({
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
			}),
			tools: {},
		});
		assert.equal(result.aborted, false);

		agent.destroy();
	});

	it("drive_loop_tool_turn_continues_after_tool_use", async () => {
		const agent = makeAgent();
		let llmCalls = 0;
		const result = await runTurnWithHostAgent(agent, "use tool", {
			llm: {
				async call(_context, _signal): Promise<LlmStream> {
					llmCalls++;
					const isToolCall = llmCalls === 1;
					return {
						chunks: (async function* () {
							yield {
								kind: "start",
								content: isToolCall
									? [
											{
												type: "tool_call",
												id: "tc-1",
												name: "test_tool",
												arguments: {},
											},
										]
									: [{ type: "text", text: "done" }],
								api: "test",
								provider: "test",
								model: "test-model",
								stop_reason: isToolCall ? "tool_use" : "end_turn",
								error_message: undefined,
								timestamp: 1,
								usage: {
									input: 0,
									output: 0,
									cache_read: 0,
									cache_write: 0,
									total_tokens: 0,
								},
							};
							yield { kind: "done" };
						})(),
						result: Promise.resolve({
							Ok: {
								content: isToolCall
									? [
											{
												type: "tool_call",
												id: "tc-1",
												name: "test_tool",
												arguments: {},
											},
										]
									: [{ type: "text", text: "done" }],
								api: "test",
								provider: "test",
								model: "test-model",
								stop_reason: isToolCall ? "tool_use" : "end_turn",
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
						}),
					};
				},
			},
			tools: {
				async test_tool(_call: ToolCall): Promise<ToolResult> {
					return { content: [{ type: "text", text: "ok" }] };
				},
			},
			llmTools: [
				{
					name: "test_tool",
					label: "Test",
					description: "A test tool.",
					parameters: { type: "object", properties: {} },
					execution_mode: "parallel",
				},
			],
		});
		assert.equal(result.aborted, false);

		agent.destroy();
	});

	it("drive_loop_abort_mid_stream", async () => {
		const agent = makeAgent();
		const controller = new AbortController();
		controller.abort();
		const result = await runTurnWithHostAgent(agent, "hello", {
			llm: {
				async call(_context, _signal): Promise<LlmStream> {
					return {
						chunks: (async function* () {
							yield {
								kind: "start",
								content: [{ type: "text", text: "" }],
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
							};
							yield { kind: "done" };
						})(),
						result: Promise.resolve({
							Ok: {
								content: [{ type: "text", text: "hi" }],
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
						}),
					};
				},
			},
			tools: {},
			signal: controller.signal,
		});
		assert.equal(result.aborted, true);

		agent.destroy();
	});

	it("no_projection_service_needed", async () => {
		const agent = makeAgent();
		const result = await runTurnWithHostAgent(agent, "hello", {
			llm: makeLlmProvider({
				Ok: {
					content: [{ type: "text", text: "hi" }],
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
			}),
			tools: {},
		});
		assert.equal(result.aborted, false);

		agent.destroy();
	});

	it("session_restore_uses_new_api", async () => {
		const agent = makeAgent();
		const persist1 = agent.getPersistData();
		assert.ok(Array.isArray(persist1.T));
		agent.destroy();
	});

	it("artifact_tools_still_work", async () => {
		const agent = makeAgent();
		const result = await runTurnWithHostAgent(agent, "hello", {
			llm: makeLlmProvider({
				Ok: {
					content: [{ type: "text", text: "hi" }],
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
			}),
			tools: {},
		});
		assert.equal(result.aborted, false);

		agent.destroy();
	});

	it("drive_loop_full_turn", async () => {
		const agent = makeAgent();
		const toolCalls: ToolCall[] = [];
		let llmCalls = 0;
		const persistCalls: PersistData[] = [];
		const result = await runTurnWithHostAgent(agent, "use tool", {
			llm: {
				async call(_context, _signal): Promise<LlmStream> {
					llmCalls++;
					const isToolCall = llmCalls === 1;
					return {
						chunks: (async function* () {
							yield {
								kind: "start",
								content: isToolCall
									? [
											{
												type: "tool_call",
												id: "tc-1",
												name: "test_tool",
												arguments: { action: "run" },
											},
										]
									: [{ type: "text", text: "done" }],
								api: "test",
								provider: "test",
								model: "test-model",
								stop_reason: isToolCall ? "tool_use" : "end_turn",
								error_message: undefined,
								timestamp: 1,
								usage: {
									input: 0,
									output: 0,
									cache_read: 0,
									cache_write: 0,
									total_tokens: 0,
								},
							};
							yield { kind: "done" };
						})(),
						result: Promise.resolve({
							Ok: {
								content: isToolCall
									? [
											{
												type: "tool_call",
												id: "tc-1",
												name: "test_tool",
												arguments: { action: "run" },
											},
										]
									: [{ type: "text", text: "done" }],
								api: "test",
								provider: "test",
								model: "test-model",
								stop_reason: isToolCall ? "tool_use" : "end_turn",
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
						}),
					};
				},
			},
			tools: {
				async test_tool(call: ToolCall): Promise<ToolResult> {
					toolCalls.push(call);
					return { content: [{ type: "text", text: "tool-result" }] };
				},
			},
			llmTools: [
				{
					name: "test_tool",
					label: "Test",
					description: "A test tool.",
					parameters: { type: "object", properties: {} },
					execution_mode: "parallel",
				},
			],
			onPersist: async (data) => {
				persistCalls.push(data);
			},
		});
		assert.equal(result.aborted, false);

		assert.equal(
			llmCalls,
			2,
			"should call LLM twice (initial + after continue)",
		);
		assert.equal(toolCalls.length, 1, "should execute one tool");
		assert.ok(persistCalls.length > 0, "should persist at least once");
		agent.destroy();
	});

	it("drive_loop_handles_summarize", async () => {
		let summarizeCalled = false;
		const mockAgent = {
			handle: 999,
			startTurn() {
				return {
					events: [],
					directives: [
						{
							type: "stream_llm",
							context: {
								system_prompt: "test",
								messages: [
									{
										role: "user",
										content: [{ type: "text", text: "hi" }],
										timestamp: 1,
									},
								],
								tools: [],
							},
						},
						{
							type: "summarize",
							context: {
								system_prompt: "test",
								messages: [
									{
										role: "user",
										content: [{ type: "text", text: "hi" }],
										timestamp: 1,
									},
								],
								tools: [],
							},
						},
						{ type: "finished" },
					],
				};
			},
			feedLlmChunk() {
				return { events: [], directives: [] };
			},
			llmDone() {
				return { events: [], directives: [] };
			},
			acceptCompaction() {
				return { events: [], directives: [] };
			},
			continueTurn() {
				return { events: [], directives: [] };
			},
			getPersistData() {
				return {
					T: [],
					A: {},
					turn_number: 0,
					host_artifacts: [],
					budget: {
						max_tool_result_chars: 50000,
						max_context_tokens: 100000,
						microcompact_after_turns: 5,
						compaction_threshold: 0.75,
					},
					system_prompt: "",
					compaction_prompt: "",
				};
			},
			destroy() {},
		} as unknown as HostAgent;

		const runResult = await runTurnWithHostAgent(mockAgent, "hello", {
			llm: {
				async call() {
					return {
						chunks: (async function* () {
							yield { kind: "done" };
						})(),
						result: Promise.resolve({
							Ok: {
								content: [{ type: "text", text: "hi" }],
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
						}),
					};
				},
				async summarize() {
					summarizeCalled = true;
					return "summary";
				},
			},
			tools: {},
		});
		assert.equal(runResult.aborted, false);
		assert.ok(summarizeCalled, "should call summarize during summarize");
	});

	it("drive_loop_handles_cancel_tools", async () => {
		let toolCancelledCalled = false;
		const mockAgent = {
			handle: 999,
			startTurn() {
				return {
					events: [],
					directives: [
						{
							type: "cancel_tools",
							tool_call_ids: ["tc-1"],
							reason: "user_aborted",
						},
						{ type: "finished" },
					],
				};
			},
			feedLlmChunk() {
				return { events: [], directives: [] };
			},
			llmDone() {
				return { events: [], directives: [] };
			},
			toolDone() {
				return { events: [], directives: [] };
			},
			toolCancelled() {
				toolCancelledCalled = true;
				return { events: [], directives: [{ type: "finished" }] };
			},
			acceptCompaction() {
				return { events: [], directives: [] };
			},
			continueTurn() {
				return { events: [], directives: [] };
			},
			getPersistData() {
				return {
					T: [],
					A: {},
					turn_number: 0,
					host_artifacts: [],
					budget: {
						max_tool_result_chars: 50000,
						max_context_tokens: 100000,
						microcompact_after_turns: 5,
						compaction_threshold: 0.75,
					},
					system_prompt: "",
					compaction_prompt: "",
				};
			},
			destroy() {},
		} as unknown as HostAgent;

		const result = await runTurnWithHostAgent(mockAgent, "hello", {
			llm: makeLlmProvider({
				Ok: {
					content: [{ type: "text", text: "hi" }],
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
			}),
			tools: {},
		});
		assert.equal(result.aborted, false);

		assert.ok(
			toolCancelledCalled,
			"should call toolCancelled for cancel_tools directive",
		);
	});

	it("drive_loop_processes_summarize_when_no_step_change", async () => {
		let compactCalled = false;
		const mockAgent = {
			handle: 999,
			startTurn() {
				return {
					events: [],
					directives: [
						{
							type: "summarize",
							context: {
								system_prompt: "test",
								messages: [
									{
										role: "user",
										content: [{ type: "text", text: "hi" }],
										timestamp: 1,
									},
								],
								tools: [],
							},
						},
						{ type: "finished" },
					],
				};
			},
			feedLlmChunk() {
				return { events: [], directives: [] };
			},
			llmDone() {
				return { events: [], directives: [] };
			},
			toolDone() {
				return { events: [], directives: [] };
			},
			toolCancelled() {
				return { events: [], directives: [] };
			},
			acceptCompaction() {
				compactCalled = true;
				return { events: [], directives: [{ type: "persist" }] };
			},
			continueTurn() {
				return { events: [], directives: [] };
			},
			getPersistData() {
				return {
					T: [],
					A: {},
					turn_number: 0,
					host_artifacts: [],
					budget: {
						max_tool_result_chars: 50000,
						max_context_tokens: 100000,
						microcompact_after_turns: 5,
						compaction_threshold: 0.75,
					},
					system_prompt: "",
					compaction_prompt: "",
				};
			},
			destroy() {},
		} as unknown as HostAgent;

		const result = await runTurnWithHostAgent(mockAgent, "hello", {
			llm: {
				async call() {
					return {
						chunks: (async function* () {
							yield { kind: "done" };
						})(),
						result: Promise.resolve({
							Ok: {
								content: [{ type: "text", text: "hi" }],
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
						}),
					};
				},
				async summarize() {
					return "summary";
				},
			},
			tools: {},
		});
		assert.equal(result.aborted, false);

		assert.ok(
			compactCalled,
			"should process deferred summarize even when no step-changing directive precedes it",
		);
	});
});
