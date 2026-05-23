/**
 * Tests for the coding-agent tool contract.
 *
 * Covers:
 * - Tool schemas have required fields
 * - Path validation rules
 * - Each tool handler with valid and invalid inputs
 * - In-memory workspace operations
 * - Integration: fake agent loop reads and writes using typed tools
 * - Tool errors round-trip through Rust as is_error: true
 */

import { describe, it } from "node:test";
import assert from "node:assert/strict";

import { CODING_TOOLS, TOOL_NAMES } from "../src/tools/schemas.ts";
import { CodingToolRegistry, MemoryWorkspace } from "../src/tools/codingTools.ts";
import { validateWorkspacePath, PathError } from "../src/tools/path.ts";
import { AgentHost, defaultAgentOptions } from "../src/agentHost.ts";
import { FakeLlm } from "../src/fakeLlm.ts";

// --- Helpers ---

function call(registry: CodingToolRegistry, name: string, args: Record<string, unknown>): object {
  return registry.execute({
    id: "test-call",
    name,
    arguments: args,
  });
}

function seedWorkspace(): MemoryWorkspace {
  const ws = new MemoryWorkspace();
  ws.writeFile("src/main.rs", "fn main() { println!(\"hello\"); }");
  ws.writeFile("src/lib.rs", "pub fn add(a: i32, b: i32) -> i32 { a + b }");
  ws.writeFile("src/utils/mod.rs", "pub mod helpers;");
  ws.writeFile("src/utils/helpers.rs", "pub fn greet() -> String { \"hi\".into() }");
  ws.writeFile("README.md", "# My Project\n\nA test project.");
  ws.writeFile("Cargo.toml", "[package]\nname = \"test\"");
  return ws;
}

// --- Schema tests ---

describe("Tool schemas", () => {
  it("all coding tools have required fields", () => {
    for (const tool of CODING_TOOLS) {
      assert.ok(tool.name, "name must exist");
      assert.ok(tool.label, "label must exist");
      assert.ok(tool.description, "description must exist");
      assert.ok(tool.description.length >= 20, `description for ${tool.name} should be detailed`);
      assert.ok(tool.parameters, "parameters schema must exist");
      assert.ok(
        tool.execution_mode === "parallel" || tool.execution_mode === "sequential",
        "execution_mode must be parallel or sequential"
      );
    }
  });

  it("defines exactly the expected tools", () => {
    const names = CODING_TOOLS.map((t) => t.name).sort();
    assert.deepEqual(names, ["list_files", "read_file", "search_files", "write_file"]);
  });

  it("tool parameter schemas are valid JSON Schema objects", () => {
    for (const tool of CODING_TOOLS) {
      const schema = tool.parameters as Record<string, unknown>;
      assert.equal(schema.type, "object", `${tool.name} parameters must be type: object`);
      assert.ok(schema.properties, `${tool.name} must have properties`);
    }
  });
});

// --- Path validation tests ---

describe("Path validation", () => {
  it("accepts valid workspace-relative paths", () => {
    assert.equal(validateWorkspacePath("src/main.rs"), "src/main.rs");
    assert.equal(validateWorkspacePath("README.md"), "README.md");
    assert.equal(validateWorkspacePath("a/b/c.txt"), "a/b/c.txt");
  });

  it("rejects empty paths", () => {
    assert.throws(() => validateWorkspacePath(""), { code: "empty_path" });
  });

  it("rejects non-string paths", () => {
    assert.throws(() => validateWorkspacePath(42 as unknown as string), {
      code: "invalid_path_type",
    });
  });

  it("rejects absolute paths with leading slash", () => {
    assert.throws(() => validateWorkspacePath("/etc/passwd"), { code: "absolute_path" });
  });

  it("rejects Windows-style absolute paths (backslash check fires first)", () => {
    assert.throws(() => validateWorkspacePath("C:\\Users\\test"), { code: "backslash_path" });
  });

  it("rejects path traversal", () => {
    assert.throws(() => validateWorkspacePath("../etc/passwd"), { code: "path_traversal" });
    assert.throws(() => validateWorkspacePath("src/../../etc/passwd"), { code: "path_traversal" });
    assert.throws(() => validateWorkspacePath("src/../secret"), { code: "path_traversal" });
  });

  it("rejects paths with empty segments", () => {
    assert.throws(() => validateWorkspacePath("src//main.rs"), { code: "empty_segment" });
  });

  it("rejects backslash-only path separators", () => {
    assert.throws(() => validateWorkspacePath("src\\main.rs"), { code: "backslash_path" });
  });

  it("rejects backslash path traversal", () => {
    assert.throws(() => validateWorkspacePath("src\\..\\secret"), { code: "backslash_path" });
  });
});

// --- read_file tests ---

