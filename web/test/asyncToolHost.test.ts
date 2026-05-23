/**
 * Tests for the async tool host path in RealAgentHost.
 *
 * These tests verify that when a ToolRuntime is provided to RealAgentHost,
 * tool execution goes through the async streaming path with:
 * - onToolStarted before each tool
 * - onToolUpdate for streaming stdout/stderr
 * - onToolDone after each tool completes
 * - tool_execution_update events in trace
 *
 * No network calls are made. A fake LLM produces deterministic responses.
 * Real ToolRuntime is used with a temp directory for real bash execution.
 */

import { describe, it } from "node:test";
import assert from "node:assert/strict";
import * as fs from "node:fs";
import * as path from "node:path";
import * as os from "node:os";

import { RealAgentHost, RealLlm, type TraceEntry } from "../src/providers/realLlm.ts";
import { ToolRuntime, type ToolUpdate } from "../src/local/toolRuntime.ts";
import { PI_CODING_TOOLS } from "../src/tools/schemas.ts";
import type { AgentAction, AgentOptions } from "../src/wasmBinding.ts";
import type { LlmRequest } from "../src/providers/types.ts";

// --- Fake LLM that drives RealAgentHost without network ---

interface FakeResponse {
  text?: string;
  toolCalls?: Array<{ id: string; name: string; arguments: Record<string, unknown> }>;
}

class FakeRealLlm {
  private queue: FakeResponse[];
  readonly log: string[] = [];

  constructor(responses: FakeResponse[]) {
    this.queue = [...responses];
  }

  async call(request: LlmRequest): Promise<{ chunks: object[]; llmResult: object }> {
    const resp = this.queue.shift();
    if (!resp) throw new Error("FakeRealLlm: no more responses");
    this.log.push(`response: ${JSON.stringify(resp)}`);

    const chunks: object[] = [];
    const content: object[] = [];

    // Start chunk
    const base = {
      content: [{ type: "text", text: "" }],
      api: "test", provider: "test", model: "test-model",
      stop_reason: "end_turn" as string,
      error_message: null as string | null,
      timestamp: Date.now(),
      usage: { input: 0, output: 0, cache_read: 0, cache_write: 0, total_tokens: 0 },
    };
    chunks.push({ kind: "start", ...base });

    if (resp.text) {
      content.push({ type: "text", text: resp.text });
      // Split into streaming chunks
      const pieceSize = Math.max(10, Math.ceil(resp.text.length / 3));
      for (let i = 0; i < resp.text.length; i += pieceSize) {
        chunks.push({ kind: "text_delta", text: resp.text.slice(i, i + pieceSize) });
      }
    }

    if (resp.toolCalls) {
      for (const tc of resp.toolCalls) {
        content.push({ type: "tool_call", id: tc.id, name: tc.name, arguments: tc.arguments });
      }
    }

    const stopReason = resp.toolCalls?.length ? "tool_use" : "end_turn";

    const llmResult = {
      Ok: {
        content,
        api: "test", provider: "test", model: "test-model",
        stop_reason: stopReason,
        timestamp: Date.now(),
        usage: { input: 0, output: 0, cache_read: 0, cache_write: 0, total_tokens: 0 },
      },
    };

    return { chunks, llmResult };
  }
}

// --- Helpers ---

function findEvents(trace: TraceEntry[], type: string): TraceEntry[] {
  return trace.filter((e) => e.phase === "event" && e.type === type);
}

function findHost(trace: TraceEntry[], type: string): TraceEntry[] {
  return trace.filter((e) => e.phase === "host" && e.type === type);
}

function findActions(trace: TraceEntry[], type: string): TraceEntry[] {
  return trace.filter((e) => e.phase === "action" && e.type === type);
}

function testOptions(overrides?: Partial<AgentOptions>): AgentOptions {
  return {
    system_prompt: "You are a test agent.",
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
    tools: PI_CODING_TOOLS,
    ...overrides,
  };
}

