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
	restoreHostAgent,
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
	stepProcessor,
} from "../src/services/agentService.ts";
import { createArtifactToolRegistry } from "../src/services/toolService.ts";

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

function makeAgentWithLowBudget(): HostAgent {
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
			max_tool_result_chars: 3000,
			max_context_tokens: 100000,
			microcompact_after_turns: 5,
			compaction_threshold: 0.75,
		},
	);
	assert.ok(result.ok);
	return new HostAgent(result.data!.handle);
}

function makeAgentWithVeryLowBudget(): HostAgent {
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
			max_context_tokens: 30,
			microcompact_after_turns: 5,
			compaction_threshold: 0.5,
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

	it("TurnResultOutput contains markers after projection", async () => {
		const agent = makeAgent();
		const capturedSteps: TurnResultOutput[] = [];
		const originalProcessStep = stepProcessor.processStep.bind(stepProcessor);
		stepProcessor.processStep = async (step, hostAgent, config) => {
			capturedSteps.push(step);
			return originalProcessStep(step, hostAgent, config);
		};

		try {
			let llmCalls = 0;
			await runTurnWithHostAgent(agent, "use tool", {
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
													name: "grep",
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
													name: "grep",
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
					async grep(_call: ToolCall): Promise<ToolResult> {
						return { content: [{ type: "text", text: "x".repeat(3001) }] };
					},
				},
				llmTools: [
					{
						name: "grep",
						label: "Grep",
						description: "A test tool.",
						parameters: { type: "object", properties: {} },
						execution_mode: "parallel",
					},
				],
			});

			const hasMarkers = capturedSteps.some(
				(step) =>
					(step as any).markers?.some(
						(m: { type: string }) => m.type === "new_artifacts",
					),
			);
			assert.ok(
				hasMarkers,
				"at least one TurnResultOutput should contain NewArtifacts markers after projection",
			);
		} finally {
			stepProcessor.processStep = originalProcessStep;
		}
		agent.destroy();
	});

	it("ArtifactStore.save called for new artifacts", async () => {
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
		const agent = new HostAgent(createResult.data!.handle, "test-session");

		const saveCalls: Array<{
			sessionId: string;
			artifactId: string;
			content: string;
		}> = [];
		const artifactStore = {
			async save(sessionId: string, artifactId: string, content: string) {
				saveCalls.push({ sessionId, artifactId, content });
			},
			async load() {
				return null;
			},
			async search() {
				return [];
			},
		};

		let llmCalls = 0;
		await runTurnWithHostAgent(agent, "use tool", {
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
												name: "grep",
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
												name: "grep",
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
				async grep(_call: ToolCall): Promise<ToolResult> {
					return { content: [{ type: "text", text: "x".repeat(3001) }] };
				},
			},
			llmTools: [
				{
					name: "grep",
					label: "Grep",
					description: "A test tool.",
					parameters: { type: "object", properties: {} },
					execution_mode: "parallel",
				},
			],
			artifactStore,
		});

		assert.ok(
			saveCalls.length > 0,
			"ArtifactStore.save should be called for new artifacts",
		);
		assert.ok(
			saveCalls[0].content.length > 3000,
			"saved content should be the full artifact text",
		);
		agent.destroy();
	});

	it("ArtifactStore.load called for artifact_read", async () => {
		const loadCalls: Array<{ sessionId: string; artifactId: string }> = [];
		const artifactStore = {
			async save() {},
			async load(sessionId: string, artifactId: string) {
				loadCalls.push({ sessionId, artifactId });
				return "test content";
			},
			async search() {
				return [];
			},
		};

		const registry = createArtifactToolRegistry(
			() => 0,
			artifactStore,
			() => "session-1",
		);

		const result = await registry.artifact_read({
			name: "artifact_read",
			arguments: { artifact_id: "art-1" },
			id: "1",
		});

		assert.equal(loadCalls.length, 1);
		assert.equal(loadCalls[0].sessionId, "session-1");
		assert.equal(loadCalls[0].artifactId, "art-1");
		assert.ok(
			result.content[0].type === "text" &&
				result.content[0].text.includes("test content"),
			"artifact_read should return content from ArtifactStore.load",
		);
	});

	it("ArtifactStore.search called for artifact_search", async () => {
		const searchCalls: Array<{ sessionId: string; pattern: string }> = [];
		const artifactStore = {
			async save() {},
			async load() {
				return null;
			},
			async search(sessionId: string, query: string) {
				searchCalls.push({ sessionId, pattern: query });
				return [
					{ id: "art-1", snippet: "found it", match_count: 1 },
				];
			},
		};

		const registry = createArtifactToolRegistry(
			() => 0,
			artifactStore,
			() => "session-1",
		);

		const result = await registry.artifact_search({
			name: "artifact_search",
			arguments: { pattern: "hello" },
			id: "1",
		});

		assert.equal(searchCalls.length, 1);
		assert.equal(searchCalls[0].sessionId, "session-1");
		assert.equal(searchCalls[0].pattern, "hello");
		assert.ok(
			result.content[0].type === "text" &&
				result.content[0].text.includes("art-1"),
			"artifact_search should return results from ArtifactStore.search",
		);
	});

	it("processStep called after every transition", async () => {
		const agent = makeAgent();
		let processStepCalls = 0;
		const originalProcessStep = stepProcessor.processStep.bind(stepProcessor);
		stepProcessor.processStep = async (step, hostAgent, config) => {
			processStepCalls++;
			return originalProcessStep(step, hostAgent, config);
		};

		try {
			let llmCalls = 0;
			await runTurnWithHostAgent(agent, "use tool", {
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
						return {
							content: [{ type: "text", text: "tool-result" }],
						};
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

			assert.equal(
				processStepCalls,
				5,
				"processStep should be called after startTurn, llmDone, toolDone, continueTurn, and final llmDone",
			);
		} finally {
			stepProcessor.processStep = originalProcessStep;
		}
		agent.destroy();
	});

	// -----------------------------------------------------------------------
	// E2E integration tests — full artifact lifecycle
	// -----------------------------------------------------------------------

	it("e2e_full_turn_with_projection_artifact_tools_work", async () => {
		const agent = makeAgentWithLowBudget();
		const registry = createArtifactToolRegistry(() => agent.handle);

		let llmCalls = 0;
		await runTurnWithHostAgent(agent, "use tool", {
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
												name: "grep",
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
												name: "grep",
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
				async grep(_call: ToolCall): Promise<ToolResult> {
					return { content: [{ type: "text", text: "x".repeat(3001) }] };
				},
			},
			llmTools: [
				{
					name: "grep",
					label: "Grep",
					description: "A test tool.",
					parameters: { type: "object", properties: {} },
					execution_mode: "parallel",
				},
			],
		});

		// Verify artifact_read returns the projected content
		const readResult = await registry.artifact_read({
			name: "artifact_read",
			arguments: { artifact_id: "entry-0" },
			id: "1",
		});
		assert.ok("content" in readResult);
		const text = readResult.content[0].type === "text" ? readResult.content[0].text : "";
		assert.ok(
			text.length > 3000,
			"artifact_read should return full projected content",
		);
		assert.ok(
			!text.includes("not found"),
			"artifact_read should find the projected artifact",
		);

		// Verify artifact_search finds the artifact
		const searchResult = await registry.artifact_search({
			name: "artifact_search",
			arguments: { pattern: "xxxx" },
			id: "1",
		});
		assert.ok("content" in searchResult);
		const searchText = searchResult.content[0].type === "text" ? searchResult.content[0].text : "";
		const parsed = JSON.parse(searchText);
		assert.ok(Array.isArray(parsed));
		assert.ok(
			parsed.length > 0,
			"artifact_search should find at least one projected artifact",
		);
		assert.ok(
			parsed.some((r: { id: string }) => r.id === "entry-0"),
			"artifact_search results should contain the projected artifact id",
		);

		agent.destroy();
	});

	it("e2e_compaction_creates_searchable_artifacts", async () => {
		const agent = makeAgentWithVeryLowBudget();
		const registry = createArtifactToolRegistry(() => agent.handle);

		// Turn 1: run a tool to create an OriginalTool in T
		let llmCalls = 0;
		await runTurnWithHostAgent(agent, "use tool", {
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
												name: "bash",
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
												name: "bash",
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
				async bash(_call: ToolCall): Promise<ToolResult> {
					return {
						content: [{ type: "text", text: "tool output for compaction test" }],
					};
				},
			},
			llmTools: [
				{
					name: "bash",
					label: "Bash",
					description: "A test tool.",
					parameters: { type: "object", properties: {} },
					execution_mode: "parallel",
				},
			],
		});

		// Turn 2: long prompt to trigger compaction
		const longPrompt = "a".repeat(100);
		let summarizeCalled = false;
		const persistCalls: PersistData[] = [];

		await runTurnWithHostAgent(agent, longPrompt, {
			llm: {
				async call(): Promise<LlmStream> {
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
					return "compaction summary";
				},
			},
			tools: {},
			onPersist: async (data) => {
				persistCalls.push(data);
			},
		});

		assert.ok(summarizeCalled, "summarize should be called when compaction is triggered");

		// Verify the last persist data contains artifacts from compaction
		const lastPersist = persistCalls[persistCalls.length - 1];
		assert.ok(lastPersist);
		assert.ok(
			lastPersist.host_artifacts.some((a: [string, string]) => a[0] === "entry-0"),
			"host_artifacts should contain compacted artifact entry-0",
		);

		// Verify artifact_read returns the compacted content
		const readResult = await registry.artifact_read({
			name: "artifact_read",
			arguments: { artifact_id: "entry-0" },
			id: "1",
		});
		assert.ok("content" in readResult);
		const text = readResult.content[0].type === "text" ? readResult.content[0].text : "";
		assert.ok(
			text.includes("tool output for compaction test"),
			"artifact_read should return compacted tool output",
		);

		agent.destroy();
	});

	it("e2e_artifact_store_integration", async () => {
		const saveCalls: Array<{ sessionId: string; artifactId: string; content: string }> = [];
		const loadCalls: Array<{ sessionId: string; artifactId: string }> = [];
		const searchCalls: Array<{ sessionId: string; pattern: string }> = [];

		const artifactStore = {
			async save(sessionId: string, artifactId: string, content: string) {
				saveCalls.push({ sessionId, artifactId, content });
			},
			async load(sessionId: string, artifactId: string) {
				loadCalls.push({ sessionId, artifactId });
				return "stored artifact content";
			},
			async search(sessionId: string, query: string) {
				searchCalls.push({ sessionId, pattern: query });
				return [{ id: "stored-art-1", snippet: "found", match_count: 1 }];
			},
		};

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
				max_tool_result_chars: 3000,
				max_context_tokens: 100000,
				microcompact_after_turns: 5,
				compaction_threshold: 0.75,
			},
		);
		assert.ok(createResult.ok);
		const agent = new HostAgent(createResult.data!.handle, "test-session");

		const persistCalls: PersistData[] = [];
		let llmCalls = 0;

		await runTurnWithHostAgent(agent, "use tool", {
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
												name: "grep",
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
												name: "grep",
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
				async grep(_call: ToolCall): Promise<ToolResult> {
					return { content: [{ type: "text", text: "x".repeat(3001) }] };
				},
			},
			llmTools: [
				{
					name: "grep",
					label: "Grep",
					description: "A test tool.",
					parameters: { type: "object", properties: {} },
					execution_mode: "parallel",
				},
			],
			artifactStore,
			onPersist: async (data) => {
				persistCalls.push(data);
			},
		});

		// Verify ArtifactStore.save was called
		assert.ok(saveCalls.length > 0, "ArtifactStore.save should be called");
		assert.equal(saveCalls[0].sessionId, "test-session");
		assert.equal(saveCalls[0].artifactId, "entry-0");
		assert.ok(
			saveCalls[0].content.length > 3000,
			"saved content should be the full artifact text",
		);

		// Create registry with ArtifactStore to test load/search
		const registry = createArtifactToolRegistry(
			() => agent.handle,
			artifactStore,
			() => "test-session",
		);

		// Verify artifact_read uses ArtifactStore.load
		const readResult = await registry.artifact_read({
			name: "artifact_read",
			arguments: { artifact_id: "art-1" },
			id: "1",
		});
		assert.ok(loadCalls.length > 0, "ArtifactStore.load should be called");
		assert.equal(loadCalls[loadCalls.length - 1].artifactId, "art-1");
		assert.ok(
			readResult.content[0].type === "text" &&
				readResult.content[0].text.includes("stored artifact content"),
			"artifact_read should return content from ArtifactStore",
		);

		// Verify artifact_search uses ArtifactStore.search
		const searchResult = await registry.artifact_search({
			name: "artifact_search",
			arguments: { pattern: "test" },
			id: "1",
		});
		assert.ok(searchCalls.length > 0, "ArtifactStore.search should be called");
		assert.equal(searchCalls[searchCalls.length - 1].pattern, "test");
		assert.ok(
			searchResult.content[0].type === "text" &&
				searchResult.content[0].text.includes("stored-art-1"),
			"artifact_search should return results from ArtifactStore",
		);

		// Verify PersistData was still saved
		assert.ok(persistCalls.length > 0, "onPersist should still be called");

		agent.destroy();
	});

	it("e2e_restore_gap_fills_artifacts_from_core", async () => {
		// Create a PersistData with core artifacts but empty host_artifacts
		const persistData: PersistData = {
			T: [],
			A: {
				"entry-0": {
					entry_id: "entry-0",
					tool_call_id: "tc-1",
					tool_name: "bash",
					content: [{ type: "text", text: "core artifact content" }],
					is_error: false,
					turn: 1,
				},
			},
			turn_number: 1,
			host_artifacts: [],
			budget: {
				max_tool_result_chars: 50000,
				max_context_tokens: 100000,
				microcompact_after_turns: 5,
				compaction_threshold: 0.75,
			},
			system_prompt: "test",
			compaction_prompt: "Summarize.",
		};

		const restoreResult = restoreHostAgent(
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
			persistData,
		);
		assert.ok(restoreResult.ok, `restore failed: ${restoreResult.error?.message}`);
		const agent = new HostAgent(restoreResult.data!.handle);

		// Verify artifact_read works for the gap-filled artifact
		const registry = createArtifactToolRegistry(() => agent.handle);
		const readResult = await registry.artifact_read({
			name: "artifact_read",
			arguments: { artifact_id: "entry-0" },
			id: "1",
		});
		assert.ok("content" in readResult);
		const text = readResult.content[0].type === "text" ? readResult.content[0].text : "";
		assert.ok(
			text.includes("core artifact content"),
			"artifact_read should return content gap-filled from core A",
		);

		// Verify host_state.artifacts was populated
		const state = agent.getPersistData();
		assert.ok(
			state.host_artifacts.some((a: [string, string]) => a[0] === "entry-0"),
			"host_artifacts should contain entry-0 after restore",
		);

		agent.destroy();
	});

	it("e2e_overwrite_guard_preserves_manual_artifacts", async () => {
		// Step 1: Create agent and run a turn that projects an artifact
		const agent = makeAgentWithLowBudget();

		let llmCalls = 0;
		await runTurnWithHostAgent(agent, "use tool", {
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
												name: "grep",
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
												name: "grep",
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
				async grep(_call: ToolCall): Promise<ToolResult> {
					return { content: [{ type: "text", text: "x".repeat(3001) }] };
				},
			},
			llmTools: [
				{
					name: "grep",
					label: "Grep",
					description: "A test tool.",
					parameters: { type: "object", properties: {} },
					execution_mode: "parallel",
				},
			],
		});

		// Step 2: Get persist data and manually modify host_artifacts
		const persist1 = agent.getPersistData();
		assert.ok(persist1.host_artifacts.length > 0);
		const modifiedArtifacts = persist1.host_artifacts.map((a: [string, string]) => {
			if (a[0] === "entry-0") {
				return [a[0], "manually modified content"] as [string, string];
			}
			return a;
		});

		// Step 3: Restore with modified host_artifacts (core A still has original)
		const modifiedPersist: PersistData = {
			...persist1,
			host_artifacts: modifiedArtifacts,
		};

		agent.destroy();

		const restoreResult = restoreHostAgent(
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
			modifiedPersist,
		);
		assert.ok(restoreResult.ok);
		const restoredAgent = new HostAgent(restoreResult.data!.handle);

		// Step 4: Verify the manually modified artifact is preserved
		const registry = createArtifactToolRegistry(() => restoredAgent.handle);
		const readResult = await registry.artifact_read({
			name: "artifact_read",
			arguments: { artifact_id: "entry-0" },
			id: "1",
		});
		assert.ok("content" in readResult);
		const text = readResult.content[0].type === "text" ? readResult.content[0].text : "";
		assert.equal(
			text,
			"manually modified content",
			"manually modified artifact should NOT be overwritten by core A on restore",
		);

		restoredAgent.destroy();
	});

	// -----------------------------------------------------------------------
	// Error handling and edge case tests
	// -----------------------------------------------------------------------

	it("artifactStore.save throws error propagates to turn", async () => {
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
				max_tool_result_chars: 3000,
				max_context_tokens: 100000,
				microcompact_after_turns: 5,
				compaction_threshold: 0.75,
			},
		);
		assert.ok(createResult.ok);
		const agent = new HostAgent(createResult.data!.handle, "test-session");

		const artifactStore = {
			async save() {
				throw new Error("storage backend failure");
			},
			async load() {
				return null;
			},
			async search() {
				return [];
			},
		};

		let llmCalls = 0;
		let caughtError: Error | undefined;
		try {
			await runTurnWithHostAgent(agent, "use tool", {
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
													name: "grep",
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
													name: "grep",
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
					async grep(_call: ToolCall): Promise<ToolResult> {
						return { content: [{ type: "text", text: "x".repeat(3001) }] };
					},
				},
				llmTools: [
					{
						name: "grep",
						label: "Grep",
						description: "A test tool.",
						parameters: { type: "object", properties: {} },
						execution_mode: "parallel",
					},
				],
				artifactStore,
			});
		} catch (e) {
			caughtError = e as Error;
		}

		assert.ok(caughtError, "error should be propagated from ArtifactStore.save");
		assert.ok(
			caughtError!.message.includes("storage backend failure"),
			"error message should contain the original save error",
		);
		agent.destroy();
	});

	it("artifactStore.load returns null artifact_read throws not found", async () => {
		const artifactStore = {
			async save() {},
			async load() {
				return null;
			},
			async search() {
				return [];
			},
		};

		const registry = createArtifactToolRegistry(
			() => 0,
			artifactStore,
			() => "session-1",
		);

		const result = await registry.artifact_read({
			name: "artifact_read",
			arguments: { artifact_id: "missing-art" },
			id: "1",
		});

		assert.ok("content" in result);
		const text = result.content[0].type === "text" ? result.content[0].text : "";
		assert.ok(
			text.includes("not found"),
			"artifact_read should return error text when ArtifactStore.load returns null",
		);
		assert.ok(
			text.includes("missing-art"),
			"error text should include the artifact id",
		);
	});

	it("empty markers array processStep is safe", async () => {
		const agent = makeAgent();
		const saveCalls: string[] = [];
		const artifactStore = {
			async save(_sessionId: string, artifactId: string) {
				saveCalls.push(artifactId);
			},
			async load() {
				return null;
			},
			async search() {
				return [];
			},
		};

		const step: TurnResultOutput = {
			events: [],
			directives: [{ type: "finished" }],
			markers: [],
		} as TurnResultOutput;

		const result = await stepProcessor.processStep(step, agent, {
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
			artifactStore,
		});

		assert.equal(saveCalls.length, 0, "save should not be called for empty markers");
		assert.equal(result.directives.length, 1, "step should be returned unchanged");
		agent.destroy();
	});

	it("duplicate entry_ids sync is idempotent", async () => {
		// Step 1: Create a real agent with sessionId and run a turn that projects an artifact
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
				max_tool_result_chars: 3000,
				max_context_tokens: 100000,
				microcompact_after_turns: 5,
				compaction_threshold: 0.75,
			},
		);
		assert.ok(createResult.ok);
		const agent = new HostAgent(createResult.data!.handle, "test-session");

		let llmCalls = 0;
		await runTurnWithHostAgent(agent, "use tool", {
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
												name: "grep",
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
													name: "grep",
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
					async grep(_call: ToolCall): Promise<ToolResult> {
						return { content: [{ type: "text", text: "x".repeat(3001) }] };
					},
				},
				llmTools: [
					{
						name: "grep",
						label: "Grep",
						description: "A test tool.",
						parameters: { type: "object", properties: {} },
						execution_mode: "parallel",
					},
				],
			});

		// Step 2: Get the projected artifact id from persist data
		const persistData = agent.getPersistData();
		assert.ok(persistData.host_artifacts.length > 0, "turn should have projected at least one artifact");
		const artifactId = persistData.host_artifacts[0][0];

		// Step 3: Create a fresh ArtifactStore and call processStep with duplicate entry_ids
		const saveCalls: Array<{ sessionId: string; artifactId: string }> = [];
		const artifactStore = {
			async save(sessionId: string, artifactId: string) {
				saveCalls.push({ sessionId, artifactId });
			},
			async load() {
				return null;
			},
			async search() {
				return [];
			},
		};

		const step: TurnResultOutput = {
			events: [],
			directives: [{ type: "finished" }],
			markers: [
				{
					type: "new_artifacts",
					entry_ids: [artifactId, artifactId, artifactId],
				},
			],
		} as TurnResultOutput;

		await stepProcessor.processStep(step, agent, {
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
			artifactStore,
		});

		assert.equal(
			saveCalls.length,
			1,
			"save should be called only once for duplicate entry_ids",
		);
		assert.equal(saveCalls[0].artifactId, artifactId);
		agent.destroy();
	});

	it("backward compatibility old code ignores markers", async () => {
		const agent = makeAgent();
		const events: string[] = [];

		// Simulate old WASM: no markers field at all
		const step: any = {
			events: [{ type: "message_start" }],
			directives: [{ type: "finished" }],
		};

		const result = await stepProcessor.processStep(step, agent, {
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

		assert.equal(result.directives.length, 1, "step should be returned with directives");
		assert.ok(events.includes("message_start"), "events should still be processed");
		agent.destroy();
	});
});
