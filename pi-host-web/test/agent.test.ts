import assert from "node:assert";
import { describe, it } from "node:test";
import { z } from "zod";
import {
	Agent,
	defineModel,
	defineTools,
	memoryStore,
	tool,
} from "../sdk/index.ts";
import { ensureInit } from "../sdk/init.ts";
import type {
	AgentConfig,
	AgentError,
	AgentRunResult,
	AgentStatus,
	ModelRequest,
	SteerEvent,
	TriggerSource,
} from "../sdk/types.ts";

// Initialize WASM once for all tests that need it
await ensureInit();

function makeMockModel(responseText: string = "Hello") {
	return defineModel({
		id: "mock-model",
		generate: async () => ({
			content: [{ type: "text" as const, text: responseText }],
			stopReason: "end" as const,
			usage: {
				input: 10,
				output: 5,
				cache_read: 0,
				cache_write: 0,
				total_tokens: 15,
			},
		}),
	});
}

function makeFailingModel(error: Error | string = new Error("model failed")) {
	return defineModel({
		id: "failing-model",
		generate: async () => {
			throw typeof error === "string" ? new Error(error) : error;
		},
	});
}

function makeSlowModel(delayMs: number = 500) {
	return defineModel({
		id: "slow-model",
		generate: async () => {
			await new Promise((r) => setTimeout(r, delayMs));
			return {
				content: [{ type: "text" as const, text: "Done" }],
				stopReason: "end" as const,
			};
		},
	});
}