async function withTempDir(fn: (dir: string) => Promise<void>): Promise<void> {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "pi-oxide-async-host-"));
  try {
    await fn(dir);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

// --- Tests ---

describe("RealAgentHost with async ToolRuntime", () => {
  it("streams bash stdout via tool_execution_update events", async () => {
    await withTempDir(async (dir) => {
      const updates: ToolUpdate[] = [];
      const runtime = new ToolRuntime({
        cwd: dir,
        bashPolicy: { mode: "unrestricted" },
        callbacks: { onUpdate: (u) => updates.push(u) },
      });

      // Fake LLM: first call asks to run bash, second call finishes
      const fakeLlm = new FakeRealLlm([
        {
          toolCalls: [{
            id: "tc-bash-1",
            name: "bash",
            arguments: {
              command: 'node -e "for(let i=0;i<5;i++) console.log(\'line \'+i)"',
              timeout: 5000,
            },
          }],
        },
        { text: "Done running the command." },
      ]);

      // Wrap fakeLlm to satisfy RealLlm interface
      const llm = fakeLlm as unknown as RealLlm;
      const host = new RealAgentHost(llm, { log: [], execute: () => ({}) }, runtime);

      const result = await host.run(testOptions(), "run a command");

      // Agent should finish
      assert.equal(result.terminalAction.type, "finished");

      // Trace should have tool_execution_start events
      const startEvents = findEvents(result.trace, "tool_execution_start");
      assert.ok(startEvents.length >= 1, `expected >= 1 tool_execution_start, got ${startEvents.length}`);

      // Trace should have tool_execution_update events (streaming)
      const updateEvents = findEvents(result.trace, "tool_execution_update");
      assert.ok(updateEvents.length >= 3, `expected >= 3 tool_execution_update, got ${updateEvents.length}`);

      // Trace should have tool_execution_end events
      const endEvents = findEvents(result.trace, "tool_execution_end");
      assert.ok(endEvents.length >= 1, `expected >= 1 tool_execution_end, got ${endEvents.length}`);

      // Tool updates should come before tool_done in the trace
      const firstUpdateIdx = result.trace.findIndex(
        (e) => e.phase === "event" && e.type === "tool_execution_update",
      );
      const toolDoneIdx = result.trace.findIndex(
        (e) => e.phase === "host" && e.type === "tool_done",
      );
      assert.ok(firstUpdateIdx < toolDoneIdx, "tool_execution_update should appear before tool_done");

      // Verify runtime updates were collected (chunking depends on OS buffering)
      assert.ok(updates.length >= 1, `expected >= 1 runtime update, got ${updates.length}`);

      host.cleanup(result.handle);
      runtime.cleanup();
    });
  });

  it("streams stderr separately from stdout", async () => {
    await withTempDir(async (dir) => {
      const updates: ToolUpdate[] = [];
      const runtime = new ToolRuntime({
        cwd: dir,
        bashPolicy: { mode: "unrestricted" },
        callbacks: { onUpdate: (u) => updates.push(u) },
      });

      const fakeLlm = new FakeRealLlm([
        {
          toolCalls: [{
            id: "tc-stderr-1",
            name: "bash",
            arguments: {
              command: 'node -e "console.error(\'stderr-here\'); console.log(\'stdout-here\')"',
              timeout: 5000,
            },
          }],
        },
        { text: "Done." },
      ]);

      const llm = fakeLlm as unknown as RealLlm;
      const host = new RealAgentHost(llm, { log: [], execute: () => ({}) }, runtime);

      const result = await host.run(testOptions(), "run stderr test");
      assert.equal(result.terminalAction.type, "finished");

      // Check that stderr updates exist
      const stderrUpdates = updates.filter((u) => u.stream === "stderr");
      const stdoutUpdates = updates.filter((u) => u.stream === "stdout");
      assert.ok(stderrUpdates.length > 0, "should have stderr updates");
      assert.ok(stdoutUpdates.length > 0, "should have stdout updates");

      const stderrText = stderrUpdates.map((u) => u.chunk).join("");
      assert.ok(stderrText.includes("stderr-here"), `stderr: ${stderrText}`);

      // tool_execution_update events should include both streams
      const traceUpdates = findEvents(result.trace, "tool_execution_update");
      assert.ok(traceUpdates.length >= 2, `expected >= 2 updates, got ${traceUpdates.length}`);

      host.cleanup(result.handle);
      runtime.cleanup();
    });
  });

  it("handles parallel bash calls with separate streaming", async () => {
    await withTempDir(async (dir) => {
      const updates: ToolUpdate[] = [];
      const runtime = new ToolRuntime({
        cwd: dir,
        bashPolicy: { mode: "unrestricted" },
        callbacks: { onUpdate: (u) => updates.push(u) },
      });

      const fakeLlm = new FakeRealLlm([
        {
          toolCalls: [
            {
              id: "tc-para-1",
              name: "bash",
              arguments: { command: "node -e \"setTimeout(() => console.log('result-A'), 200)\"", timeout: 5000 },
            },
            {
              id: "tc-para-2",
              name: "bash",
              arguments: { command: "node -e \"setTimeout(() => console.log('result-B'), 200)\"", timeout: 5000 },
            },
          ],
        },
        { text: "Both commands done." },
      ]);

      const llm = fakeLlm as unknown as RealLlm;
      const host = new RealAgentHost(llm, { log: [], execute: () => ({}) }, runtime);

      const result = await host.run(testOptions(), "run parallel commands");
      assert.equal(result.terminalAction.type, "finished");

      // Both tools should have updates
      const updates1 = updates.filter((u) => u.toolCallId === "tc-para-1");
      const updates2 = updates.filter((u) => u.toolCallId === "tc-para-2");
      assert.ok(updates1.length > 0, "tc-para-1 should have updates");
      assert.ok(updates2.length > 0, "tc-para-2 should have updates");

      // Both should have tool_execution_start and tool_execution_end
      const startEvents = findEvents(result.trace, "tool_execution_start");
      const endEvents = findEvents(result.trace, "tool_execution_end");
      assert.equal(startEvents.length, 2, "should have 2 tool_execution_start events");
      assert.equal(endEvents.length, 2, "should have 2 tool_execution_end events");

      // Two tool_done host entries
      const toolDoneEntries = findHost(result.trace, "tool_done");
      assert.equal(toolDoneEntries.length, 2);

      host.cleanup(result.handle);
      runtime.cleanup();
    });
  });

  it("writes a file and reads it back through the host", async () => {
    await withTempDir(async (dir) => {
      const updates: ToolUpdate[] = [];
      const runtime = new ToolRuntime({
        cwd: dir,
        bashPolicy: { mode: "unrestricted" },
        callbacks: { onUpdate: (u) => updates.push(u) },
      });

      const fakeLlm = new FakeRealLlm([
        // First: write a file
        {
          toolCalls: [{
            id: "tc-write",
            name: "write",
            arguments: { path: "hello.txt", content: "Hello from async runtime!" },
          }],
        },
        // Second: read it back
        {
          toolCalls: [{
            id: "tc-read",
            name: "read",
            arguments: { path: "hello.txt" },
          }],
        },
        // Third: finish
        { text: "File written and read successfully." },
      ]);

      const llm = fakeLlm as unknown as RealLlm;
      const host = new RealAgentHost(llm, { log: [], execute: () => ({}) }, runtime);

      const result = await host.run(testOptions(), "write then read a file");
      assert.equal(result.terminalAction.type, "finished");

      // Should have 2 tool_done entries (write + read)
      const toolDoneEntries = findHost(result.trace, "tool_done");
      assert.equal(toolDoneEntries.length, 2);

      // File should exist
      const content = fs.readFileSync(path.join(dir, "hello.txt"), "utf-8");
      assert.equal(content, "Hello from async runtime!");

      // Two execute_tools actions
      const execActions = findActions(result.trace, "execute_tools");
      assert.equal(execActions.length, 2);

      // Three stream_llm actions (initial + after write + after read)
      const streamActions = findActions(result.trace, "stream_llm");
      assert.equal(streamActions.length, 3);

      host.cleanup(result.handle);
      runtime.cleanup();
    });
  });

  it("agent finishes correctly after async tool execution", async () => {
    await withTempDir(async (dir) => {
      const runtime = new ToolRuntime({
        cwd: dir,
        bashPolicy: { mode: "unrestricted" },
        callbacks: { onUpdate: () => {} },
      });

      // Simple flow: bash echo, then finish
      const fakeLlm = new FakeRealLlm([
        {
          toolCalls: [{
            id: "tc-echo",
            name: "bash",
            arguments: { command: "echo hello-world", timeout: 5000 },
          }],
        },
        { text: "The output says hello-world." },
      ]);

      const llm = fakeLlm as unknown as RealLlm;
      const host = new RealAgentHost(llm, { log: [], execute: () => ({}) }, runtime);

      const result = await host.run(testOptions(), "echo hello");
      assert.equal(result.terminalAction.type, "finished");

      // Trace has full lifecycle
      assert.ok(findEvents(result.trace, "agent_start").length >= 1);
      assert.ok(findEvents(result.trace, "agent_end").length >= 1);
      assert.ok(findEvents(result.trace, "turn_start").length >= 2);
      assert.ok(findEvents(result.trace, "turn_end").length >= 2);
      assert.ok(findActions(result.trace, "stream_llm").length >= 2);

      // tool_execution_start should come before tool_execution_end
      const startIdx = result.trace.findIndex((e) => e.phase === "event" && e.type === "tool_execution_start");
      const endIdx = result.trace.findIndex((e) => e.phase === "event" && e.type === "tool_execution_end");
      assert.ok(startIdx >= 0, "should have tool_execution_start");
      assert.ok(endIdx >= 0, "should have tool_execution_end");
      assert.ok(startIdx < endIdx, "start should come before end");

      host.cleanup(result.handle);
      runtime.cleanup();
    });
  });
});