describe("read_file", () => {
  it("reads an existing file", () => {
    const ws = seedWorkspace();
    const reg = new CodingToolRegistry(ws);
    const result = call(reg, TOOL_NAMES.readFile, { path: "src/main.rs" }) as {
      content: { text: string }[];
    };
    assert.ok(!("error" in result), "should not be an error");
    assert.ok(result.content[0].text.includes("fn main()"));
  });

  it("returns error for missing file", () => {
    const ws = seedWorkspace();
    const reg = new CodingToolRegistry(ws);
    const result = call(reg, TOOL_NAMES.readFile, { path: "nonexistent.rs" }) as {
      error: { code: string; message: string };
    };
    assert.equal(result.error.code, "file_not_found");
    assert.ok(result.error.message.includes("nonexistent.rs"));
  });

  it("returns error for missing path argument", () => {
    const reg = new CodingToolRegistry();
    const result = call(reg, TOOL_NAMES.readFile, {}) as {
      error: { code: string };
    };
    assert.equal(result.error.code, "missing_argument");
  });

  it("returns error for invalid path", () => {
    const reg = new CodingToolRegistry();
    const result = call(reg, TOOL_NAMES.readFile, { path: "../etc/passwd" }) as {
      error: { code: string };
    };
    assert.equal(result.error.code, "path_traversal");
  });
});

// --- list_files tests ---

describe("list_files", () => {
  it("lists root directory", () => {
    const ws = seedWorkspace();
    const reg = new CodingToolRegistry(ws);
    const result = call(reg, TOOL_NAMES.listFiles, {}) as {
      content: { text: string }[];
    };
    const listing = result.content[0].text;
    assert.ok(listing.includes("src/"), "should list src directory");
    assert.ok(listing.includes("README.md"), "should list README.md");
    assert.ok(listing.includes("Cargo.toml"), "should list Cargo.toml");
  });

  it("lists a subdirectory", () => {
    const ws = seedWorkspace();
    const reg = new CodingToolRegistry(ws);
    const result = call(reg, TOOL_NAMES.listFiles, { path: "src" }) as {
      content: { text: string }[];
    };
    const listing = result.content[0].text;
    assert.ok(listing.includes("main.rs"));
    assert.ok(listing.includes("lib.rs"));
    assert.ok(listing.includes("utils/"));
  });

  it("shows empty directory message for empty dir", () => {
    const ws = seedWorkspace();
    const reg = new CodingToolRegistry(ws);
    const result = call(reg, TOOL_NAMES.listFiles, { path: "empty_dir" }) as {
      content: { text: string }[];
    };
    assert.equal(result.content[0].text, "(empty directory)");
  });

  it("rejects invalid path", () => {
    const reg = new CodingToolRegistry();
    const result = call(reg, TOOL_NAMES.listFiles, { path: "/absolute" }) as {
      error: { code: string };
    };
    assert.equal(result.error.code, "absolute_path");
  });
});

// --- search_files tests ---

describe("search_files", () => {
  it("finds files by pattern", () => {
    const ws = seedWorkspace();
    const reg = new CodingToolRegistry(ws);
    const result = call(reg, TOOL_NAMES.searchFiles, { pattern: "*.rs" }) as {
      content: { text: string }[];
    };
    const output = result.content[0].text;
    assert.ok(output.includes("src/main.rs"));
    assert.ok(output.includes("src/lib.rs"));
    assert.ok(output.includes("src/utils/helpers.rs"));
    assert.ok(!output.includes("README.md"));
  });

  it("finds files by substring", () => {
    const ws = seedWorkspace();
    const reg = new CodingToolRegistry(ws);
    const result = call(reg, TOOL_NAMES.searchFiles, { pattern: "helpers" }) as {
      content: { text: string }[];
    };
    assert.ok(result.content[0].text.includes("src/utils/helpers.rs"));
  });

  it("returns no matches message when nothing found", () => {
    const ws = seedWorkspace();
    const reg = new CodingToolRegistry(ws);
    const result = call(reg, TOOL_NAMES.searchFiles, { pattern: "nonexistent" }) as {
      content: { text: string }[];
    };
    assert.equal(result.content[0].text, "(no matches)");
  });

  it("returns error for missing pattern argument", () => {
    const reg = new CodingToolRegistry();
    const result = call(reg, TOOL_NAMES.searchFiles, {}) as {
      error: { code: string };
    };
    assert.equal(result.error.code, "missing_argument");
  });

  it("searches within a subdirectory", () => {
    const ws = seedWorkspace();
    const reg = new CodingToolRegistry(ws);
    const result = call(reg, TOOL_NAMES.searchFiles, { pattern: "*.rs", path: "src/utils" }) as {
      content: { text: string }[];
    };
    const output = result.content[0].text;
    assert.ok(output.includes("src/utils/helpers.rs"));
    assert.ok(!output.includes("src/main.rs"));
  });
});

// --- write_file tests ---

