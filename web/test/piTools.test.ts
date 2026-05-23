/**
 * Tests for the pi-compatible coding tool surface (Milestone 3.6).
 *
 * Covers:
 * - Schema validation for all seven pi-compatible tools
 * - Tool groups (PI_CODING_TOOLS, PI_READ_ONLY_TOOLS, PI_ALL_TOOLS)
 * - Individual tool handlers: read, write, edit, ls, grep, find, bash
 * - Error cases: missing args, invalid paths, edit not found, disallowed bash
 * - Deterministic full-loop programming smoke test
 */

import { describe, it } from "node:test";
import assert from "node:assert/strict";

import {
  PI_CODING_TOOLS,
  PI_READ_ONLY_TOOLS,
  PI_ALL_TOOLS,
  PI_TOOL_NAMES,
} from "../src/tools/schemas.ts";
import { CodingToolRegistry, MemoryWorkspace } from "../src/tools/codingTools.ts";
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
  ws.writeFile("src/main.ts", "function main() {\n  console.log('hello');\n}\nmain();\n");
  ws.writeFile("src/lib.ts", "export function add(a: number, b: number): number {\n  return a - b;\n}\n");
  ws.writeFile("src/utils.ts", "export function greet(name: string): string {\n  return 'Hello, ' + name;\n}\n");
  ws.writeFile("README.md", "# My Project\n\nA test project.");
  ws.writeFile("package.json", JSON.stringify({ name: "test-project", scripts: { test: "node --test" } }));
  return ws;
}

// ========================================================================
// Schema tests
// ========================================================================

describe("pi-compatible tool schemas", () => {
  it("all pi tools have required fields", () => {
    for (const tool of PI_ALL_TOOLS) {
      assert.ok(tool.name, `${tool.name} must have a name`);
      assert.ok(tool.label, `${tool.name} must have a label`);
      assert.ok(tool.description, `${tool.name} must have a description`);
      assert.ok(tool.description.length >= 20, `${tool.name} description should be detailed`);
      assert.ok(tool.parameters, `${tool.name} must have parameters`);
      assert.ok(
        tool.execution_mode === "parallel" || tool.execution_mode === "sequential",
        `${tool.name} execution_mode must be parallel or sequential`,
      );
    }
  });

  it("PI_ALL_TOOLS contains exactly seven tools", () => {
    assert.equal(PI_ALL_TOOLS.length, 7);
    const names = PI_ALL_TOOLS.map((t) => t.name).sort();
    assert.deepEqual(names, ["bash", "edit", "find", "grep", "ls", "read", "write"]);
  });

  it("PI_CODING_TOOLS contains read, bash, edit, write", () => {
    const names = PI_CODING_TOOLS.map((t) => t.name).sort();
    assert.deepEqual(names, ["bash", "edit", "read", "write"]);
  });

  it("PI_READ_ONLY_TOOLS contains read, grep, find, ls", () => {
    const names = PI_READ_ONLY_TOOLS.map((t) => t.name).sort();
    assert.deepEqual(names, ["find", "grep", "ls", "read"]);
  });

  it("tool parameter schemas are valid JSON Schema objects", () => {
    for (const tool of PI_ALL_TOOLS) {
      const schema = tool.parameters as Record<string, unknown>;
      assert.equal(schema.type, "object", `${tool.name} parameters must be type: object`);
      assert.ok(schema.properties, `${tool.name} must have properties`);
    }
  });
});

// ========================================================================
// read (pi-compatible)
// ========================================================================

