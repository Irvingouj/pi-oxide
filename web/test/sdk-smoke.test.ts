/**
 * Smoke test for the high-level JS SDK (Agent class).
 */

import assert from "node:assert";
import test from "node:test";
import { Agent, toolResult } from "../../pi-host-web/pkg/sdk/index.js";
import { FakeLlm } from "../src/fakeLlm.ts";
import { FakeToolRegistry } from "../src/fakeTools.ts";

test("SDK Agent runs a full turn with fake LLM and tools", async () => {
  const llm = new FakeLlm([
    {
      text: "I'll read the file for you.",
      toolCalls: [
        { id: "call-1", name: "read", arguments: { path: "/tmp/test.txt" } },
      ],
    },
    { text: "Done reading." },
  ]);

  const tools = new FakeToolRegistry();
  tools.register("read", () => ({ text: "hello world" }));

  const events: string[] = [];

  const agent = await Agent.create({
    system_prompt: "You are a test agent.",
    model: {
      id: "test-model",
      name: "Test",
      provider: "test",
      api: "test",
      reasoning: false,
      context_window: 4096,
      max_tokens: 1024,
      capabilities: { vision: false, json_mode: true, function_calling: true, streaming: true },
      cost: { input: 0, output: 0, cache_read: 0, cache_write: 0 },
    },
    tools: [
      {
        name: "read",
        label: "read",
        description: "Read a file",
        parameters: { type: "object", properties: {} },
      },
    ],
    messages: [],
  });

  const finalAction = await agent.run("read /tmp/test.txt", {
    llm: {
      async call(_context) {
        const resp = llm.next();
        const chunks = llm.buildChunks(resp);
        const llmResult = llm.buildLlmResult(resp);
        return {
          chunks: (async function* () {
            for (const chunk of chunks) {
              yield chunk as any;
            }
          })(),
          result: Promise.resolve(llmResult as any),
        };
      },
    },
    tools: {
      read(call) {
        return toolResult(tools.execute(call as any).text ?? "ok");
      },
    },
    onEvent(event) {
      events.push(event.type);
    },
  });

  assert.strictEqual(finalAction.type, "finished");
  assert.ok(events.includes("agent_start"), "should emit agent_start");
  assert.ok(events.includes("turn_start"), "should emit turn_start");
  assert.ok(events.includes("message_start"), "should emit message_start");

  agent.destroy();
});
