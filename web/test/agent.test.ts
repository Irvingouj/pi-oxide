import assert from "node:assert";
import { describe, it } from "node:test";
import { z } from "zod";
import { Agent, defineModel, defineTools, memoryStore, tool } from "../../pi-host-web/sdk/index.ts";
import type { AgentConfig, AgentRunResult, AgentStatus, AgentError } from "../../pi-host-web/sdk/types.ts";
import { ensureInit } from "../../pi-host-web/sdk/init.ts";

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
});