describe("read (pi-compatible)", () => {
  it("reads an existing file", () => {
    const ws = seedWorkspace();
    const reg = new CodingToolRegistry(ws);
    const result = call(reg, PI_TOOL_NAMES.read, { path: "src/main.ts" }) as {
      content: { text: string }[];
    };
    assert.ok(!("error" in result), "should not be an error");
    assert.ok(result.content[0].text.includes("console.log"));
  });

  it("reads with offset (1-based)", () => {
    const ws = seedWorkspace();
    const reg = new CodingToolRegistry(ws);
    const result = call(reg, PI_TOOL_NAMES.read, { path: "src/main.ts", offset: 2 }) as {
      content: { text: string }[];
    };
    const text = result.content[0].text;
    assert.ok(text.includes("2:"), "should show line numbers");
    assert.ok(!text.includes("1:"), "should start at line 2");
  });

  it("reads with limit", () => {
    const ws = seedWorkspace();
    const reg = new CodingToolRegistry(ws);
    const result = call(reg, PI_TOOL_NAMES.read, { path: "src/main.ts", limit: 1 }) as {
      content: { text: string }[];
    };
    const text = result.content[0].text;
    assert.ok(text.includes("1:"), "should show first line");
    assert.ok(!text.includes("2:"), "should not show second line");
  });

  it("reads with offset and limit", () => {
    const ws = seedWorkspace();
    const reg = new CodingToolRegistry(ws);
    const result = call(reg, PI_TOOL_NAMES.read, { path: "src/main.ts", offset: 2, limit: 1 }) as {
      content: { text: string }[];
    };
    const text = result.content[0].text;
    assert.ok(text.includes("2:"), "should show line 2");
    assert.ok(!text.includes("1:"), "should not show line 1");
    assert.ok(!text.includes("3:"), "should not show line 3");
  });

  it("returns error for missing file", () => {
    const ws = seedWorkspace();
    const reg = new CodingToolRegistry(ws);
    const result = call(reg, PI_TOOL_NAMES.read, { path: "nonexistent.ts" }) as {
      error: { code: string };
    };
    assert.equal(result.error.code, "file_not_found");
  });

  it("returns error for missing path argument", () => {
    const reg = new CodingToolRegistry();
    const result = call(reg, PI_TOOL_NAMES.read, {}) as {
      error: { code: string };
    };
    assert.equal(result.error.code, "missing_argument");
  });

  it("rejects invalid path", () => {
    const reg = new CodingToolRegistry();
    const result = call(reg, PI_TOOL_NAMES.read, { path: "../etc/passwd" }) as {
      error: { code: string };
    };
    assert.equal(result.error.code, "path_traversal");
  });
});

// ========================================================================
// write (pi-compatible)
// ========================================================================

describe("write (pi-compatible)", () => {
  it("writes a new file", () => {
    const ws = new MemoryWorkspace();
    const reg = new CodingToolRegistry(ws);
    const result = call(reg, PI_TOOL_NAMES.write, {
      path: "src/new_file.ts",
      content: "export const x = 1;",
    }) as { content: { text: string }[] };

    assert.ok(result.content[0].text.includes("wrote"));
    assert.equal(ws.readFile("src/new_file.ts"), "export const x = 1;");
  });

  it("overwrites an existing file", () => {
    const ws = new MemoryWorkspace();
    ws.writeFile("test.txt", "old content");
    const reg = new CodingToolRegistry(ws);

    call(reg, PI_TOOL_NAMES.write, { path: "test.txt", content: "new content" });
    assert.equal(ws.readFile("test.txt"), "new content");
  });

  it("returns error for missing arguments", () => {
    const reg = new CodingToolRegistry();

    const r1 = call(reg, PI_TOOL_NAMES.write, { path: "test.txt" }) as { error: { code: string } };
    assert.equal(r1.error.code, "missing_argument");

    const r2 = call(reg, PI_TOOL_NAMES.write, { content: "test" }) as { error: { code: string } };
    assert.equal(r2.error.code, "missing_argument");
  });
});

// ========================================================================
// edit (pi-compatible)
// ========================================================================

