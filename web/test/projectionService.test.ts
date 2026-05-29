import assert from "node:assert";
import { describe, it } from "node:test";
import type { AgentMessage, ContextProjectionState } from "@pi-oxide/pi-host-web";
import { ensureInit } from "@pi-oxide/pi-host-web";
import {
	createProjectionService,
	createTestProjectionService,
} from "../src/services/projectionService.ts";

await ensureInit();

describe("runProjection", () => {
	it("returns original messages when WASM is not initialized", () => {
		const service = createProjectionService();
		const messages: AgentMessage[] = [
			{
				role: "user",
				content: [{ type: "text", text: "hello" }],
				timestamp: 1,
			},
		];
		const result = service.runProjection("test", messages);
		assert.equal(result.length, 1);
		assert.equal(result[0].role, "user");
	});

	it("returns original messages when projectContext throws", () => {
		const service = createTestProjectionService(() => {
			throw new Error("mock projection error");
		});
		const messages: AgentMessage[] = [
			{
				role: "user",
				content: [{ type: "text", text: "hello" }],
				timestamp: 1,
			},
		];
		const result = service.runProjection("system", messages);
		assert.strictEqual(result, messages);
		assert.deepStrictEqual(result, messages);
	});

	it("returns original messages unchanged when mock projectContext returns no replacements", () => {
		const service = createTestProjectionService((input) => {
			return {
				ok: true,
				data: {
					projected_messages: input.messages,
					updated_state: {
						tools: {},
						current_turn: 0,
						last_api_usage: null,
						turns_since_compaction: 0,
					},
					report: {
						estimated_tokens: 0,
						replacements: [],
						dropped_messages: 0,
					},
				},
			};
		});
		const messages: AgentMessage[] = [
			{
				role: "user",
				content: [{ type: "text", text: "hello" }],
				timestamp: 1,
			},
		];
		const result = service.runProjection("system", messages);
		assert.strictEqual(result, messages);
		assert.deepStrictEqual(result, messages);
	});

	it("updates an existing replacement when mock projectContext returns updated state", () => {
		const service = createTestProjectionService((input) => {
			const existing = input.state.tools?.["tc-1"];
			const priorTools = input.state.tools || {};
			if (existing?.type === "replaced") {
				return {
					ok: true,
					data: {
						projected_messages: input.messages,
						updated_state: {
							...input.state,
							tools: {
								...priorTools,
								"tc-1": {
									type: "replaced",
									replacement: {
										...existing.replacement,
										outcome: { text: "updated text" },
									},
									inserted_at_turn: existing.inserted_at_turn,
								},
							},
						},
						report: {
							estimated_tokens: 0,
							replacements: [
								{
									tool_call_id: "tc-1",
									tool_name: "read",
									artifact_id: "tool-result-tc-1",
									original_chars: 100,
									preview_chars: 10,
									strategy: {
										type: "fixed",
										shape: { type: "keep_full" },
										min_age: 0,
									},
									outcome: { text: "updated text" },
								},
							],
							dropped_messages: 0,
						},
					},
				};
			}
			return {
				ok: true,
				data: {
					projected_messages: input.messages,
					updated_state: {
						...input.state,
						tools: {
							...priorTools,
							"tc-1": {
								type: "replaced",
								replacement: {
									tool_call_id: "tc-1",
									tool_name: "read",
									artifact_id: "tool-result-tc-1",
									original_chars: 100,
									preview_chars: 10,
									strategy: {
										type: "fixed",
										shape: { type: "keep_full" },
										min_age: 0,
									},
									outcome: { text: "original text" },
								},
								inserted_at_turn: 0,
							},
						},
					},
					report: {
						estimated_tokens: 0,
						replacements: [
							{
								tool_call_id: "tc-1",
								tool_name: "read",
								artifact_id: "tool-result-tc-1",
								original_chars: 100,
								preview_chars: 10,
								strategy: {
									type: "fixed",
									shape: { type: "keep_full" },
									min_age: 0,
								},
								outcome: { text: "original text" },
							},
						],
						dropped_messages: 0,
					},
				},
			};
		});

		const messages: AgentMessage[] = [
			{
				role: "tool_result",
				tool_call_id: "tc-1",
				tool_name: "read",
				content: [{ type: "text", text: "big text" }],
				is_error: false,
				timestamp: 1,
			},
		];

		const _result1 = service.runProjection("system", messages);
		const state1 = service.getState();
		const tc1 = state1.tools?.["tc-1"];
		assert.ok(tc1, "tc-1 should be in state after first projection");
		assert.equal(tc1.type, "replaced");
		const tc1Replaced = tc1 as {
			type: "replaced";
			replacement: { outcome?: { text: string } };
		};
		assert.equal(tc1Replaced.replacement.outcome?.text, "original text");

		const _result2 = service.runProjection("system", messages);
		const state2 = service.getState();
		const tc2 = state2.tools?.["tc-1"];
		assert.ok(tc2, "tc-1 should still be in state after second projection");
		assert.equal(tc2.type, "replaced");
		const tc2Replaced = tc2 as {
			type: "replaced";
			replacement: { outcome?: { text: string } };
		};
		assert.equal(tc2Replaced.replacement.outcome?.text, "updated text");
	});

	it("replaces oversized tool results with artifact markers via real WASM", () => {
		const service = createProjectionService();
		const bigText = "A".repeat(5000);
		const messages: AgentMessage[] = [
			{
				role: "user",
				content: [{ type: "text", text: "turn 0" }],
				timestamp: 1,
			},
			{
				role: "assistant",
				content: [
					{
						type: "tool_call",
						id: "tc-1",
						name: "read",
						arguments: { path: "x.rs" },
					},
				],
				api: "test",
				provider: "test",
				model: "test",
				stop_reason: "tool_use",
				timestamp: 2,
				usage: {
					input: 0,
					output: 0,
					cache_read: 0,
					cache_write: 0,
					total_tokens: 0,
				},
			},
			{
				role: "tool_result",
				tool_call_id: "tc-1",
				tool_name: "read",
				content: [{ type: "text", text: bigText }],
				is_error: false,
				timestamp: 3,
			},
			{
				role: "user",
				content: [{ type: "text", text: "turn 1" }],
				timestamp: 4,
			},
			{
				role: "assistant",
				content: [{ type: "text", text: "done" }],
				api: "test",
				provider: "test",
				model: "test",
				stop_reason: "end_turn",
				timestamp: 5,
				usage: {
					input: 0,
					output: 0,
					cache_read: 0,
					cache_write: 0,
					total_tokens: 0,
				},
			},
			{
				role: "user",
				content: [{ type: "text", text: "turn 2" }],
				timestamp: 6,
			},
			{
				role: "assistant",
				content: [{ type: "text", text: "done" }],
				api: "test",
				provider: "test",
				model: "test",
				stop_reason: "end_turn",
				timestamp: 7,
				usage: {
					input: 0,
					output: 0,
					cache_read: 0,
					cache_write: 0,
					total_tokens: 0,
				},
			},
			{
				role: "user",
				content: [{ type: "text", text: "turn 3" }],
				timestamp: 8,
			},
			{
				role: "assistant",
				content: [{ type: "text", text: "done" }],
				api: "test",
				provider: "test",
				model: "test",
				stop_reason: "end_turn",
				timestamp: 9,
				usage: {
					input: 0,
					output: 0,
					cache_read: 0,
					cache_write: 0,
					total_tokens: 0,
				},
			},
		];

		const result = service.runProjection("test", messages, {
			max_tool_result_chars: 1000,
		});
		const toolResult = result.find(
			(m): m is Extract<AgentMessage, { role: "tool_result" }> =>
				m.role === "tool_result",
		);
		assert.ok(
			toolResult,
			"tool_result should be present in projected messages",
		);
		const textBlock = toolResult.content.find(
			(c): c is { type: "text"; text: string } => c.type === "text",
		);
		assert.ok(textBlock, "tool_result should have a text block");
		assert.ok(
			textBlock.text.includes("<context-artifact"),
			"oversized tool result should be replaced with artifact marker",
		);
		assert.ok(
			textBlock.text.includes("tool-result-tc-1"),
			"artifact marker should contain the tool call id",
		);

		const state = service.getState();
		const tcState = state.tools?.["tc-1"];
		assert.ok(tcState, "tc-1 should be in projection state");
		assert.equal(tcState.type, "replaced");
	});
});

