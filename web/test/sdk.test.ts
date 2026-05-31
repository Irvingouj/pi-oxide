import assert from "node:assert";
import { describe, it } from "node:test";
import {
	ensureInit,
	createHostState,
	destroyHostState,
	createHostAgent,
	destroyHostAgent,
	startTurn,
	hostFeedLlmChunk,
	hostLlmDone,
	hostToolDone,
	hostAcceptCompaction,
	hostContinueTurn,
	getHostStatePersistData,
	restoreHostState,
} from "@pi-oxide/pi-host-web";

await ensureInit();

describe("SDK exports new HostDirective API", () => {
	it("createHostState returns a handle", () => {
		const result = createHostState({
			max_tool_result_chars: 50000,
			max_context_tokens: 100000,
			microcompact_after_turns: 5,
			compaction_threshold: 0.75,
		});
		assert.ok(result.ok, "createHostState should succeed");
		assert.ok(result.data, "createHostState should return data");
		assert.equal(typeof result.data.handle, "number");
		destroyHostState(result.data.handle);
	});

	it("createHostAgent returns a handle", () => {
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
				messages: [],
			},
			{
				max_tool_result_chars: 50000,
				max_context_tokens: 100000,
				microcompact_after_turns: 5,
				compaction_threshold: 0.75,
			},
		);
		assert.ok(result.ok, "createHostAgent should succeed");
		assert.ok(result.data, "createHostAgent should return data");
		assert.equal(typeof result.data.handle, "number");
		destroyHostAgent(result.data.handle);
	});

	it("getHostStatePersistData and restoreHostState roundtrip", () => {
		const createResult = createHostState({
			max_tool_result_chars: 50000,
			max_context_tokens: 100000,
			microcompact_after_turns: 5,
			compaction_threshold: 0.75,
		});
		assert.ok(createResult.ok);
		const handle = createResult.data!.handle;

		const persistResult = getHostStatePersistData(handle);
		assert.ok(persistResult.ok, "getHostStatePersistData should succeed");
		assert.ok(persistResult.data, "getHostStatePersistData should return data");
		const data = persistResult.data.state;
		assert.equal(data.system_prompt, "");
		assert.equal(data.name, "");
		assert.ok(Array.isArray(data.entries));
		assert.ok(Array.isArray(data.artifacts));

		const restoreResult = restoreHostState(data);
		assert.ok(restoreResult.ok, "restoreHostState should succeed");
		assert.ok(restoreResult.data, "restoreHostState should return data");
		const restoredHandle = restoreResult.data.handle;

		const persistResult2 = getHostStatePersistData(restoredHandle);
		assert.ok(persistResult2.ok);
		const data2 = persistResult2.data!.state;
		assert.equal(data2.system_prompt, "");

		destroyHostState(handle);
		destroyHostState(restoredHandle);
	});

	it("startTurn returns directives", () => {
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
				messages: [],
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

		const turnResult = startTurn(handle, {
			prompt: {
				role: "user",
				content: [{ type: "text", text: "hello" }],
				timestamp: 1,
			},
			tools: [],
		});
		assert.ok(turnResult.ok, "startTurn should succeed");
		assert.ok(turnResult.data, "startTurn should return data");
		assert.ok(Array.isArray(turnResult.data.events));
		assert.ok(Array.isArray(turnResult.data.directives));
		assert.ok(
			turnResult.data.directives.some((d: any) => d.type === "stream_llm"),
			"startTurn should emit StreamLlm directive",
		);

		destroyHostAgent(handle);
	});

	it("sdk_new_api_matches_directives", () => {
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
				messages: [],
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

		const turnResult = startTurn(handle, {
			prompt: {
				role: "user",
				content: [{ type: "text", text: "hello" }],
				timestamp: 1,
			},
			tools: [],
		});
		assert.ok(turnResult.ok);
		const directives = turnResult.data!.directives;
		const directiveTypes = directives.map((d: any) => d.type);
		assert.ok(
			directiveTypes.includes("stream_llm"),
			"SDK should emit stream_llm directive",
		);

		destroyHostAgent(handle);
	});

	it("sdk_backward_compat", () => {
		// Old API (createAgent, prompt, onLlmDone) should still exist and work
		const oldAgent = createHostAgent(
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
				messages: [],
			},
			{
				max_tool_result_chars: 50000,
				max_context_tokens: 100000,
				microcompact_after_turns: 5,
				compaction_threshold: 0.75,
			},
		);
		assert.ok(oldAgent.ok, "old-style createHostAgent should still work");
		assert.ok(oldAgent.data, "old-style createHostAgent should return data");
		destroyHostAgent(oldAgent.data!.handle);
	});
});