describe("edit (pi-compatible)", () => {
  it("applies a single edit", () => {
    const ws = new MemoryWorkspace();
    ws.writeFile("src/calc.ts", "export function add(a, b) { return a - b; }");
    const reg = new CodingToolRegistry(ws);

    const result = call(reg, PI_TOOL_NAMES.edit, {
      path: "src/calc.ts",
      edits: [{ oldText: "return a - b;", newText: "return a + b;" }],
    }) as { content: { text: string }[] };

    assert.ok(result.content[0].text.includes("edited"));
    assert.equal(ws.readFile("src/calc.ts"), "export function add(a, b) { return a + b; }");
  });

  it("applies multiple edits sequentially", () => {
    const ws = new MemoryWorkspace();
    ws.writeFile("src/app.ts", "const x = 1;\nconst y = 2;\nconsole.log(x + y);");
    const reg = new CodingToolRegistry(ws);

    call(reg, PI_TOOL_NAMES.edit, {
      path: "src/app.ts",
      edits: [
        { oldText: "const x = 1;", newText: "const x = 10;" },
        { oldText: "const y = 2;", newText: "const y = 20;" },
      ],
    });

    const content = ws.readFile("src/app.ts");
    assert.ok(content!.includes("const x = 10;"));
    assert.ok(content!.includes("const y = 20;"));
  });

  it("returns error when oldText is not found", () => {
    const ws = new MemoryWorkspace();
    ws.writeFile("src/calc.ts", "return a + b;");
    const reg = new CodingToolRegistry(ws);

    const result = call(reg, PI_TOOL_NAMES.edit, {
      path: "src/calc.ts",
      edits: [{ oldText: "return a * b;", newText: "return a + b;" }],
    }) as { error: { code: string; message: string } };

    assert.equal(result.error.code, "edit_not_found");
    assert.ok(result.error.message.includes("return a * b;"));
  });

  it("returns error for missing file", () => {
    const reg = new CodingToolRegistry();
    const result = call(reg, PI_TOOL_NAMES.edit, {
      path: "nonexistent.ts",
      edits: [{ oldText: "x", newText: "y" }],
    }) as { error: { code: string } };
    assert.equal(result.error.code, "file_not_found");
  });

  it("returns error for empty edits array", () => {
    const ws = new MemoryWorkspace();
    ws.writeFile("test.txt", "hello");
    const reg = new CodingToolRegistry(ws);

    const result = call(reg, PI_TOOL_NAMES.edit, {
      path: "test.txt",
      edits: [],
    }) as { error: { code: string } };

    assert.equal(result.error.code, "missing_argument");
  });

  it("returns error for missing arguments", () => {
    const reg = new CodingToolRegistry();

    const r1 = call(reg, PI_TOOL_NAMES.edit, { path: "test.txt" }) as { error: { code: string } };
    assert.equal(r1.error.code, "missing_argument");

    const r2 = call(reg, PI_TOOL_NAMES.edit, { edits: [{ oldText: "x", newText: "y" }] }) as { error: { code: string } };
    assert.equal(r2.error.code, "missing_argument");
  });
});

// ========================================================================
// ls (pi-compatible)
// ========================================================================

describe("ls (pi-compatible)", () => {
  it("lists root directory", () => {
    const ws = seedWorkspace();
    const reg = new CodingToolRegistry(ws);
    const result = call(reg, PI_TOOL_NAMES.ls, {}) as {
      content: { text: string }[];
    };
    const listing = result.content[0].text;
    assert.ok(listing.includes("src/"));
    assert.ok(listing.includes("README.md"));
    assert.ok(listing.includes("package.json"));
  });

  it("lists a subdirectory", () => {
    const ws = seedWorkspace();
    const reg = new CodingToolRegistry(ws);
    const result = call(reg, PI_TOOL_NAMES.ls, { path: "src" }) as {
      content: { text: string }[];
    };
    const listing = result.content[0].text;
    assert.ok(listing.includes("main.ts"));
    assert.ok(listing.includes("lib.ts"));
    assert.ok(listing.includes("utils.ts"));
  });

  it("respects limit", () => {
    const ws = seedWorkspace();
    const reg = new CodingToolRegistry(ws);
    const result = call(reg, PI_TOOL_NAMES.ls, { limit: 2 }) as {
      content: { text: string }[];
    };
    const lines = result.content[0].text.split("\n");
    assert.equal(lines.length, 2);
  });

  it("shows empty directory message", () => {
    const ws = seedWorkspace();
    const reg = new CodingToolRegistry(ws);
    const result = call(reg, PI_TOOL_NAMES.ls, { path: "empty_dir" }) as {
      content: { text: string }[];
    };
    assert.equal(result.content[0].text, "(empty directory)");
  });
});

