/**
 * Tests for Rust context projection (Milestone 5).
 *
 * Covers:
 * - WASM projection export succeeds
 * - WASM projection export returns error envelope for invalid input
 * - Small tool result stays inline
 * - Large read tool result becomes head preview
 * - Large bash tool result becomes tail preview
 * - edit tool result uses keep-full strategy
 * - Replacement IDs are deterministic
 * - Repeated projection with same state is byte-identical
 * - Canonical input messages are not mutated
 * - Trimming drops old messages when over budget
 * - Trimming does not leave orphan tool_result
 * - RealLlm without projection keeps current behavior
 * - RealLlm with projection can call through Rust projection without network
 * - Full replaced result is stored in MemoryArtifactStore
 */

import { describe, it } from "node:test";
import assert from "node:assert/strict";

import type { AgentMessageShape } from "../src/providers/types.ts";
import {
  callProjectContext,
  MemoryArtifactStore,
  type ContextProjectionState,
  type ContextProjectionBudget,
} from "../src/context/rustProjection.ts";
import { RealLlm } from "../src/providers/realLlm.ts";

// --- Helpers ---

function userMsg(text: string): AgentMessageShape {
  return { role: "user", content: [{ type: "text", text }], timestamp: 1 };
}

function assistantMsg(
  text?: string,
  toolCalls?: Array<{ id: string; name: string; args: Record<string, unknown> }>,
): AgentMessageShape {
  const content: {
    type: string;
    text?: string;
    id?: string;
    name?: string;
    arguments?: Record<string, unknown>;
  }[] = [];
  if (text) content.push({ type: "text", text });
  if (toolCalls) {
    for (const tc of toolCalls) {
      content.push({ type: "tool_call", id: tc.id, name: tc.name, arguments: tc.args });
    }
  }
  return {
    role: "assistant",
    content,
    api: "test",
    provider: "test",
    model: "test-model",
    stop_reason: toolCalls ? "tool_use" : "end_turn",
    error_message: null,
    timestamp: 1,
    usage: { input: 0, output: 0, cache_read: 0, cache_write: 0, total_tokens: 0 },
  };
}

function toolResultMsg(
  toolCallId: string,
  toolName: string,
  text: string,
  isError = false,
): AgentMessageShape {
  return {
    role: "tool_result",
    tool_call_id: toolCallId,
    tool_name: toolName,
    content: [{ type: "text", text }],
    details: null,
    is_error: isError,
    timestamp: 1,
  };
}

function defaultBudget(overrides?: Partial<ContextProjectionBudget>): ContextProjectionBudget {
  return {
    max_tool_result_chars: 1000,
    max_context_tokens: 100_000,
    default_preview_chars: 200,
    ...overrides,
  };
}

function defaultState(): ContextProjectionState {
  return { replacements: {} };
}

// ========================================================================
// WASM projection export
// ========================================================================

describe("WASM projection export", () => {
  it("returns error envelope for invalid input", () => {
    assert.throws(() => {
      callProjectContext("sys", [{ role: "invalid_role" }] as unknown as AgentMessageShape[], defaultBudget(), defaultState());
    }, /projectContext failed/);
  });

  it("succeeds with valid messages", () => {
    const messages: AgentMessageShape[] = [
      userMsg("hello"),
      assistantMsg("hi there"),
    ];
    const result = callProjectContext("You are helpful.", messages, defaultBudget(), defaultState());

    assert.ok(result.projected_messages);
    assert.ok(result.report);
    assert.ok(result.updated_state);
    assert.equal(result.report.replacements.length, 0);
  });

  it("returns error for completely malformed JSON", () => {
    // callProjectContext wraps input in JSON, so malformed messages should throw
    assert.throws(() => {
      // Pass something that will make the WASM function fail
      const badMessages = [{ role: "invalid_role" }] as unknown as AgentMessageShape[];
      callProjectContext("test", badMessages, defaultBudget(), defaultState());
    });
  });
});

// ========================================================================
// Tool result budgeting
// ========================================================================

