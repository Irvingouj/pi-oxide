/**
 * Unit tests for the Anthropic provider adapter.
 *
 * Tests message/tool conversion without hitting the network.
 */

import { describe, it } from "node:test";
import assert from "node:assert/strict";

import { convertMessages, convertTools, convertResponse } from "../src/providers/anthropic.ts";
import type { AgentMessageShape, ContentBlock } from "../src/providers/types.ts";
import { CODING_TOOLS } from "../src/tools/schemas.ts";

// --- Message conversion ---

describe("Anthropic message conversion", () => {
  it("converts a user text message", () => {
    const messages: AgentMessageShape[] = [
      {
        role: "user",
        content: [{ type: "text", text: "Hello" }],
        timestamp: 1,
      },
    ];
    const result = convertMessages(messages);
    assert.equal(result.length, 1);
    assert.equal(result[0].role, "user");
    assert.equal(result[0].content, "Hello");
  });

  it("converts an assistant text message", () => {
    const messages: AgentMessageShape[] = [
      {
        role: "assistant",
        content: [{ type: "text", text: "Hi there!" }],
        api: "test",
        provider: "test",
        model: "test-model",
        stop_reason: "end_turn",
        error_message: null,
        timestamp: 1,
        usage: { input: 10, output: 20, cache_read: 0, cache_write: 0, total_tokens: 30 },
      },
    ];
    const result = convertMessages(messages);
    assert.equal(result.length, 1);
    assert.equal(result[0].role, "assistant");
    const blocks = result[0].content as { type: string; text: string }[];
    assert.equal(blocks[0].type, "text");
    assert.equal(blocks[0].text, "Hi there!");
  });

  it("converts an assistant tool_use message", () => {
    const messages: AgentMessageShape[] = [
      {
        role: "assistant",
        content: [
          { type: "text", text: "Let me read that file." },
          { type: "tool_call", id: "call-1", name: "read_file", arguments: { path: "test.rs" } },
        ],
        api: "test",
        provider: "test",
        model: "test-model",
        stop_reason: "tool_use",
        error_message: null,
        timestamp: 1,
        usage: { input: 10, output: 20, cache_read: 0, cache_write: 0, total_tokens: 30 },
      },
    ];
    const result = convertMessages(messages);
    const blocks = result[0].content as { type: string; text?: string; id?: string; name?: string; input?: Record<string, unknown> }[];
    assert.equal(blocks.length, 2);
    assert.equal(blocks[0].type, "text");
    assert.equal(blocks[1].type, "tool_use");
    assert.equal(blocks[1].id, "call-1");
    assert.equal(blocks[1].name, "read_file");
    assert.deepEqual(blocks[1].input, { path: "test.rs" });
  });

  it("converts a tool_result message", () => {
    const messages: AgentMessageShape[] = [
      {
        role: "tool_result",
        tool_call_id: "call-1",
        tool_name: "read_file",
        content: [{ type: "text", text: "fn main() {}" }],
        details: null,
        is_error: false,
        timestamp: 1,
      },
    ];
    const result = convertMessages(messages);
    assert.equal(result.length, 1);
    assert.equal(result[0].role, "user");
    const blocks = result[0].content as { type: string; tool_use_id: string; content: string; is_error?: boolean }[];
    assert.equal(blocks[0].type, "tool_result");
    assert.equal(blocks[0].tool_use_id, "call-1");
    assert.equal(blocks[0].content, "fn main() {}");
    assert.equal(blocks[0].is_error, false);
  });

  it("converts a tool_result error", () => {
    const messages: AgentMessageShape[] = [
      {
        role: "tool_result",
        tool_call_id: "call-1",
        tool_name: "read_file",
        content: [{ type: "text", text: "file not found" }],
        details: null,
        is_error: true,
        timestamp: 1,
      },
    ];
    const result = convertMessages(messages);
    const blocks = result[0].content as { type: string; is_error: boolean }[];
    assert.equal(blocks[0].is_error, true);
  });

  /**
   * Anthropic requires that multiple tool_result messages responding to a
   * single assistant message with multiple tool_use blocks be merged into ONE
   * user message with an array of tool_result content blocks. Sending separate
   * consecutive user messages causes "Unexpected role change from user to user".
   *
   * See: https://docs.anthropic.com/en/docs/build-with-claude/tool-use
   */
  it("merges consecutive tool_result messages into a single Anthropic user message", () => {
    const messages: AgentMessageShape[] = [
      // Assistant with two tool_use blocks
      {
        role: "assistant",
        content: [
          { type: "text", text: "Reading both files." },
          { type: "tool_call", id: "call-1", name: "read_file", arguments: { path: "a.rs" } },
          { type: "tool_call", id: "call-2", name: "read_file", arguments: { path: "b.rs" } },
        ],
        api: "test",
        provider: "test",
        model: "test-model",
        stop_reason: "tool_use",
        error_message: null,
        timestamp: 1,
        usage: { input: 10, output: 20, cache_read: 0, cache_write: 0, total_tokens: 30 },
      },
      // First tool_result (from Rust core — one message per tool call)
      {
        role: "tool_result",
        tool_call_id: "call-1",
        tool_name: "read_file",
        content: [{ type: "text", text: "fn a() {}" }],
        details: null,
        is_error: false,
        timestamp: 2,
      },
      // Second tool_result (consecutive)
      {
        role: "tool_result",
        tool_call_id: "call-2",
        tool_name: "read_file",
        content: [{ type: "text", text: "fn b() {}" }],
        details: null,
        is_error: false,
        timestamp: 3,
      },
    ];

    const result = convertMessages(messages);

    // Should produce exactly 2 Anthropic messages: 1 assistant + 1 user
    assert.equal(result.length, 2, "should merge two tool_results into one user message");

    // First message: assistant with text + two tool_use blocks
    assert.equal(result[0].role, "assistant");
    const assistantBlocks = result[0].content as { type: string; id?: string }[];
    assert.equal(assistantBlocks.length, 3); // text + 2 tool_use
    assert.equal(assistantBlocks[1].type, "tool_use");
    assert.equal(assistantBlocks[1].id, "call-1");
    assert.equal(assistantBlocks[2].type, "tool_use");
    assert.equal(assistantBlocks[2].id, "call-2");

    // Second message: single user message with two tool_result blocks
    assert.equal(result[1].role, "user");
    const userBlocks = result[1].content as { type: string; tool_use_id: string; content: string; is_error: boolean }[];
    assert.equal(userBlocks.length, 2, "user message should contain two tool_result blocks");
    assert.equal(userBlocks[0].type, "tool_result");
    assert.equal(userBlocks[0].tool_use_id, "call-1");
    assert.equal(userBlocks[0].content, "fn a() {}");
    assert.equal(userBlocks[0].is_error, false);
    assert.equal(userBlocks[1].type, "tool_result");
    assert.equal(userBlocks[1].tool_use_id, "call-2");
    assert.equal(userBlocks[1].content, "fn b() {}");
    assert.equal(userBlocks[1].is_error, false);
  });

  it("merges consecutive tool_results even when some are errors", () => {
    const messages: AgentMessageShape[] = [
      {
        role: "assistant",
        content: [
          { type: "tool_call", id: "call-1", name: "read_file", arguments: { path: "exists.rs" } },
          { type: "tool_call", id: "call-2", name: "read_file", arguments: { path: "missing.rs" } },
        ],
        api: "test",
        provider: "test",
        model: "test-model",
        stop_reason: "tool_use",
        error_message: null,
        timestamp: 1,
        usage: { input: 10, output: 20, cache_read: 0, cache_write: 0, total_tokens: 30 },
      },
      {
        role: "tool_result",
        tool_call_id: "call-1",
        tool_name: "read_file",
        content: [{ type: "text", text: "fn exists() {}" }],
        details: null,
        is_error: false,
        timestamp: 2,
      },
      {
        role: "tool_result",
        tool_call_id: "call-2",
        tool_name: "read_file",
        content: [{ type: "text", text: "file not found: missing.rs" }],
        details: null,
        is_error: true,
        timestamp: 3,
      },
    ];

    const result = convertMessages(messages);
    assert.equal(result.length, 2, "assistant + single merged user message");

    const userBlocks = result[1].content as { type: string; tool_use_id: string; is_error: boolean }[];
    assert.equal(userBlocks.length, 2);
    assert.equal(userBlocks[0].is_error, false);
    assert.equal(userBlocks[1].is_error, true);
  });

  it("does not merge tool_results separated by a non-tool_result message", () => {
    const messages: AgentMessageShape[] = [
      {
        role: "tool_result",
        tool_call_id: "call-1",
        tool_name: "read_file",
        content: [{ type: "text", text: "fn a() {}" }],
        details: null,
        is_error: false,
        timestamp: 1,
      },
      // A user message in between breaks the merge
      {
        role: "user",
        content: [{ type: "text", text: "Now read another file" }],
        timestamp: 2,
      },
      {
        role: "tool_result",
        tool_call_id: "call-2",
        tool_name: "read_file",
        content: [{ type: "text", text: "fn b() {}" }],
        details: null,
        is_error: false,
        timestamp: 3,
      },
    ];

    const result = convertMessages(messages);
    // Should produce 3 separate messages: user (tool_result) + user (text) + user (tool_result)
    assert.equal(result.length, 3);
  });
});