describe("artifact store", () => {
	it("clearArtifacts removes all entries", () => {
		const service = createProjectionService();
		service.clearArtifacts();
		assert.equal(service.readArtifact("any"), undefined);
	});

	it("searchArtifacts returns empty when no match", () => {
		const service = createProjectionService();
		service.clearArtifacts();
		const results = service.searchArtifacts("nonexistent");
		assert.equal(results.length, 0);
	});

	it("readArtifact returns full text for seeded artifact", () => {
		const service = createTestProjectionService();
		service.clearArtifacts();
		service.__seedArtifactForTest("tool-result-tc-1", "original full text");
		assert.equal(
			service.readArtifact("tool-result-tc-1"),
			"original full text",
		);
	});

	it("searchArtifacts returns matching artifacts for seeded data", () => {
		const service = createTestProjectionService();
		service.clearArtifacts();
		service.__seedArtifactForTest("tool-result-tc-1", "hello world");
		service.__seedArtifactForTest("tool-result-tc-2", "goodbye world");
		const results = service.searchArtifacts("hello");
		assert.equal(results.length, 1);
		assert.equal(results[0].id, "tool-result-tc-1");
		assert.equal(results[0].snippet, "hello world");
	});

	it("caps store at MAX_ARTIFACTS (1000) with FIFO eviction", () => {
		const service = createTestProjectionService();
		service.clearArtifacts();
		for (let i = 0; i < 1001; i++) {
			service.__seedArtifactForTest(`artifact-${i}`, `text-${i}`);
		}
		assert.equal(service.readArtifact("artifact-0"), undefined);
		assert.equal(service.readArtifact("artifact-1"), "text-1");
		assert.equal(service.readArtifact("artifact-1000"), "text-1000");
	});

	it("snapshotArtifacts and loadArtifacts round-trip", () => {
		const service = createTestProjectionService();
		service.clearArtifacts();
		service.__seedArtifactForTest("art-1", "hello world");
		service.__seedArtifactForTest("art-2", "goodbye world");

		const snapshot = service.snapshotArtifacts();
		assert.equal(snapshot.length, 2);
		assert.equal(snapshot[0].id, "art-1");
		assert.equal(snapshot[0].text, "hello world");

		const service2 = createTestProjectionService();
		service2.loadArtifacts(snapshot);
		assert.equal(service2.readArtifact("art-1"), "hello world");
		assert.equal(service2.readArtifact("art-2"), "goodbye world");
	});
});

