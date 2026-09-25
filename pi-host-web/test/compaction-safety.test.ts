import assert from "node:assert";
import { describe, it } from "node:test";
import { hostAcceptCompaction, startTurn } from "../pi_host_web.js";
import { buildUserMessage } from "../sdk/orchestration/config-builders.ts";
import { modelResponseToLlmStream } from "../sdk/orchestration/model-adapter.ts";
import { runTurnWithHostAgent } from "../sdk/bindings/turn-loop.ts";
import { Agent } from "../sdk/agent.ts";
import { createHostAgentInstance } from "../sdk/bindings/host-agent.ts";
import { ensureInit } from "../sdk/index.ts";
import { memoryStore } from "../sdk/stores.ts";
import type { AgentRunConfig } from "../sdk/bindings/types.ts";
import type {
	AgentInitialHistoryEntry,
	AgentModel,
	AgentStore,
} from "../sdk/types.ts";

await ensureInit();

const sessionId = "compaction-safety-session";

function historySeed(): AgentInitialHistoryEntry[] {
	const usage = {
		input: 0,
		output: 0,
		cache_read: 0,
		cache_write: 0,
		total_tokens: 0,
	};
	return [
		{
			entryId: "history-user-1",
			turnNumber: 1,
			message: {
				role: "user",
				content: [
					{
						type: "text",
						text: "Earlier user facts and decisions must survive compaction. ".repeat(3),
					},
				],
				timestamp: 1,
			},
		},
		{
			entryId: "history-assistant-1",
			turnNumber: 1,
			message: {
				role: "assistant",
				content: [
					{
						type: "text",
						text: "The first answer contains a confirmed constraint. ".repeat(3),
					},
				],
				api: "test",
				provider: "test",
				model: "test-model",
				stopReason: "end_turn",
				timestamp: 2,
				usage,
			},
		},
		{
			entryId: "history-user-2",
			turnNumber: 2,
			message: {
				role: "user",
				content: [
					{
						type: "text",
						text: "Another earlier request carries an important identifier. ".repeat(3),
					},
				],
				timestamp: 3,
			},
		},
		{
			entryId: "history-assistant-2",
			turnNumber: 2,
			message: {
				role: "assistant",
				content: [
					{
						type: "text",
						text: "The second answer records its completion state. ".repeat(3),
					},
				],
				api: "test",
				provider: "test",
				model: "test-model",
				stopReason: "end_turn",
				timestamp: 4,
				usage,
			},
		},
	];
}

function modelWithSummarizer(
	summarize: NonNullable<AgentModel["summarize"]>,
): AgentModel {
	return {
		id: "tiny-window-model",
		contextWindow: 128,
		maxTokens: 16,
		generate: async () => {
			throw new Error("generation should not run before compaction");
		},
		summarize,
	};
}

function createAgent(model: AgentModel, store: AgentStore): Agent {
	return new Agent({
		sessionId,
		model,
		initialHistory: historySeed(),
		store,
		context: { maxTokens: 10, summarize: true },
	});
}

async function persistedSnapshotText(store: AgentStore): Promise<string> {
	const snapshot = await store.loadSession(sessionId);
	assert.ok(snapshot, "failed run should persist its current transcript");
	return JSON.stringify(snapshot.data) ?? "";
}

function assertUncompactedHistory(snapshotText: string): void {
	assert.match(snapshotText, /Earlier user facts and decisions/);
	assert.match(snapshotText, /Another earlier request/);
	assert.match(snapshotText, /Current request before compaction/);
	assert.doesNotMatch(snapshotText, /"type":"compaction"/);
}

