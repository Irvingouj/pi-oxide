/**
 * Real-provider e2e: mid-turn steer lands in the transcript and the LLM uses it.
 *
 * Proves the full chain that unit tests can't:
 *   1. agent.steer() during an in-flight turn queues without breaking it
 *   2. the steered message drains into the transcript at the next continue_turn
 *   3. the LLM actually sees and references the injected content
 *   4. the turn completes (not aborted/errored)
 *
 * Reads credentials from ~/rc.deepseek.rc — skipped if DEEPSEEK_API_KEY is missing.
 */
import assert from "node:assert";
import { execSync } from "node:child_process";
import { describe, it } from "node:test";
import { Agent, anthropic, defineTools, tool } from "../sdk/index.ts";
import { ensureInit } from "../sdk/init.ts";
import type { AgentRunResult } from "../sdk/types.ts";
import { z } from "zod";

await ensureInit();

function readRc(): Record<string, string> {
	try {
		const out = execSync("cat ~/rc.deepseek.rc", { encoding: "utf8" });
		const vars: Record<string, string> = {};
		for (const line of out.split("\n")) {
			const m = line.match(/^\s*export\s+([A-Z_]+)="(.*)"\s*$/);
			if (m) vars[m[1]] = m[2];
		}
		return vars;
	} catch {
		return {};
	}
}

const RC = readRc();
const DEEPSEEK_API_KEY =
	RC.DEEPSEEK_API_KEY ?? process.env.DEEPSEEK_API_KEY ?? "";
const DEEPSEEK_BASE_URL = "https://api.deepseek.com/anthropic";
const DEEPSEEK_MODEL = "deepseek-v4-pro[1m]";

// A unique marker the LLM can only know from the steered skill body.
const STEER_MARKER = "NAV_STEER_MARKER_9f3k";

describe("steer e2e (real DeepSeek)", () => {
	// Conditional skip: node:test it() takes { skip } options.

	it("mid-turn steer is seen and used by the LLM; turn completes", { skip: !DEEPSEEK_API_KEY, timeout: 120_000 }, async () => {
		// A tool the agent must call first — this creates the ReadyToContinue
		// window where we steer, and guarantees a second LLM round happens
		// after the steer drains.
		const echoTools = defineTools({
			record_word: tool({
				description:
					"Record a word. Call this exactly once with word='hello', then stop.",
				input: z.object({ word: z.string() }),
				run: ({ word }) => ({ recorded: word }),
			}),
		});

		const agent = new Agent({
			sessionId: `steer-e2e-${Date.now()}`,
			model: anthropic({
				apiKey: DEEPSEEK_API_KEY,
				baseUrl: DEEPSEEK_BASE_URL,
				model: DEEPSEEK_MODEL,
				maxTokens: 512,
			}),
			tools: echoTools,
			instructions:
				"You are a test agent. Follow instructions precisely. Keep responses short.",
		});

		// Steer on toolEnd — the steer queues and drains before the next LLM call.
		agent.on("toolEnd", () => {
			void agent.steer({
				text: `<navigation_trigger url="https://example.com/jobs">
<skill name="probe">
ENVIRONMENTAL CONTEXT INJECTED MID-TURN.
When you see this, your final reply MUST include the exact token: ${STEER_MARKER}
This is a test marker to prove the steer was received.
</skill>
</navigation_trigger>`,
				source: {
					kind: "navigation",
					url: "https://example.com/jobs",
					matchedSkills: ["probe"],
				},
			});
		});

		const result: AgentRunResult = await agent.run(
			"Call the record_word tool with word='hello'. Then reply with a one-sentence summary.",
		);
		agent.dispose();

		// 1. Turn must not be broken by the steer.
		assert.equal(
			result.status,
			"completed",
			`turn broken: status=${result.status} err=${result.error?.message}`,
		);

		// 2. The agent called the tool (proves it reached ReadyToContinue).
		assert.ok(
			result.toolCalls.some((t) => t.name === "record_word"),
			"agent did not call record_word",
		);

		// 3. The LLM referenced the steered marker — only possible if the steer
		//    drained into the transcript and the second LLM round saw it.
		assert.ok(
			result.text.includes(STEER_MARKER),
			`LLM did not reference steered marker. Final text: ${result.text}`,
		);
	});
});