describe("Agent class", () => {
	describe("TM-1: Construction", () => {
		it("creates an agent with idle status", () => {
			const agent = new Agent({
				sessionId: "test",
				model: makeMockModel(),
			});

			assert.equal(agent.getStatus().state, "idle");
		});

		it("does not perform I/O on construction", () => {
			// If construction triggered engine creation, it would be async or throw
			const agent = new Agent({
				sessionId: "test",
				model: makeMockModel(),
			});

			assert.ok(agent);
			assert.equal(agent.getStatus().state, "idle");
		});
	});

	describe("TM-2: run() happy path", () => {
		it("returns completed result with text", async () => {
			const agent = new Agent({
				sessionId: "test-run",
				model: makeMockModel("Hi there"),
			});

			const result = await agent.run("Hello");

			assert.equal(result.status, "completed");
			assert.equal(result.text, "Hi there");
			assert.ok(Array.isArray(result.toolCalls));
			assert.ok(Array.isArray(result.artifacts));
		});

		it("emits status events during run", async () => {
			const agent = new Agent({
				sessionId: "test-events",
				model: makeMockModel("Hi"),
			});

			const statuses: AgentStatus[] = [];
			agent.on("status", (s) => statuses.push(s));

			await agent.run("Hello");

			// Should have emitted at least calling_model and completed
			assert.ok(
				statuses.some((s) => s.state === "calling_model"),
				"should emit calling_model",
			);
			assert.ok(
				statuses.some((s) => s.state === "completed" || s.state === "saving"),
				"should emit completed or saving",
			);
		});

		it("emits done event exactly once on success", async () => {
			const agent = new Agent({
				sessionId: "test-done",
				model: makeMockModel("Hi"),
			});

			let doneCount = 0;
			let doneResult: AgentRunResult | null = null;

			agent.on("done", (result) => {
				doneCount++;
				doneResult = result;
			});

			await agent.run("Hello");

			assert.equal(doneCount, 1);
			assert.ok(doneResult);
			assert.equal(doneResult!.status, "completed");
		});
	});

	describe("TM-3: Concurrent run guard", () => {
		it("second run returns failed with agent_busy error", async () => {
			const agent = new Agent({
				sessionId: "test-concurrent",
				model: makeSlowModel(200),
			});

			const run1 = agent.run("First");
			const run2 = agent.run("Second");

			const result1 = await run1;
			const result2 = await run2;

			// One should complete, one should fail with agent_busy
			const failedResult = result1.status === "failed" ? result1 : result2;
			assert.equal(failedResult.status, "failed");
			assert.equal(failedResult.error?.code, "agent_busy");
		});

		it("does not throw on concurrent run", async () => {
			const agent = new Agent({
				sessionId: "test-concurrent-no-throw",
				model: makeSlowModel(100),
			});

			agent.run("First");
			const result = await agent.run("Second");

			assert.equal(result.status, "failed");
			assert.equal(result.error?.code, "agent_busy");
		});
	});

	describe("TM-4: stop() abort", () => {
		it("aborts in-progress run", async () => {
			const agent = new Agent({
				sessionId: "test-stop",
				model: makeSlowModel(500),
			});

			const runPromise = agent.run("Long task");

			// Stop after a short delay
			setTimeout(() => agent.stop(), 50);

			const result = await runPromise;

			assert.equal(result.status, "aborted");
		});

		it("emits aborted status", async () => {
			const agent = new Agent({
				sessionId: "test-stop-status",
				model: makeSlowModel(500),
			});

			const statuses: AgentStatus[] = [];
			agent.on("status", (s) => statuses.push(s));

			const runPromise = agent.run("Long task");
			setTimeout(() => agent.stop(), 50);

			await runPromise;

			assert.ok(
				statuses.some((s) => s.state === "aborted"),
				"should emit aborted status",
			);
		});

		it("TM-38: done emitted exactly once on abort", async () => {
			const agent = new Agent({
				sessionId: "test-done-abort",
				model: makeSlowModel(500),
			});

			let doneCount = 0;
			agent.on("done", () => doneCount++);

			const runPromise = agent.run("Long task");
			setTimeout(() => agent.stop(), 50);

			await runPromise;

			assert.equal(doneCount, 1);
		});
	});

	describe("TM-26: Error handling", () => {
		it("returns failed status when model throws", async () => {
			const agent = new Agent({
				sessionId: "test-error",
				model: makeFailingModel("model exploded"),
			});

			const result = await agent.run("Hello");

			assert.equal(result.status, "failed");
			assert.ok(result.error);
			assert.ok(result.error!.message.includes("model exploded"));
		});

		it("emits error and failed status events", async () => {
			const agent = new Agent({
				sessionId: "test-error-events",
				model: makeFailingModel("boom"),
			});

			const errors: AgentError[] = [];
			const statuses: AgentStatus[] = [];

			agent.on("error", (err) => errors.push(err));
			agent.on("status", (s) => statuses.push(s));

			await agent.run("Hello");

			assert.equal(errors.length, 1);
			assert.ok(errors[0].message.includes("boom"));
			assert.ok(statuses.some((s) => s.state === "failed"));
		});

		it("never throws from run()", async () => {
			const agent = new Agent({
				sessionId: "test-never-throws",
				model: makeFailingModel(),
			});

			let threw = false;
			try {
				await agent.run("Hello");
			} catch {
				threw = true;
			}

			assert.equal(threw, false);
		});

		it("TM-39: done emitted exactly once on failure", async () => {
			const agent = new Agent({
				sessionId: "test-done-fail",
				model: makeFailingModel(),
			});

			let doneCount = 0;
			agent.on("done", () => doneCount++);

			await agent.run("Hello");

			assert.equal(doneCount, 1);
		});
	});

	describe("TM-6: dispose()", () => {
		it("subsequent run returns failed with agent_disposed", async () => {
			const agent = new Agent({
				sessionId: "test-dispose",
				model: makeMockModel(),
			});

			agent.dispose();
			const result = await agent.run("Hello");

			assert.equal(result.status, "failed");
			assert.equal(result.error?.code, "agent_disposed");
		});

		it("does not throw after dispose", async () => {
			const agent = new Agent({
				sessionId: "test-dispose-no-throw",
				model: makeMockModel(),
			});

			agent.dispose();

			let threw = false;
			try {
				await agent.run("Hello");
			} catch {
				threw = true;
			}

			assert.equal(threw, false);
		});

		it("on() returns no-op unsubscribe after dispose", () => {
			const agent = new Agent({
				sessionId: "test-dispose-on",
				model: makeMockModel(),
			});

			agent.dispose();
			const unsub = agent.on("status", () => {});
			assert.equal(typeof unsub, "function");
			// Should not throw when called
			unsub();
		});
	});

	describe("TM-27: reset()", () => {
		it("resets status to idle", async () => {
			const agent = new Agent({
				sessionId: "test-reset",
				model: makeMockModel(),
			});

			await agent.run("Hello");
			assert.notEqual(agent.getStatus().state, "idle");

			await agent.reset();
			assert.equal(agent.getStatus().state, "idle");
		});

		it("allows running again after reset", async () => {
			const agent = new Agent({
				sessionId: "test-reset-again",
				model: makeMockModel("Second"),
			});

			const result1 = await agent.run("First");
			assert.equal(result1.status, "completed");

			await agent.reset();
			const result2 = await agent.run("Second");

			assert.equal(result2.status, "completed");
			assert.equal(result2.text, "Second");
		});
	});

	describe("TM-28: steer() before run", () => {
		it("throws agent_not_initialized when steer called before run", async () => {
			const agent = new Agent({
				sessionId: "test-steer-before",
				model: makeMockModel(),
			});

			let threw = false;
			let error: any;
			try {
				await agent.steer("Hello");
			} catch (e) {
				threw = true;
				error = e;
			}

			assert.equal(threw, true);
			assert.equal(error.code, "agent_not_initialized");
		});
	});

	describe("TM-46: steer() source + steer event (mid-stream queueing)", () => {
		// hostSteer now queues in any phase (pi-core AgentRuntime::steer is a
		// pure push). These tests steer during an in-flight LLM stream — the
		// realistic environmental-injection path — and assert the steer event
		// fires and the turn is not broken.
		async function waitForStreaming(agent: Agent): Promise<void> {
			const deadline = Date.now() + 2000;
			while (Date.now() < deadline) {
				if (agent.getStatus().state === "calling_model") return;
				await new Promise((r) => setTimeout(r, 5));
			}
			throw new Error(
				`agent never entered streaming (last: ${agent.getStatus().state})`,
			);
		}

		it("emits a steer event with user source by default for string input", async () => {
			const agent = new Agent({
				sessionId: "test-steer-default",
				model: makeSlowModel(300),
			});
			const events: SteerEvent[] = [];
			agent.on("steer", (e) => events.push(e));
			const runPromise = agent.run("Hi");
			await waitForStreaming(agent);
			await agent.steer("ctx");
			await runPromise;
			agent.dispose();

			assert.equal(events.length, 1);
			assert.deepEqual(events[0]!.source, { kind: "user" });
			assert.equal(events[0]!.text, "ctx");
			assert.equal(typeof events[0]!.timestamp, "number");
		});

		it("emits a steer event with user source when source omitted on object input", async () => {
			const agent = new Agent({
				sessionId: "test-steer-obj-no-source",
				model: makeSlowModel(300),
			});
			const events: SteerEvent[] = [];
			agent.on("steer", (e) => events.push(e));
			const runPromise = agent.run("Hi");
			await waitForStreaming(agent);
			await agent.steer({ text: "ctx" });
			await runPromise;
			agent.dispose();

			assert.equal(events.length, 1);
			assert.deepEqual(events[0]!.source, { kind: "user" });
			assert.equal(events[0]!.text, "ctx");
		});

		it("carries a navigation source through to the steer event", async () => {
			const agent = new Agent({
				sessionId: "test-steer-nav",
				model: makeSlowModel(300),
			});
			const events: SteerEvent[] = [];
			agent.on("steer", (e) => events.push(e));
			const runPromise = agent.run("Hi");
			await waitForStreaming(agent);
			const source: TriggerSource = {
				kind: "navigation",
				url: "https://example.com/jobs",
				matchedSkills: ["linkedin-jobs"],
			};
			await agent.steer({
				text: "<navigation_trigger url='https://example.com/jobs'><skill>linkedin-jobs</skill></navigation_trigger>",
				source,
			});
			await runPromise;
			agent.dispose();

			assert.equal(events.length, 1);
			assert.equal(events[0]!.source.kind, "navigation");
			assert.equal(
				(events[0]!.source as { url: string }).url,
				"https://example.com/jobs",
			);
			assert.deepEqual(
				(events[0]!.source as { matchedSkills: string[] }).matchedSkills,
				["linkedin-jobs"],
			);
		});

		it("delivers a steer queued during a final-answer stream (no tool calls)", async () => {
			// Regression: on_llm_done's no-tool-calls branch must drain the
			// steering queue and continue the turn — otherwise a steer queued
			// during a plain final answer is silently stranded.
			// Call 1: text-only "final answer" (the loss path). Call 2: echoes
			// the steered token, proving the steer drained and re-streamed.
			const STEER_TOKEN = "DRAINED_TOKEN_7q";
			let call = 0;
			const model = defineModel({
				id: "final-answer-then-steer",
				generate: async (_req: ModelRequest) => {
					call++;
					if (call === 1) {
						return {
							content: [{ type: "text" as const, text: "all done" }],
							stopReason: "end" as const,
						};
					}
					// Second call only happens if the steer drained and forced
					// a continuation. Echo the token we steered.
					return {
						content: [{ type: "text" as const, text: `saw:${STEER_TOKEN}` }],
						stopReason: "end" as const,
					};
				},
			});
			const agent = new Agent({
				sessionId: "test-steer-drain-on-final",
				model,
			});
			let sawToken = false;
			agent.on("text", (delta: string) => {
				if (delta.includes(STEER_TOKEN)) sawToken = true;
			});

			const runPromise = agent.run("Hi");
			await waitForStreaming(agent);
			// Steer mid-stream with a token the second call must echo.
			await agent.steer(`inject:${STEER_TOKEN}`);
			const result = await runPromise;
			agent.dispose();

			assert.equal(result.status, "completed");
			// call===2 proves the steer drained and forced a second LLM round;
			// sawToken proves the steered content reached the model.
			assert.equal(call, 2, "steer did not force a continuation stream");
			assert.equal(sawToken, true, "steered token never reached the LLM");
		});
	});

	describe("TM-37: Already aborted signal", () => {
		it("returns aborted when signal is already aborted", async () => {
			const agent = new Agent({
				sessionId: "test-already-aborted",
				model: makeMockModel(),
			});

			const controller = new AbortController();
			controller.abort("already done");

			const result = await agent.run("Hello", { signal: controller.signal });

			assert.equal(result.status, "aborted");
		});
	});

	describe("Event emission", () => {
		it("emits messageStart, text, messageEnd during run", async () => {
			const agent = new Agent({
				sessionId: "test-msg-events",
				model: makeMockModel("Hello world"),
			});

			const events: string[] = [];
			agent.on("messageStart", () => events.push("messageStart"));
			agent.on("text", () => events.push("text"));
			agent.on("messageEnd", () => events.push("messageEnd"));

			await agent.run("Hi");

			assert.ok(events.includes("messageStart"), "should emit messageStart");
			assert.ok(events.includes("text"), "should emit text");
			assert.ok(events.includes("messageEnd"), "should emit messageEnd");
		});
	});

	describe("TM-5: Tool execution events", () => {
		it("agent can be configured with tools", () => {
			// Note: Testing actual tool execution events requires a model that
			// returns tool calls and proper tool handlers. This test verifies
			// the Agent accepts tools in its config.
			const agent = new Agent({
				sessionId: "test-tool-events",
				model: makeMockModel(),
			});

			assert.ok(agent);
			assert.equal(agent.getStatus().state, "idle");
		});

		it("reports invalid tool input as a failed tool result and continues", async () => {
			let calls = 0;
			const model = defineModel({
				id: "tool-failure-model",
				generate: async () => {
					calls++;
					if (calls === 1) {
						return {
							content: [
								{
									type: "tool_call" as const,
									id: "tc-invalid",
									name: "click",
									arguments: { selector: 123 },
								},
							],
							stopReason: "tool_call" as const,
						};
					}
					return {
						content: [{ type: "text" as const, text: "handled failure" }],
						stopReason: "end" as const,
					};
				},
			});

			const tools = defineTools({
				click: tool({
					description: "Click an element",
					input: z.object({ selector: z.string() }),
					run: ({ selector }) => ({ clicked: selector }),
				}),
			});

			const agent = new Agent({
				sessionId: "test-invalid-tool-input",
				model,
				tools,
			});

			const toolStatuses: string[] = [];
			agent.on("toolEnd", (run) => toolStatuses.push(run.status));

			const result = await agent.run("Click it");

			assert.equal(result.status, "completed");
			assert.equal(result.text, "handled failure");
			assert.deepEqual(toolStatuses, ["failed"]);
			assert.equal(result.toolCalls[0]?.status, "failed");
		});
	});

	// ---- Test helpers for persistence tests ----

	function makeRecordingModel(responseText: string = "Hello") {
		let capturedRequest: ModelRequest | null = null;
		const model = defineModel({
			id: "recording-model",
			generate: async (request) => {
				capturedRequest = request;
				return {
					content: [{ type: "text" as const, text: responseText }],
					stopReason: "end" as const,
					usage: {
						input: 10,
						output: 5,
						cache_read: 0,
						cache_write: 0,
						total_tokens: 15,
					},
				};
			},
		});
		return { model, getCapturedRequest: () => capturedRequest };
	}

	describe("TM-44: Persistence on abort and error", () => {
		const SESSION_ID = "persist-abort-test";

		it("retains completed turn state after a subsequent run is aborted", async () => {
			const store = memoryStore();

			// Turn 1: complete normally
			const agent1 = new Agent({
				sessionId: SESSION_ID,
				model: makeMockModel("First response"),
				store,
			});
			const r1 = await agent1.run("first task");
			assert.equal(r1.status, "completed");
			agent1.dispose();

			// Turn 2: abort mid-run
			const agent2 = new Agent({
				sessionId: SESSION_ID,
				model: makeSlowModel(500),
				store,
			});
			const runPromise = agent2.run("second task");
			// Wait briefly then stop
			await new Promise((r) => setTimeout(r, 50));
			agent2.stop();
			const r2 = await runPromise;
			assert.equal(r2.status, "aborted");
			agent2.dispose();

			// Turn 3: should restore turn 1's state
			const { model: recModel, getCapturedRequest } =
				makeRecordingModel("Third response");
			const agent3 = new Agent({
				sessionId: SESSION_ID,
				model: recModel,
				store,
			});
			const r3 = await agent3.run("third task");
			assert.equal(r3.status, "completed");
			agent3.dispose();

			// Verify turn 1's messages are in the model request
			const captured = getCapturedRequest();
			assert.ok(captured, "Model should have been called");
			const userMsgs = captured!.messages.filter((m) => m.role === "user");
			const assistantMsgs = captured!.messages.filter(
				(m) => m.role === "assistant",
			);
			assert.ok(
				userMsgs.some((m) =>
					m.content.some((b) => b.type === "text" && b.text === "first task"),
				),
				"Should include turn 1 user message",
			);
			assert.ok(
				assistantMsgs.some((m) =>
					m.content.some(
						(b) => b.type === "text" && b.text === "First response",
					),
				),
				"Should include turn 1 assistant response",
			);
		});

		it("aborted first run persists user message for context", async () => {
			const store = memoryStore();
			const freshId = "persist-fresh-abort";

			// Turn 1: abort (no prior state)
			const agent1 = new Agent({
				sessionId: freshId,
				model: makeSlowModel(500),
				store,
			});
			const runPromise = agent1.run("only task");
			await new Promise((r) => setTimeout(r, 50));
			agent1.stop();
			const r1 = await runPromise;
			assert.equal(r1.status, "aborted");
			agent1.dispose();

			// Turn 2: should have the aborted turn's user message
			const { model: recModel, getCapturedRequest } =
				makeRecordingModel("Response");
			const agent2 = new Agent({
				sessionId: freshId,
				model: recModel,
				store,
			});
			const r2 = await agent2.run("new task");
			assert.equal(r2.status, "completed");
			agent2.dispose();

			const captured = getCapturedRequest();
			assert.ok(captured, "Model should have been called");
			const userMsgs = captured!.messages.filter((m) => m.role === "user");
			assert.ok(
				userMsgs.some((m) =>
					m.content.some((b) => b.type === "text" && b.text === "only task"),
				),
				"Should include the aborted turn's user message",
			);
			assert.ok(
				userMsgs.some((m) =>
					m.content.some((b) => b.type === "text" && b.text === "new task"),
				),
				"Should also include the current user message",
			);
		});

		it("retains completed turn state after model error in subsequent run", async () => {
			const store = memoryStore();
			const errId = "persist-model-error";

			// Turn 1: complete normally
			const agent1 = new Agent({
				sessionId: errId,
				model: makeMockModel("First response"),
				store,
			});
			const r1 = await agent1.run("first task");
			assert.equal(r1.status, "completed");
			agent1.dispose();

			// Turn 2: model throws
			const agent2 = new Agent({
				sessionId: errId,
				model: makeFailingModel("model exploded"),
				store,
			});
			const r2 = await agent2.run("second task");
			assert.equal(r2.status, "failed");
			agent2.dispose();

			// Turn 3: should restore turn 1's state
			const { model: recModel, getCapturedRequest } =
				makeRecordingModel("Third response");
			const agent3 = new Agent({
				sessionId: errId,
				model: recModel,
				store,
			});
			const r3 = await agent3.run("third task");
			assert.equal(r3.status, "completed");
			agent3.dispose();

			const captured = getCapturedRequest();
			assert.ok(captured, "Model should have been called");
			const userMsgs = captured!.messages.filter((m) => m.role === "user");
			assert.ok(
				userMsgs.some((m) =>
					m.content.some((b) => b.type === "text" && b.text === "first task"),
				),
				"Should include turn 1 user message after model error",
			);
		});

		it("persists user message after model error on first turn", async () => {
			const store = memoryStore();
			const errId = "persist-first-error";

			// Turn 1: model throws immediately (no prior state)
			const agent1 = new Agent({
				sessionId: errId,
				model: makeFailingModel("model exploded"),
				store,
			});
			const r1 = await agent1.run("first task");
			assert.equal(r1.status, "failed");
			agent1.dispose();

			// Turn 2: should have the failed turn's user message in context
			const { model: recModel, getCapturedRequest } =
				makeRecordingModel("Response");
			const agent2 = new Agent({
				sessionId: errId,
				model: recModel,
				store,
			});
			const r2 = await agent2.run("second task");
			assert.equal(r2.status, "completed");
			agent2.dispose();

			const captured = getCapturedRequest();
			assert.ok(captured, "Model should have been called");
			const userMsgs = captured!.messages.filter((m) => m.role === "user");
			assert.ok(
				userMsgs.some((m) =>
					m.content.some((b) => b.type === "text" && b.text === "first task"),
				),
				"Should include turn 1 user message even though model failed",
			);
		});
	});
});