// ========================================================================
// grep (pi-compatible)
// ========================================================================

describe("grep (pi-compatible)", () => {
  it("finds matching lines", () => {
    const ws = seedWorkspace();
    const reg = new CodingToolRegistry(ws);
    const result = call(reg, PI_TOOL_NAMES.grep, { pattern: "console.log" }) as {
      content: { text: string }[];
    };
    const output = result.content[0].text;
    assert.ok(output.includes("console.log"));
    assert.ok(output.includes("src/main.ts"));
  });

  it("supports literal mode", () => {
    const ws = seedWorkspace();
    ws.writeFile("src/code.ts", "const x = a + b;\nconst y = a + b;\n");
    const reg = new CodingToolRegistry(ws);
    const result = call(reg, PI_TOOL_NAMES.grep, { pattern: "a + b", literal: true }) as {
      content: { text: string }[];
    };
    assert.ok(result.content[0].text.includes("a + b"));
  });

  it("supports glob filtering", () => {
    const ws = seedWorkspace();
    const reg = new CodingToolRegistry(ws);
    const result = call(reg, PI_TOOL_NAMES.grep, { pattern: "function", glob: "*.ts" }) as {
      content: { text: string }[];
    };
    const output = result.content[0].text;
    assert.ok(output.includes("src/main.ts"));
    assert.ok(!output.includes("README.md"));
  });

  it("searches within a specific directory path", () => {
    const ws = seedWorkspace();
    const reg = new CodingToolRegistry(ws);
    const result = call(reg, PI_TOOL_NAMES.grep, { pattern: "function", path: "src" }) as {
      content: { text: string }[];
    };
    const output = result.content[0].text;
    assert.ok(output.includes("src/lib.ts"));
    assert.ok(!output.includes("README.md"));
  });

  it("searches within an exact file path", () => {
    const ws = seedWorkspace();
    const reg = new CodingToolRegistry(ws);
    const result = call(reg, PI_TOOL_NAMES.grep, { pattern: "function", path: "src/main.ts" }) as {
      content: { text: string }[];
    };
    const output = result.content[0].text;
    assert.ok(output.includes("src/main.ts"), "should find match in the specified file");
    assert.ok(
      !output.includes("src/lib.ts") && !output.includes("src/utils.ts"),
      "should not include other files",
    );
  });

  it("exact file path grep does not search other files", () => {
    const ws = seedWorkspace();
    const reg = new CodingToolRegistry(ws);
    // "add" appears in lib.ts but not main.ts
    const result = call(reg, PI_TOOL_NAMES.grep, { pattern: "add", path: "src/main.ts" }) as {
      content: { text: string }[];
    };
    assert.equal(result.content[0].text, "(no matches)");
  });

  it("exact file path grep with no match returns no matches", () => {
    const ws = seedWorkspace();
    const reg = new CodingToolRegistry(ws);
    const result = call(reg, PI_TOOL_NAMES.grep, { pattern: "zzzznonexistent", path: "src/lib.ts" }) as {
      content: { text: string }[];
    };
    assert.equal(result.content[0].text, "(no matches)");
  });

  it("shows context lines", () => {
    const ws = seedWorkspace();
    const reg = new CodingToolRegistry(ws);
    const result = call(reg, PI_TOOL_NAMES.grep, { pattern: "console.log", context: 1 }) as {
      content: { text: string }[];
    };
    const output = result.content[0].text;
    // Should have context marker lines (space instead of >)
    assert.ok(output.includes(">"), "should mark match line with >");
  });

  it("respects limit", () => {
    const ws = seedWorkspace();
    const reg = new CodingToolRegistry(ws);
    const result = call(reg, PI_TOOL_NAMES.grep, { pattern: "function", limit: 1 }) as {
      content: { text: string }[];
    };
    const output = result.content[0].text;
    // Should only have one result line
    const matchLines = output.split("\n").filter((l: string) => l.includes(">"));
    assert.equal(matchLines.length, 1);
  });

  it("returns no matches message", () => {
    const ws = seedWorkspace();
    const reg = new CodingToolRegistry(ws);
    const result = call(reg, PI_TOOL_NAMES.grep, { pattern: "zzzznonexistent" }) as {
      content: { text: string }[];
    };
    assert.equal(result.content[0].text, "(no matches)");
  });

  it("returns error for missing pattern", () => {
    const reg = new CodingToolRegistry();
    const result = call(reg, PI_TOOL_NAMES.grep, {}) as { error: { code: string } };
    assert.equal(result.error.code, "missing_argument");
  });

  it("returns error for invalid regex", () => {
    const ws = seedWorkspace();
    const reg = new CodingToolRegistry(ws);
    const result = call(reg, PI_TOOL_NAMES.grep, { pattern: "[invalid" }) as {
      error: { code: string };
    };
    assert.equal(result.error.code, "invalid_pattern");
  });
});