describe("write_file", () => {
  it("writes a new file", () => {
    const ws = new MemoryWorkspace();
    const reg = new CodingToolRegistry(ws);
    const result = call(reg, TOOL_NAMES.writeFile, {
      path: "src/new_file.rs",
      content: "pub fn new() {}",
    }) as { content: { text: string }[] };

    assert.ok(result.content[0].text.includes("wrote"));
    assert.equal(ws.readFile("src/new_file.rs"), "pub fn new() {}");
  });

  it("overwrites an existing file", () => {
    const ws = new MemoryWorkspace();
    ws.writeFile("test.txt", "old content");
    const reg = new CodingToolRegistry(ws);

    call(reg, TOOL_NAMES.writeFile, { path: "test.txt", content: "new content" });
    assert.equal(ws.readFile("test.txt"), "new content");
  });

  it("returns error for missing arguments", () => {
    const reg = new CodingToolRegistry();

    const result1 = call(reg, TOOL_NAMES.writeFile, { path: "test.txt" }) as {
      error: { code: string };
    };
    assert.equal(result1.error.code, "missing_argument");

    const result2 = call(reg, TOOL_NAMES.writeFile, { content: "test" }) as {
      error: { code: string };
    };
    assert.equal(result2.error.code, "missing_argument");
  });

  it("rejects invalid path", () => {
    const reg = new CodingToolRegistry();
    const result = call(reg, TOOL_NAMES.writeFile, { path: "../evil.sh", content: "bad" }) as {
      error: { code: string };
    };
    assert.equal(result.error.code, "path_traversal");
  });
});

// --- Unknown tool test ---

describe("Unknown tool handling", () => {
  it("returns error for unknown tool name", () => {
    const reg = new CodingToolRegistry();
    const result = call(reg, "explode_computer", {}) as {
      error: { code: string; message: string };
    };
    assert.equal(result.error.code, "unknown_tool");
    assert.ok(result.error.message.includes("explode_computer"));
  });
});

// --- Integration: fake agent loop with typed coding tools ---

describe("Integration: fake agent loop with coding tools", () => {
  it("reads and writes files through the full agent loop", () => {
    const ws = new MemoryWorkspace();
    ws.writeFile("greeting.txt", "Hello, World!");

    const reg = new CodingToolRegistry(ws);

    // LLM: first reads a file, then writes a modified version
    const llm = new FakeLlm([
      // First response: read the file
      {
        toolCalls: [
          {
            id: "call-1",
            name: "read_file",
            arguments: { path: "greeting.txt" },
          },
        ],
      },
      // Second response: write the modified file
      {
        toolCalls: [
          {
            id: "call-2",
            name: "write_file",
            arguments: { path: "greeting.txt", content: "Hello, Rust!" },
          },
        ],
      },
      // Third response: summary
      { text: "I've updated greeting.txt to say 'Hello, Rust!'." },
    ]);

    // CodingToolRegistry implements ToolRegistry, so it plugs directly into AgentHost.
    const host = new AgentHost(llm, reg);

    // Pass tool definitions so the Rust core knows about our tools
    const result = host.run(
      defaultAgentOptions({ tools: CODING_TOOLS }),
      "change greeting.txt to say Hello, Rust!"
    );

    assert.equal(result.terminalAction.type, "finished");

    // The workspace should have the updated content
    assert.equal(ws.readFile("greeting.txt"), "Hello, Rust!");

    // Three stream_llm actions (read, write, summary)
    assert.equal(
      result.trace.filter((e) => e.phase === "action" && e.type === "stream_llm").length,
      3
    );

    // Two execute_tools actions
    assert.equal(
      result.trace.filter((e) => e.phase === "action" && e.type === "execute_tools").length,
      2
    );

    // Tool execution events should show both tools
    const toolStartEvents = result.trace.filter(
      (e) => e.phase === "event" && e.type === "tool_execution_start"
    );
    assert.equal(toolStartEvents.length, 2);

    host.cleanup(result.handle);
  });

  it("tool errors round-trip through Rust as is_error: true", () => {
    const ws = new MemoryWorkspace();
    const reg = new CodingToolRegistry(ws);

    // LLM tries to read a non-existent file
    const llm = new FakeLlm([
      {
        toolCalls: [
          {
            id: "call-1",
            name: "read_file",
            arguments: { path: "missing.txt" },
          },
        ],
      },
      { text: "The file doesn't exist. I'll create it." },
    ]);

    const host = new AgentHost(llm, reg);

    const result = host.run(
      defaultAgentOptions({ tools: CODING_TOOLS }),
      "read missing.txt"
    );

    assert.equal(result.terminalAction.type, "finished");

    // The tool_execution_end event should have is_error: true
    const toolEndEvents = result.trace.filter(
      (e) => e.phase === "event" && e.type === "tool_execution_end"
    );
    assert.equal(toolEndEvents.length, 1);
    assert.equal((toolEndEvents[0].data as Record<string, unknown>).is_error, true);

    // The host trace should have a tool_done with the error payload
    const toolDoneEntries = result.trace.filter(
      (e) => e.phase === "host" && e.type === "tool_done"
    );
    assert.equal(toolDoneEntries.length, 1);
    const payload = toolDoneEntries[0].data as { payload: { error: { code: string } } };
    assert.equal(payload.payload.error.code, "file_not_found");

    host.cleanup(result.handle);
  });
});