describe("backward compat", () => {
	it("migrates old session state with replacements through real WASM", () => {
		const service = createProjectionService();
		const oldState = {
			replacements: {
				"tc-old": {
					tool_call_id: "tc-old",
					tool_name: "read",
					artifact_id: "art-old",
					original_chars: 5000,
					preview_chars: 200,
					strategy: { type: "head", max_chars: 200 },
				},
			},
			turn_count: 3,
			last_api_usage: null,
			turns_since_compaction: 0,
		};
		service.restoreState(oldState as unknown as ContextProjectionState);

		const messages: AgentMessage[] = [
			{
				role: "user",
				content: [{ type: "text", text: "turn 0" }],
				timestamp: 1,
			},
			{
				role: "assistant",
				content: [
					{
						type: "tool_call",
						id: "tc-old",
						name: "read",
						arguments: { path: "x.rs" },
					},
				],
				api: "test",
				provider: "test",
				model: "test",
				stop_reason: "tool_use",
				timestamp: 2,
				usage: {
					input: 0,
					output: 0,
					cache_read: 0,
					cache_write: 0,
					total_tokens: 0,
				},
			},
			{
				role: "tool_result",
				tool_call_id: "tc-old",
				tool_name: "read",
				content: [{ type: "text", text: "A".repeat(5000) }],
				is_error: false,
				timestamp: 3,
			},
			{
				role: "user",
				content: [{ type: "text", text: "turn 1" }],
				timestamp: 4,
			},
			{
				role: "assistant",
				content: [{ type: "text", text: "done" }],
				api: "test",
				provider: "test",
				model: "test",
				stop_reason: "end_turn",
				timestamp: 5,
				usage: {
					input: 0,
					output: 0,
					cache_read: 0,
					cache_write: 0,
					total_tokens: 0,
				},
			},
			{
				role: "user",
				content: [{ type: "text", text: "turn 2" }],
				timestamp: 6,
			},
			{
				role: "assistant",
				content: [{ type: "text", text: "done" }],
				api: "test",
				provider: "test",
				model: "test",
				stop_reason: "end_turn",
				timestamp: 7,
				usage: {
					input: 0,
					output: 0,
					cache_read: 0,
					cache_write: 0,
					total_tokens: 0,
				},
			},
			{
				role: "user",
				content: [{ type: "text", text: "turn 3" }],
				timestamp: 8,
			},
		];

		const _result = service.runProjection("test", messages, {
			max_tool_result_chars: 1000,
		});
		const state = service.getState();
		const tcOld = state.tools?.["tc-old"];
		assert.ok(tcOld, "tc-old should be in migrated state after projection");
		assert.equal(tcOld.type, "replaced");
	});
});