// ========================================================================
// find (pi-compatible)
// ========================================================================

describe("find (pi-compatible)", () => {
  it("finds files by pattern", () => {
    const ws = seedWorkspace();
    const reg = new CodingToolRegistry(ws);
    const result = call(reg, PI_TOOL_NAMES.find, { pattern: "*.ts" }) as {
      content: { text: string }[];
    };
    const output = result.content[0].text;
    assert.ok(output.includes("src/main.ts"));
    assert.ok(output.includes("src/lib.ts"));
    assert.ok(!output.includes("README.md"));
  });

  it("finds files within a subdirectory", () => {
    const ws = seedWorkspace();
    const reg = new CodingToolRegistry(ws);
    const result = call(reg, PI_TOOL_NAMES.find, { pattern: "*.ts", path: "src" }) as {
      content: { text: string }[];
    };
    const output = result.content[0].text;
    assert.ok(output.includes("src/main.ts"));
    assert.ok(!output.includes("README.md"));
  });

  it("respects limit", () => {
    const ws = seedWorkspace();
    const reg = new CodingToolRegistry(ws);
    const result = call(reg, PI_TOOL_NAMES.find, { pattern: "*.ts", limit: 1 }) as {
      content: { text: string }[];
    };
    const lines = result.content[0].text.split("\n");
    assert.equal(lines.length, 1);
  });

  it("returns no matches message", () => {
    const ws = seedWorkspace();
    const reg = new CodingToolRegistry(ws);
    const result = call(reg, PI_TOOL_NAMES.find, { pattern: "*.xyz" }) as {
      content: { text: string }[];
    };
    assert.equal(result.content[0].text, "(no matches)");
  });

  it("returns error for missing pattern", () => {
    const reg = new CodingToolRegistry();
    const result = call(reg, PI_TOOL_NAMES.find, {}) as { error: { code: string } };
    assert.equal(result.error.code, "missing_argument");
  });
});

// ========================================================================
// bash (pi-compatible — constrained fake)
// ========================================================================

