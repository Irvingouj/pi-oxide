/**
 * Tests for the JS host loop with fake LLM and fake tools.
 *
 * These tests drive the WASM Rust agent through complete loops,
 * verifying that every AgentAction and AgentEvent is correctly
 * processed and logged in the trace.
 */

import { describe, it } from "node:test";
import assert from "node:assert/strict";

import { AgentHost, defaultAgentOptions } from "../src/agentHost.ts";
import { FakeLlm } from "../src/fakeLlm.ts";
import { FakeToolRegistry } from "../src/fakeTools.ts";

// --- Helpers ---

function findActions(trace: ReturnType<AgentHost["trace"]>, type: string) {
  return trace.filter((e) => e.phase === "action" && e.type === type);
}

function findEvents(trace: ReturnType<AgentHost["trace"]>, type: string) {
  return trace.filter((e) => e.phase === "event" && e.type === type);
}

function findHost(trace: ReturnType<AgentHost["trace"]>, type: string) {
  return trace.filter((e) => e.phase === "host" && e.type === type);
}

// --- Tests ---

describe("AgentHost with fake LLM and fake tools", () => {
  it("completes a no-tool text response", () => {
    const llm = new FakeLlm([{ text: "Hello! How can I help?" }]);
    const tools = new FakeToolRegistry();
    const host = new AgentHost(llm, tools);

    const result = host.run(defaultAgentOptions(), "hi");

    // Should finish
    assert.equal(result.terminalAction.type, "finished");

    // Should have exactly one stream_llm action
    const streamActions = findActions(result.trace, "stream_llm");
    assert.equal(streamActions.length, 1);

    // Should have a finished action
    const finishedActions = findActions(result.trace, "finished");
    assert.equal(finishedActions.length, 1);

    // Should have agent_start, turn_start, turn_end, agent_end events
    assert.ok(findEvents(result.trace, "agent_start").length >= 1);
    assert.ok(findEvents(result.trace, "turn_start").length >= 1);
    assert.ok(findEvents(result.trace, "turn_end").length >= 1);
    assert.ok(findEvents(result.trace, "agent_end").length >= 1);

    // Should have feed_llm_chunk host entries from streaming
    const feedChunkEntries = findHost(result.trace, "feed_llm_chunk");
    assert.ok(feedChunkEntries.length >= 2, "should have feed_llm_chunk entries (start + at least one text_delta)");
    // First chunk should be a Start chunk
    assert.equal((feedChunkEntries[0].data as Record<string, unknown>).kind, "start");

    // Should have message_start (from Start chunk) and message_end events
    assert.ok(findEvents(result.trace, "message_start").length >= 2,
      "should have message_start from user message and LLM Start chunk");
    assert.ok(findEvents(result.trace, "message_end").length >= 1);

    // Should have message_update events from TextDelta chunks
    const messageUpdateEvents = findEvents(result.trace, "message_update");
    assert.ok(messageUpdateEvents.length >= 1,
      "should have message_update events from streaming text deltas");

    // LLM should have been called once
    assert.equal(llm.log.length, 1);
    // Tools should NOT have been called
    assert.equal(tools.log.length, 0);

    host.cleanup(result.handle);
  });

  it("completes a single tool call", () => {
    const llm = new FakeLlm([
      // First response: ask to call a tool
      {
        toolCalls: [
          { id: "call-1", name: "read_file", arguments: { path: "/foo.rs" } },
        ],
      },
      // Second response: text after tool result
      { text: "The file contains a hello world program." },
    ]);

    const tools = new FakeToolRegistry();
    tools.register("read_file", () => ({ text: "fn main() { println!(\"hello\"); }" }));

    const host = new AgentHost(llm, tools);
    const result = host.run(defaultAgentOptions(), "read /foo.rs");

    assert.equal(result.terminalAction.type, "finished");

    // Two stream_llm actions (one before tool, one after)
    assert.equal(findActions(result.trace, "stream_llm").length, 2);

    // One execute_tools action
    assert.equal(findActions(result.trace, "execute_tools").length, 1);

    // Tool was called
    assert.equal(tools.log.length, 1);
    assert.ok(tools.log[0].includes("read_file"));

    // Should have tool_execution_start and tool_execution_end events
    assert.ok(findEvents(result.trace, "tool_execution_start").length >= 1);
    assert.ok(findEvents(result.trace, "tool_execution_end").length >= 1);

    // Two turns (one for the tool call, one for the final text)
    assert.equal(findEvents(result.trace, "turn_start").length, 2);
    assert.equal(findEvents(result.trace, "turn_end").length, 2);

    host.cleanup(result.handle);
  });

  it("completes multiple tool calls in parallel", () => {
    const llm = new FakeLlm([
      // First response: ask for two tools
      {
        toolCalls: [
          { id: "call-1", name: "read_file", arguments: { path: "/a.rs" } },
          { id: "call-2", name: "list_files", arguments: { dir: "/src" } },
        ],
      },
      // Second response: summarize
      { text: "Here's the directory listing and the file contents." },
    ]);

    const tools = new FakeToolRegistry();
    tools.register("read_file", () => ({ text: "fn foo() {}" }));
    tools.register("list_files", () => ({ text: "a.rs\nb.rs\nc.rs" }));

    const host = new AgentHost(llm, tools);
    const result = host.run(defaultAgentOptions(), "show /src");

    assert.equal(result.terminalAction.type, "finished");
    assert.equal(findActions(result.trace, "execute_tools").length, 1);
    assert.equal(tools.log.length, 2);

    // Both tools should have start and end events
    assert.equal(findEvents(result.trace, "tool_execution_start").length, 2);
    assert.equal(findEvents(result.trace, "tool_execution_end").length, 2);

    host.cleanup(result.handle);
  });

  it("round-trips tool errors through Rust and back into the trace", () => {
    const llm = new FakeLlm([
      // First: ask for a tool
      {
        toolCalls: [
          { id: "call-1", name: "failing_tool", arguments: {} },
        ],
      },
      // Second: respond after tool error
      { text: "The tool failed. Let me try something else." },
    ]);

    const tools = new FakeToolRegistry();
    tools.register("failing_tool", () => ({
      text: "permission denied: /etc/secret",
      isError: true,
    }));

    const host = new AgentHost(llm, tools);
    const result = host.run(defaultAgentOptions(), "run failing tool");

    assert.equal(result.terminalAction.type, "finished");

    // Tool error should appear in the host trace
    const toolDone = findHost(result.trace, "tool_done");
    assert.equal(toolDone.length, 1);
    assert.ok(toolDone[0].data.payload.error, "tool result should contain error payload");

    // Tool execution end event should have is_error: true
    const toolEndEvents = findEvents(result.trace, "tool_execution_end");
    assert.equal(toolEndEvents.length, 1);
    assert.equal((toolEndEvents[0].data as Record<string, unknown>).is_error, true);

    // But the agent should still finish (not crash)
    assert.equal(findActions(result.trace, "finished").length, 1);

    host.cleanup(result.handle);
  });

  it("handles follow-up messages", () => {
    // Follow-up is queued while the agent is streaming.
    // When the LLM turn ends without tools, the agent drains the follow-up
    // queue and auto-continues with another stream_llm.
    const llm = new FakeLlm([
      { text: "First response." },
      { text: "Follow-up response after queued message." },
    ]);

    const tools = new FakeToolRegistry();
    const host = new AgentHost(llm, tools);

    // Queue a follow-up on the first stream_llm action (while agent is streaming)
    let streamCount = 0;
    host.onStreamLlm = (handle) => {
      streamCount++;
      if (streamCount === 1) {
        // Queue follow-up before the first LLM response is processed
        host.runFollowUp(handle, {
          role: "user",
          content: [{ type: "text", text: "tell me more" }],
          timestamp: Date.now(),
        });
      }
    };

    const result = host.run(defaultAgentOptions(), "first prompt");

    // The agent should have auto-continued after the first response
    // because follow_up was queued, so it finishes after the second response.
    assert.equal(result.terminalAction.type, "finished");

    // Both LLM responses should have been consumed
    assert.equal(llm.log.length, 2);

    // Two stream_llm actions (initial + follow-up auto-continue)
    assert.equal(findActions(result.trace, "stream_llm").length, 2);

    // Follow-up should be in the host trace
    const followUpEntries = findHost(result.trace, "follow_up");
    assert.equal(followUpEntries.length, 1);

    host.cleanup(result.handle);
  });

  it("handles steering messages", () => {
    // Steering is injected during the agent run. When the current turn ends,
    // the steering message is drained and the agent auto-continues.
    // Flow: prompt → stream_llm → tool_call → execute_tools → stream_llm
    //       → steer is queued → text response → auto-continue (steer drained)
    //       → stream_llm → final text → finished
    const llm = new FakeLlm([
      // First response: ask for a tool call
      {
        toolCalls: [
          { id: "call-1", name: "read_file", arguments: { path: "/a.rs" } },
        ],
      },
      // Second response: after tool result
      { text: "I see the file contents." },
      // Third response: after steering is auto-drained
      { text: "Noting the steering instruction too." },
    ]);

    const tools = new FakeToolRegistry();
    tools.register("read_file", () => ({ text: "file contents here" }));

    const host = new AgentHost(llm, tools);

    // Steer on the 2nd stream_llm (after tool execution, before LLM response).
    // The steering will be drained after the 2nd LLM response finishes (no tools),
    // causing an auto-continue to the 3rd stream_llm.
    let streamCount = 0;
    host.onStreamLlm = (handle) => {
      streamCount++;
      if (streamCount === 2) {
        host.runSteer(handle, {
          role: "user",
          content: [{ type: "text", text: "focus on the main function" }],
          timestamp: Date.now(),
        });
      }
    };

    const result = host.run(
      defaultAgentOptions({ steering_mode: "all" }),
      "read the file"
    );

    assert.equal(result.terminalAction.type, "finished");

    // Steering should have produced a queue_update event
    const steerEvents = findEvents(result.trace, "queue_update");
    assert.ok(steerEvents.length >= 1, "should have queue_update event from steering");

    // Three stream_llm actions (initial, post-tool, post-steering)
    assert.equal(findActions(result.trace, "stream_llm").length, 3);

    // Steer entry in host trace
    const steerHost = findHost(result.trace, "steer");
    assert.equal(steerHost.length, 1);

    // All 3 LLM responses consumed
    assert.equal(llm.log.length, 3);

    host.cleanup(result.handle);
  });

  it("preserves complete trace with every action and event in order", () => {
    const llm = new FakeLlm([{ text: "Done." }]);
    const tools = new FakeToolRegistry();
    const host = new AgentHost(llm, tools);

    const result = host.run(defaultAgentOptions(), "do something");

    // The trace should start with host:create_agent, host:prompt
    assert.equal(result.trace[0].phase, "host");
    assert.equal(result.trace[0].type, "create_agent");
    assert.equal(result.trace[1].phase, "host");
    assert.equal(result.trace[1].type, "prompt");

    // Trace should end with action:finished
    const lastAction = result.trace.filter((e) => e.phase === "action").at(-1);
    assert.equal(lastAction?.type, "finished");

    // Verify ordering: no action should appear after the terminal action
    const actionIndices = result.trace
      .map((e, i) => (e.phase === "action" ? i : -1))
      .filter((i) => i >= 0);
    const finishedIdx = actionIndices.at(-1)!;
    // Everything after finishedIdx should be events (from the finished step), not actions
    for (let i = finishedIdx + 1; i < result.trace.length; i++) {
      assert.notEqual(result.trace[i].phase, "action",
        `unexpected action after finished at trace index ${i}`);
    }

    // Print the trace for visual verification
    console.log("\n=== Trace ===");
    for (const entry of result.trace) {
      console.log(`  [${entry.phase}] ${entry.type}`);
    }
    console.log("=== End Trace ===\n");

    host.cleanup(result.handle);
  });

  it("handles LLM error responses", () => {
    const llm = new FakeLlm([
      { error: { code: "rate_limited", message: "Too many requests" } },
    ]);

    const tools = new FakeToolRegistry();
    const host = new AgentHost(llm, tools);

    const result = host.run(defaultAgentOptions(), "trigger error");

    // Error should cause the agent to finish
    assert.equal(result.terminalAction.type, "finished");

    // The final messages should include the error
    const finished = result.terminalAction as { type: string; messages: unknown[] };
    assert.ok(finished.messages.length > 0);

    host.cleanup(result.handle);
  });
});
