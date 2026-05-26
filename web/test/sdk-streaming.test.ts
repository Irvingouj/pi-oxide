/**
 * Test that the SDK streams LLM chunks progressively to feedLlmChunk.
 */

import assert from "node:assert";
import test from "node:test";
import { Agent } from "../../pi-host-web/pkg/sdk/index.js";

test("SDK streams LLM chunks progressively", async () => {
  const events: any[] = [];

  const agent = await Agent.create({
    system_prompt: "test",
    model: {
      id: "test",
      name: "Test",
      provider: "test",
      api: "test",
      reasoning: false,
      context_window: 4096,
      max_tokens: 1024,
      capabilities: { vision: false, json_mode: true, function_calling: true, streaming: true },
      cost: { input: 0, output: 0, cache_read: 0, cache_write: 0 },
    },
    tools: [],
    messages: [],
  });

  const chunkTimestamps: number[] = [];

  const finalAction = await agent.run("hello", {
    llm: {
      async call(_context) {
        return {
          chunks: (async function* () {
            yield { kind: "start", content: [{ type: "text", text: "" }], api: "test", provider: "test", model: "test", stop_reason: "end_turn", error_message: null, timestamp: 0, usage: { input: 0, output: 0, cache_read: 0, cache_write: 0, total_tokens: 0 } };
            await new Promise((r) => setTimeout(r, 10));
            chunkTimestamps.push(Date.now());
            yield { kind: "text_delta", text: "Hello" };
            await new Promise((r) => setTimeout(r, 10));
            chunkTimestamps.push(Date.now());
            yield { kind: "text_delta", text: " world" };
          })(),
          result: Promise.resolve({
            Ok: {
              content: [{ type: "text", text: "Hello world" }],
              api: "test",
              provider: "test",
              model: "test",
              stop_reason: "end_turn",
              error_message: null,
              timestamp: Date.now(),
              usage: { input: 0, output: 0, cache_read: 0, cache_write: 0, total_tokens: 0 },
            },
          }),
        };
      },
    },
    tools: {},
    onEvent(event) {
      events.push(event);
    },
  });

  assert.strictEqual(finalAction.type, "finished");
  assert.ok(
    chunkTimestamps.length >= 2,
    "should have fed multiple chunks progressively"
  );
  assert.ok(
    chunkTimestamps[1] > chunkTimestamps[0],
    "chunks should be spaced in time (streamed)"
  );
  assert.ok(
    events.some((e) => e.type === "message_update"),
    "should emit message_update for streamed text"
  );

  // Deltas must carry incremental text, not accumulated text
  const messageUpdateEvents = events.filter((e) => e.type === "message_update");
  const textDeltas = messageUpdateEvents
    .map((e: any) => e.delta)
    .filter((d) => d?.kind === "text_delta")
    .map((d) => d.text);
  assert.deepStrictEqual(
    textDeltas,
    ["Hello", " world"],
    "text_delta should contain incremental chunks, not accumulated text"
  );

  agent.destroy();
});