describe("bash (pi-compatible)", () => {
  it("simulates npm test with correct source", () => {
    const ws = new MemoryWorkspace();
    ws.writeFile("package.json", JSON.stringify({ scripts: { test: "node --test" } }));
    ws.writeFile("src/index.ts", "export function add(a: number, b: number): number {\n  return a + b;\n}\n");
    const reg = new CodingToolRegistry(ws);

    const result = call(reg, PI_TOOL_NAMES.bash, { command: "npm test" }) as {
      content: { text: string }[];
    };
    assert.ok(result.content[0].text.includes("pass 2"));
    assert.ok(result.content[0].text.includes("fail 0"));
  });

  it("simulates test failure with buggy source", () => {
    const ws = new MemoryWorkspace();
    ws.writeFile("package.json", JSON.stringify({ scripts: { test: "node --test" } }));
    ws.writeFile("src/index.ts", "export function add(a: number, b: number): number {\n  return a - b;\n}\n");
    const reg = new CodingToolRegistry(ws);

    const result = call(reg, PI_TOOL_NAMES.bash, { command: "npm test" }) as {
      content: { text: string }[];
      exitCode?: number;
    };
    assert.ok(result.content[0].text.includes("fail 1"));
    assert.equal(result.exitCode, 1);
  });

  it("simulates npm run build", () => {
    const ws = new MemoryWorkspace();
    const reg = new CodingToolRegistry(ws);

    const result = call(reg, PI_TOOL_NAMES.bash, { command: "npm run build" }) as {
      content: { text: string }[];
    };
    assert.ok(result.content[0].text.includes("Build completed"));
  });

  it("returns error for disallowed command", () => {
    const ws = new MemoryWorkspace();
    const reg = new CodingToolRegistry(ws);

    const result = call(reg, PI_TOOL_NAMES.bash, { command: "rm -rf /" }) as {
      error: { code: string; message: string };
    };
    assert.equal(result.error.code, "disallowed_command");
    assert.ok(result.error.message.includes("rm -rf /"));
  });

  it("returns error for missing command", () => {
    const reg = new CodingToolRegistry();
    const result = call(reg, PI_TOOL_NAMES.bash, {}) as { error: { code: string } };
    assert.equal(result.error.code, "missing_argument");
  });

  it("returns error for empty command", () => {
    const reg = new CodingToolRegistry();
    const result = call(reg, PI_TOOL_NAMES.bash, { command: "" }) as { error: { code: string } };
    assert.equal(result.error.code, "missing_argument");
  });
});

// ========================================================================
// Deterministic full-loop programming smoke test
// ========================================================================

describe("Deterministic programming smoke test", () => {
  it("agent reads buggy file, edits it, runs tests, and fixes it", () => {
    // Set up a buggy project
    const ws = new MemoryWorkspace();
    ws.writeFile("package.json", JSON.stringify({ name: "calc", scripts: { test: "node --test" } }));
    ws.writeFile(
      "src/index.ts",
      "export function add(a: number, b: number): number {\n  return a - b;\n}\n",
    );

    const reg = new CodingToolRegistry(ws);

    // Fake LLM simulates a coding agent that:
    // 1. Reads the buggy source file
    // 2. Edits the bug (a - b → a + b)
    // 3. Runs tests to verify
    // 4. Reports success
    const llm = new FakeLlm([
      // Step 1: read the file
      {
        toolCalls: [
          { id: "call-1", name: "read", arguments: { path: "src/index.ts" } },
        ],
      },
      // Step 2: edit the bug
      {
        toolCalls: [
          {
            id: "call-2",
            name: "edit",
            arguments: {
              path: "src/index.ts",
              edits: [{ oldText: "return a - b;", newText: "return a + b;" }],
            },
          },
        ],
      },
      // Step 3: run tests
      {
        toolCalls: [
          { id: "call-3", name: "bash", arguments: { command: "npm test" } },
        ],
      },
      // Step 4: final summary
      { text: "I fixed the bug. The add function now returns a + b and tests pass." },
    ]);

    const host = new AgentHost(llm, reg);

    const result = host.run(
      defaultAgentOptions({ tools: PI_CODING_TOOLS }),
      "Fix the bug in src/index.ts so that the add function works correctly",
    );

    // Terminal action is finished
    assert.equal(result.terminalAction.type, "finished");

    // Workspace contains the fixed code
    const finalContent = ws.readFile("src/index.ts");
    assert.ok(finalContent!.includes("return a + b;"), "workspace should contain the fix");
    assert.ok(!finalContent!.includes("return a - b;"), "workspace should not contain the bug");

    // Trace includes the expected tool sequence
    const toolActions = result.trace.filter(
      (e) => e.phase === "host" && e.type === "tool_done",
    );
    assert.equal(toolActions.length, 3, "should have 3 tool executions");

    const toolNames = toolActions.map(
      (e) => (e.data as { tool_name: string }).tool_name,
    );
    assert.deepEqual(toolNames, ["read", "edit", "bash"]);

    // Four stream_llm actions (read, edit, bash, summary)
    assert.equal(
      result.trace.filter((e) => e.phase === "action" && e.type === "stream_llm").length,
      4,
    );

    host.cleanup(result.handle);
  });
});