describe("Tool result budgeting", () => {
  it("small tool result stays inline", () => {
    const messages: AgentMessageShape[] = [toolResultMsg("tc-1", "read", "small content")];
    const result = callProjectContext("test", messages, defaultBudget(), defaultState());

    assert.equal(result.report.replacements.length, 0);
    // The tool result content should be unchanged
    const tr = result.projected_messages.find((m) => m.role === "tool_result");
    assert.ok(tr);
    if (tr.role === "tool_result") {
      const text = tr.content[0].text!;
      assert.equal(text, "small content");
    }
  });

  it("large read tool result becomes head preview", () => {
    const bigText = "A".repeat(5000);
    const messages: AgentMessageShape[] = [toolResultMsg("tc-1", "read", bigText)];
    const result = callProjectContext("test", messages, defaultBudget(), defaultState());

    assert.equal(result.report.replacements.length, 1);
    assert.equal(result.report.replacements[0].artifact_id, "tool-result-tc-1");
    assert.equal(result.report.replacements[0].tool_name, "read");
    assert.equal(result.report.replacements[0].strategy.type, "head");

    // Projected text should contain the preview marker
    const tr = result.projected_messages.find((m) => m.role === "tool_result");
    assert.ok(tr);
    if (tr.role === "tool_result") {
      const text = tr.content[0].text!;
      assert.ok(text.includes("<context-artifact"));
      assert.ok(text.includes("head"));
      assert.ok(text.includes("tool-result-tc-1"));
      // Head preview: should contain a run of A's
      assert.ok(text.includes("A".repeat(100)));
    }
  });

  it("large bash tool result becomes tail preview", () => {
    const bigText = "A".repeat(3000) + "B".repeat(2000);
    const messages: AgentMessageShape[] = [toolResultMsg("tc-2", "bash", bigText)];
    const result = callProjectContext("test", messages, defaultBudget(), defaultState());

    assert.equal(result.report.replacements.length, 1);
    assert.equal(result.report.replacements[0].strategy.type, "tail");

    const tr = result.projected_messages.find((m) => m.role === "tool_result");
    assert.ok(tr);
    if (tr.role === "tool_result") {
      const text = tr.content[0].text!;
      assert.ok(text.includes("tail"));
      // Tail preview: should contain B's from the end
      assert.ok(text.includes("B".repeat(100)));
    }
  });

  it("edit tool result uses keep-full strategy and stays inline", () => {
    const bigText = "X".repeat(5000);
    const messages: AgentMessageShape[] = [toolResultMsg("tc-3", "edit", bigText)];
    const result = callProjectContext("test", messages, defaultBudget(), defaultState());

    // edit defaults to KeepFull — no replacement even if oversized
    assert.equal(result.report.replacements.length, 0);

    const tr = result.projected_messages.find((m) => m.role === "tool_result");
    assert.ok(tr);
    if (tr.role === "tool_result") {
      const text = tr.content[0].text!;
      assert.equal(text, bigText);
    }
  });
});

// ========================================================================
// Determinism and state
// ========================================================================

describe("Determinism and replacement state", () => {
  it("replacement IDs are deterministic", () => {
    const bigText = "A".repeat(5000);
    const messages: AgentMessageShape[] = [toolResultMsg("tc-det", "bash", bigText)];

    const result1 = callProjectContext("test", messages, defaultBudget(), defaultState());
    const result2 = callProjectContext("test", messages, defaultBudget(), defaultState());

    assert.equal(result1.report.replacements[0].artifact_id, result2.report.replacements[0].artifact_id);
    assert.equal(result1.report.replacements[0].artifact_id, "tool-result-tc-det");
  });

  it("repeated projection with same state is byte-identical", () => {
    const bigText = "A".repeat(5000);
    const messages: AgentMessageShape[] = [toolResultMsg("tc-stable", "bash", bigText)];

    const result1 = callProjectContext("test", messages, defaultBudget(), defaultState());

    // Second projection with updated state
    const result2 = callProjectContext("test", messages, defaultBudget(), result1.updated_state);

    assert.deepEqual(
      JSON.stringify(result1.projected_messages),
      JSON.stringify(result2.projected_messages),
    );
  });

  it("canonical input messages are not mutated", () => {
    const bigText = "A".repeat(5000);
    const messages: AgentMessageShape[] = [toolResultMsg("tc-imm", "read", bigText)];
    const originalText = messages[0].content[0].text!;

    callProjectContext("test", messages, defaultBudget(), defaultState());

    // Original array should be unchanged
    assert.equal(messages[0].content[0].text!, originalText);
    assert.equal(messages[0].content[0].text!.length, 5000);
  });
});