describe("compaction safety", () => {
	it("calls the summarizer with its LLM adapter as the receiver", async () => {
		const host = await createHostAgentInstance({
			sessionId,
			model: modelWithSummarizer(async () => "unused"),
			initialHistory: historySeed(),
			context: { maxTokens: 10 },
		});
		let receiver: AgentRunConfig["llm"] | undefined;
		const llm: AgentRunConfig["llm"] = {
			call: async () =>
				modelResponseToLlmStream(
					{ content: [{ type: "text", text: "Continued" }], stopReason: "end" },
					new AbortController().signal,
				),
			summarize: async function (messages) {
				receiver = this;
				return messages.length > 0 ? "summary" : "empty";
			},
		};

		try {
			await runTurnWithHostAgent(host, buildUserMessage("Current request"), {
				llm,
				tools: {},
				llmTools: [],
			});
		} finally {
			host.destroy();
		}

		assert.equal(receiver, llm);
	});

	it("rejects empty summaries at the WASM boundary without consuming the host", async () => {
		const host = await createHostAgentInstance({
			sessionId,
			model: modelWithSummarizer(async () => "unused"),
			initialHistory: historySeed(),
			context: { maxTokens: 10 },
		});
		const start = startTurn(host.handle, {
			prompt: {
				role: "user",
				content: [{ type: "text", text: "Current request before compaction" }],
				timestamp: 5,
			},
			tools: [],
		});
		assert.equal(start.ok, true);
		const before = host.getPersistData();

		const rejected = hostAcceptCompaction(host.handle, " \n", []);
		assert.equal(rejected.ok, false);
		assert.equal(rejected.error?.code, "empty_compaction_summary");
		assert.deepEqual(host.getPersistData(), before);
		host.destroy();
	});

	it("rejects an empty summary without committing compaction", async () => {
		const store = memoryStore();
		let summarizeCalled = false;
		const agent = createAgent(
			modelWithSummarizer(async () => {
				summarizeCalled = true;
				return "  \n";
			}),
			store,
		);

		const result = await agent.run("Current request before compaction");
		const snapshotText = await persistedSnapshotText(store);
		agent.dispose();

		assert.equal(summarizeCalled, true, "test must exercise compaction");
		assert.equal(result.status, "failed");
		assert.equal(result.error?.code, "empty_compaction_summary");
		assertUncompactedHistory(snapshotText);
	});

	it("rejects empty output from the SDK default summarizer", async () => {
		const store = memoryStore();
		const model: AgentModel = {
			id: "default-summarizer-model",
			contextWindow: 128,
			maxTokens: 16,
			generate: async (request) => ({
				content: [
					{
						type: "text",
						text: request.instructions.startsWith("Summarize") ? "" : "ok",
					},
				],
				stopReason: "end",
			}),
		};
		const agent = createAgent(model, store);

		const result = await agent.run("Current request before compaction");
		const snapshotText = await persistedSnapshotText(store);
		agent.dispose();

		assert.equal(result.status, "failed");
		assertUncompactedHistory(snapshotText);
	});

	it("keeps the transcript when summarization rejects", async () => {
		const store = memoryStore();
		const agent = createAgent(
			modelWithSummarizer(async () => {
				throw new Error("summary provider unavailable");
			}),
			store,
		);

		const result = await agent.run("Current request before compaction");
		const snapshotText = await persistedSnapshotText(store);
		agent.dispose();

		assert.equal(result.status, "failed");
		assert.match(result.error?.message ?? "", /summary provider unavailable/);
		assertUncompactedHistory(snapshotText);
	});

	it("does not apply a late summary after cancellation", async () => {
		const store = memoryStore();
		let resolveSummary: ((summary: string) => void) | null = null;
		let notifyStarted: (() => void) | null = null;
		const summarizerStarted = new Promise<void>((resolve) => {
			notifyStarted = resolve;
		});
		const agent = createAgent(
			modelWithSummarizer(
				() =>
					new Promise<string>((resolve) => {
						resolveSummary = resolve;
					notifyStarted?.();
					}),
			),
			store,
		);
		const abortController = new AbortController();
		const run = agent.run("Current request before compaction", {
			signal: abortController.signal,
		});

		await summarizerStarted;
		abortController.abort();
		if (!resolveSummary) throw new Error("summarizer did not start");
		resolveSummary("A late summary must not be committed.");
		const result = await run;
		const snapshotText = await persistedSnapshotText(store);
		agent.dispose();

		assert.equal(result.status, "aborted");
		assertUncompactedHistory(snapshotText);
	});
});
