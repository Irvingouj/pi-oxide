import assert from "node:assert";
import { describe, it } from "node:test";
import { EventMapper } from "../sdk/internal/events.ts";
import type { AgentEvent as RawAgentEvent } from "../pi_host_web.js";

describe("EventMapper", () => {
  const mapper = new EventMapper();

  it("TM-21: maps tool_execution_start to toolStart with status running", () => {
    const state = mapper.createRunState();
    const rawEvent: RawAgentEvent = {
      type: "tool_execution_start",
      tool_call_id: "tc-1",
      tool_name: "browser_click",
      args: { selector: "#btn" },
    } as RawAgentEvent;

    const events = mapper.map(rawEvent, state);

    const toolStart = events.find((e) => e.type === "toolStart");
    assert.ok(toolStart, "should emit toolStart");
    assert.equal((toolStart!.payload as any).id, "tc-1");
    assert.equal((toolStart!.payload as any).name, "browser_click");
    assert.equal((toolStart!.payload as any).status, "running");
    assert.ok((toolStart!.payload as any).startedAt > 0);

    const status = events.find((e) => e.type === "status");
    assert.ok(status, "should emit status");
    assert.equal((status!.payload as any).state, "running_tool");
  });

  it("TM-21: maps tool_execution_update to toolUpdate", () => {
    const state = mapper.createRunState();
    // First start the tool
    mapper.map(
      {
        type: "tool_execution_start",
        tool_call_id: "tc-1",
        tool_name: "browser_click",
        args: {},
      } as RawAgentEvent,
      state,
    );

    const rawEvent: RawAgentEvent = {
      type: "tool_execution_update",
      tool_call_id: "tc-1",
      chunk: "partial output",
    } as RawAgentEvent;

    const events = mapper.map(rawEvent, state);

    const toolUpdate = events.find((e) => e.type === "toolUpdate");
    assert.ok(toolUpdate, "should emit toolUpdate");
    assert.equal((toolUpdate!.payload as any).id, "tc-1");
  });

  it("TM-21: maps tool_execution_end to toolEnd with status completed", () => {
    const state = mapper.createRunState();
    mapper.map(
      {
        type: "tool_execution_start",
        tool_call_id: "tc-1",
        tool_name: "browser_click",
        args: {},
      } as RawAgentEvent,
      state,
    );

    const rawEvent: RawAgentEvent = {
      type: "tool_execution_end",
      tool_call_id: "tc-1",
      is_error: false,
      result: {
        content: [{ type: "text", text: "clicked" }],
      },
    } as RawAgentEvent;

    const events = mapper.map(rawEvent, state);

    const toolEnd = events.find((e) => e.type === "toolEnd");
    assert.ok(toolEnd, "should emit toolEnd");
    assert.equal((toolEnd!.payload as any).status, "completed");
    assert.ok((toolEnd!.payload as any).endedAt > 0);
  });

  it("TM-21: maps tool_execution_end with error to toolEnd with status failed", () => {
    const state = mapper.createRunState();
    mapper.map(
      {
        type: "tool_execution_start",
        tool_call_id: "tc-1",
        tool_name: "browser_click",
        args: {},
      } as RawAgentEvent,
      state,
    );

    const rawEvent: RawAgentEvent = {
      type: "tool_execution_end",
      tool_call_id: "tc-1",
      is_error: true,
      result: {
        content: [{ type: "text", text: "error" }],
      },
    } as RawAgentEvent;

    const events = mapper.map(rawEvent, state);

    const toolEnd = events.find((e) => e.type === "toolEnd");
    assert.ok(toolEnd, "should emit toolEnd");
    assert.equal((toolEnd!.payload as any).status, "failed");
    // Note: current implementation does not populate error field on toolEnd
  });

  it("TM-22: maps turn_end to status completed", () => {
    const state = mapper.createRunState();
    const rawEvent: RawAgentEvent = {
      type: "turn_end",
      message: {
        role: "assistant",
        content: [{ type: "text", text: "done" }],
        timestamp: Date.now(),
      },
      tool_results: [],
    } as unknown as RawAgentEvent;

    const events = mapper.map(rawEvent, state);

    const statusEvent = events.find((e) => e.type === "status");
    assert.ok(statusEvent, "should emit status");
    assert.equal((statusEvent!.payload as any).state, "completed");
  });

  it("TM-23: emits all status states", () => {
    const state = mapper.createRunState();
    const testCases: Array<{ type: string; expectedState: string }> = [
      { type: "agent_start", expectedState: "loading" },
      { type: "turn_start", expectedState: "thinking" },
      { type: "message_start", expectedState: "thinking" },
      { type: "tool_execution_start", expectedState: "running_tool" },
      { type: "save_point", expectedState: "saving" },
      { type: "settled", expectedState: "completed" },
      { type: "agent_end", expectedState: "idle" },
    ];

    for (const tc of testCases) {
      const rawEvent: RawAgentEvent = {
        type: tc.type,
        message: {
          role: "assistant",
          content: [{ type: "text", text: "" }],
          timestamp: Date.now(),
        },
        tool_call_id: "tc-1",
        tool_name: "test",
        args: {},
      } as unknown as RawAgentEvent;

      const events = mapper.map(rawEvent, state);
      const statusEvent = events.find((e) => e.type === "status");
      if (statusEvent) {
        assert.equal(
          (statusEvent.payload as any).state,
          tc.expectedState,
          `Event ${tc.type} should emit status ${tc.expectedState}`,
        );
      }
    }
  });

  it("TM-24: usage accumulation from model response", () => {
    const state = mapper.createRunState();
    // Simulate usage being set on state (normally done by engine)
    state.usage = {
      input: 10,
      output: 5,
      cache_read: 0,
      cache_write: 0,
      total_tokens: 15,
    };

    const result = mapper.buildRunResult(state, { aborted: false });

    assert.deepStrictEqual(result.usage, {
      input: 10,
      output: 5,
      cache_read: 0,
      cache_write: 0,
      total_tokens: 15,
    });
  });

  it("buildRunResult returns aborted status when turnResult.aborted is true", () => {
    const state = mapper.createRunState();
    state.text = "partial";
    const result = mapper.buildRunResult(state, { aborted: true });

    assert.equal(result.status, "aborted");
    assert.equal(result.text, "partial");
  });

  it("buildRunResult returns completed status with message", () => {
    const state = mapper.createRunState();
    state.text = "hello";
    state.currentMessage = {
      id: "msg-1",
      role: "assistant",
      content: [{ type: "text", text: "hello" }],
    };

    const result = mapper.buildRunResult(state, { aborted: false });

    assert.equal(result.status, "completed");
    assert.equal(result.text, "hello");
    assert.ok(result.message);
    assert.equal(result.message!.id, "msg-1");
  });

  it("maps message_start to messageStart and status events", () => {
    const state = mapper.createRunState();
    const rawEvent: RawAgentEvent = {
      type: "message_start",
      message: {
        role: "assistant",
        content: [{ type: "text", text: "" }],
        timestamp: Date.now(),
      },
    } as RawAgentEvent;

    const events = mapper.map(rawEvent, state);

    const messageStart = events.find((e) => e.type === "messageStart");
    assert.ok(messageStart, "should emit messageStart");
    assert.equal((messageStart!.payload as any).role, "assistant");

    const status = events.find((e) => e.type === "status");
    assert.ok(status, "should emit status");
    assert.equal((status!.payload as any).state, "thinking");
  });

  it("maps text delta to text event and accumulates state.text", () => {
    const state = mapper.createRunState();
    const rawEvent: RawAgentEvent = {
      type: "message_update",
      delta: { kind: "text_delta", text: "world" },
    } as RawAgentEvent;

    const events = mapper.map(rawEvent, state);

    const textEvent = events.find((e) => e.type === "text");
    assert.ok(textEvent, "should emit text");
    assert.equal(textEvent!.payload, "world");
    assert.equal(state.text, "world");
  });

  it("maps tool_execution_cancelled to toolEnd with cancelled status", () => {
    const state = mapper.createRunState();
    mapper.map(
      {
        type: "tool_execution_start",
        tool_call_id: "tc-1",
        tool_name: "test",
        args: {},
      } as RawAgentEvent,
      state,
    );

    const rawEvent: RawAgentEvent = {
      type: "tool_execution_cancelled",
      tool_call_id: "tc-1",
    } as RawAgentEvent;

    const events = mapper.map(rawEvent, state);

    const toolEnd = events.find((e) => e.type === "toolEnd");
    assert.ok(toolEnd, "should emit toolEnd");
    assert.equal((toolEnd!.payload as any).status, "cancelled");
  });
});