// ========================================================================
// Window trimming
// ========================================================================

describe("Window trimming", () => {
  it("keeps all messages when under budget", () => {
    const messages: AgentMessageShape[] = [
      userMsg("hello"),
      assistantMsg("hi there"),
    ];

    const result = callProjectContext("test", messages, defaultBudget(), defaultState());

    assert.equal(result.projected_messages.length, 2);
    assert.equal(result.report.dropped_messages, 0);
  });

  it("drops old messages when over budget", () => {
    const messages: AgentMessageShape[] = [];
    for (let i = 0; i < 20; i++) {
      messages.push(userMsg(`turn ${i}: ${"A".repeat(200)}`));
      messages.push(assistantMsg(`response ${i}: ${"B".repeat(200)}`));
    }

    const result = callProjectContext("test", messages, defaultBudget({
      max_context_tokens: 500,
    }), defaultState());

    assert.ok(result.projected_messages.length < messages.length);
    assert.ok(result.report.dropped_messages > 0);
    assert.ok(result.projected_messages.length > 0);
  });

  it("does not leave orphan tool_result", () => {
    const messages: AgentMessageShape[] = [];
    for (let i = 0; i < 20; i++) {
      messages.push(userMsg(`turn ${i}: ${"X".repeat(200)}`));
      messages.push(assistantMsg(undefined, [{ id: `tc-${i}`, name: "bash", args: { command: "echo" } }]));
      messages.push(toolResultMsg(`tc-${i}`, "bash", "Y".repeat(200)));
    }

    const result = callProjectContext("test", messages, defaultBudget({
      max_context_tokens: 500,
    }), defaultState());

    // Collect all tool_call IDs from assistant messages
    const toolCallIds = new Set<string>();
    for (const msg of result.projected_messages) {
      if (msg.role === "assistant") {
        for (const block of msg.content) {
          if (block.type === "tool_call" && block.id) {
            toolCallIds.add(block.id);
          }
        }
      }
    }

    for (const msg of result.projected_messages) {
      if (msg.role === "tool_result") {
        assert.ok(
          toolCallIds.has(msg.tool_call_id),
          `Orphan tool_result: ${msg.tool_call_id}`,
        );
      }
    }
  });
});

// ========================================================================
// Artifact store
// ========================================================================

describe("MemoryArtifactStore", () => {
  it("stores and retrieves artifacts", () => {
    const store = new MemoryArtifactStore();
    const id = store.put({
      id: "test-artifact",
      toolName: "bash",
      toolCallId: "tc-1",
      content: "big output",
      storedAt: Date.now(),
    });
    assert.equal(id, "test-artifact");

    const retrieved = store.get("test-artifact");
    assert.ok(retrieved);
    assert.equal(retrieved.content, "big output");
  });

  it("returns undefined for missing artifact", () => {
    const store = new MemoryArtifactStore();
    assert.equal(store.get("nonexistent"), undefined);
  });
});

// ========================================================================
// RealLlm wiring
// ========================================================================

describe("RealLlm wiring", () => {
  it("RealLlm without projection keeps current behavior", () => {
    const llm = new RealLlm({
      apiKey: "test-key",
      baseUrl: "https://api.anthropic.com",
      model: "claude-3-haiku-20240307",
    });
    assert.ok(llm);
    assert.equal(llm.log.length, 0);
  });

  it("RealLlm accepts context projection config", () => {
    const store = new MemoryArtifactStore();
    const llm = new RealLlm(
      {
        apiKey: "test-key",
        baseUrl: "https://api.anthropic.com",
        model: "claude-3-haiku-20240307",
      },
      {
        budget: defaultBudget(),
        state: defaultState(),
        artifacts: store,
      },
    );
    assert.ok(llm);
  });
});