// --- Tool conversion ---

describe("Anthropic tool conversion", () => {
  it("converts coding tool definitions", () => {
    const tools = convertTools(CODING_TOOLS);
    assert.equal(tools.length, 4);

    for (const tool of tools) {
      assert.ok(tool.name, "tool must have name");
      assert.ok(tool.description, "tool must have description");
      assert.ok(tool.input_schema, "tool must have input_schema");
      assert.equal((tool.input_schema as { type: string }).type, "object");
    }
  });

  it("tool names match expected", () => {
    const tools = convertTools(CODING_TOOLS);
    const names = tools.map((t) => t.name).sort();
    assert.deepEqual(names, ["list_files", "read_file", "search_files", "write_file"]);
  });
});

// --- Response conversion ---

describe("Anthropic response conversion", () => {
  it("converts a text-only response", () => {
    const resp = {
      id: "msg-1",
      type: "message" as const,
      role: "assistant" as const,
      content: [{ type: "text" as const, text: "Hello!" }],
      model: "test-model",
      stop_reason: "end_turn" as const,
      usage: { input_tokens: 10, output_tokens: 5 },
    };
    const { llmResult, chunks } = convertResponse(resp, "anthropic", "test-model");

    // llmResult should be { Ok: ... }
    const ok = (llmResult as { Ok: { content: ContentBlock[]; stop_reason: string } }).Ok;
    assert.ok(ok, "should be Ok variant");
    assert.equal(ok.stop_reason, "end_turn");
    assert.equal(ok.content[0].type, "text");
    assert.equal(ok.content[0].text, "Hello!");

    // Should have Start + TextDelta chunks
    assert.equal(chunks.length, 2);
    assert.equal((chunks[0] as { kind: string }).kind, "start");
    assert.equal((chunks[1] as { kind: string; text: string }).kind, "text_delta");
    assert.equal((chunks[1] as { kind: string; text: string }).text, "Hello!");
  });

  it("converts a tool_use response", () => {
    const resp = {
      id: "msg-2",
      type: "message" as const,
      role: "assistant" as const,
      content: [
        {
          type: "tool_use" as const,
          id: "call-1",
          name: "read_file",
          input: { path: "test.rs" },
        },
      ],
      model: "test-model",
      stop_reason: "tool_use" as const,
      usage: { input_tokens: 10, output_tokens: 20 },
    };
    const { llmResult } = convertResponse(resp, "anthropic", "test-model");

    const ok = (llmResult as { Ok: { content: ContentBlock[]; stop_reason: string } }).Ok;
    assert.equal(ok.stop_reason, "tool_use");
    assert.equal(ok.content[0].type, "tool_call");
    assert.equal(ok.content[0].id, "call-1");
    assert.equal(ok.content[0].name, "read_file");
  });
});
