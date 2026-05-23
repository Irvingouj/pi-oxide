/**
 * Tests for local machine host tools (Milestone 4).
 *
 * Covers:
 * - Path boundary validation
 * - read: existing file, offset/limit, missing file, directory read
 * - write: creates parent dirs, writes file, rejects outside cwd
 * - edit: exact replacement, missing oldText, ambiguous oldText
 * - bash: allowed command, disallowed command, timeout
 * - LocalToolRegistry: unknown tool
 */

import { describe, it } from "node:test";
import assert from "node:assert/strict";
import * as fs from "node:fs";
import * as path from "node:path";
import * as os from "node:os";

import { resolveLocalPath } from "../src/local/path.ts";
import { LocalToolRegistry } from "../src/local/localToolRegistry.ts";

// --- Helpers ---

function call(registry: LocalToolRegistry, name: string, args: Record<string, unknown>): object {
  return registry.execute({
    id: "test-call",
    name,
    arguments: args,
  });
}

function withCwd(fn: (d: string) => void): void {
  const d = fs.mkdtempSync(path.join(os.tmpdir(), "pi-oxide-local-"));
  try {
    fn(d);
  } finally {
    fs.rmSync(d, { recursive: true, force: true });
  }
}

// ========================================================================
// Path boundary
// ========================================================================

describe("Local path boundary", () => {
  it("accepts path inside cwd", () => {
    withCwd((d) => {
      const resolved = resolveLocalPath(d, "src/main.ts");
      assert.equal(resolved, path.resolve(d, "src/main.ts"));
    });
  });

  it("rejects traversal outside cwd", () => {
    withCwd((d) => {
      assert.throws(() => resolveLocalPath(d, "../etc/passwd"), { code: "path_traversal" });
    });
  });

  it("rejects absolute path outside cwd", () => {
    withCwd((d) => {
      assert.throws(() => resolveLocalPath(d, "/etc/passwd"), { code: "absolute_path" });
    });
  });

  it("rejects empty path", () => {
    withCwd((d) => {
      assert.throws(() => resolveLocalPath(d, ""), { code: "empty_path" });
    });
  });

  it("rejects non-string path", () => {
    withCwd((d) => {
      assert.throws(() => resolveLocalPath(d, 42 as unknown as string), { code: "missing_path" });
    });
  });
});

// ========================================================================
// read
// ========================================================================

describe("Local read", () => {
  it("reads an existing file", () => {
    withCwd((d) => {
      fs.mkdirSync(path.join(d, "src"), { recursive: true });
      fs.writeFileSync(path.join(d, "src", "hello.txt"), "Hello, World!");
      const reg = new LocalToolRegistry({ cwd: d });
      const result = call(reg, "read", { path: "src/hello.txt" }) as {
        content: { text: string }[];
      };
      assert.ok(!("error" in result));
      assert.ok(result.content[0].text.includes("Hello, World!"));
    });
  });

  it("reads with offset and limit", () => {
    withCwd((d) => {
      fs.writeFileSync(path.join(d, "lines.txt"), "line1\nline2\nline3\nline4\nline5\n");
      const reg = new LocalToolRegistry({ cwd: d });
      const result = call(reg, "read", { path: "lines.txt", offset: 2, limit: 2 }) as {
        content: { text: string }[];
      };
      const text = result.content[0].text;
      assert.ok(text.includes("2: line2"));
      assert.ok(text.includes("3: line3"));
      assert.ok(!text.includes("1:"));
      assert.ok(!text.includes("4:"));
    });
  });

  it("returns error for missing file", () => {
    withCwd((d) => {
      const reg = new LocalToolRegistry({ cwd: d });
      const result = call(reg, "read", { path: "nonexistent.txt" }) as {
        error: { code: string };
      };
      assert.equal(result.error.code, "file_not_found");
    });
  });

  it("returns error for missing path argument", () => {
    withCwd((d) => {
      const reg = new LocalToolRegistry({ cwd: d });
      const result = call(reg, "read", {}) as { error: { code: string } };
      assert.equal(result.error.code, "missing_path");
    });
  });

  it("returns error for directory path", () => {
    withCwd((d) => {
      fs.mkdirSync(path.join(d, "src"), { recursive: true });
      const reg = new LocalToolRegistry({ cwd: d });
      const result = call(reg, "read", { path: "src" }) as {
        error: { code: string };
      };
      assert.equal(result.error.code, "is_directory");
    });
  });
});

// ========================================================================
// write
// ========================================================================

describe("Local write", () => {
  it("creates parent directory and file", () => {
    withCwd((d) => {
      const reg = new LocalToolRegistry({ cwd: d });
      const result = call(reg, "write", {
        path: "src/deep/new_file.txt",
        content: "created!",
      }) as { content: { text: string }[] };

      assert.ok(result.content[0].text.includes("wrote"));
      assert.equal(
        fs.readFileSync(path.join(d, "src", "deep", "new_file.txt"), "utf-8"),
        "created!",
      );
    });
  });

  it("overwrites existing file", () => {
    withCwd((d) => {
      fs.writeFileSync(path.join(d, "test.txt"), "old");
      const reg = new LocalToolRegistry({ cwd: d });
      call(reg, "write", { path: "test.txt", content: "new" });
      assert.equal(fs.readFileSync(path.join(d, "test.txt"), "utf-8"), "new");
    });
  });

  it("rejects outside cwd", () => {
    withCwd((d) => {
      const reg = new LocalToolRegistry({ cwd: d });
      const result = call(reg, "write", { path: "../evil.sh", content: "bad" }) as {
        error: { code: string };
      };
      assert.equal(result.error.code, "path_traversal");
    });
  });

  it("returns error for missing content", () => {
    withCwd((d) => {
      const reg = new LocalToolRegistry({ cwd: d });
      const result = call(reg, "write", { path: "test.txt" }) as {
        error: { code: string };
      };
      assert.equal(result.error.code, "missing_content");
    });
  });
});

// ========================================================================
// edit
// ========================================================================

describe("Local edit", () => {
  it("applies exact edit", () => {
    withCwd((d) => {
      fs.writeFileSync(path.join(d, "calc.ts"), "return a - b;");
      const reg = new LocalToolRegistry({ cwd: d });
      const result = call(reg, "edit", {
        path: "calc.ts",
        edits: [{ oldText: "return a - b;", newText: "return a + b;" }],
      }) as { content: { text: string }[]; details: { edits: number; diff: string } };

      assert.ok(result.content[0].text.includes("edited"));
      assert.equal(result.details.edits, 1);
      assert.ok(result.details.diff.includes("-return a - b;"));
      assert.ok(result.details.diff.includes("+return a + b;"));
      assert.equal(
        fs.readFileSync(path.join(d, "calc.ts"), "utf-8"),
        "return a + b;",
      );
    });
  });

  it("rejects missing oldText", () => {
    withCwd((d) => {
      fs.writeFileSync(path.join(d, "calc.ts"), "return a + b;");
      const reg = new LocalToolRegistry({ cwd: d });
      const result = call(reg, "edit", {
        path: "calc.ts",
        edits: [{ oldText: "return a * b;", newText: "return a + b;" }],
      }) as { error: { code: string } };
      assert.equal(result.error.code, "edit_not_found");
    });
  });

  it("rejects empty oldText", () => {
    withCwd((d) => {
      fs.writeFileSync(path.join(d, "test.txt"), "hello");
      const reg = new LocalToolRegistry({ cwd: d });
      const result = call(reg, "edit", {
        path: "test.txt",
        edits: [{ oldText: "", newText: "world" }],
      }) as { error: { code: string } };
      assert.equal(result.error.code, "empty_oldText");
    });
  });

  it("rejects ambiguous duplicate oldText", () => {
    withCwd((d) => {
      fs.writeFileSync(path.join(d, "dup.ts"), "x = 1;\nx = 1;\n");
      const reg = new LocalToolRegistry({ cwd: d });
      const result = call(reg, "edit", {
        path: "dup.ts",
        edits: [{ oldText: "x = 1;", newText: "x = 2;" }],
      }) as { error: { code: string } };
      assert.equal(result.error.code, "ambiguous_edit");
    });
  });

  it("rejects empty edits array", () => {
    withCwd((d) => {
      fs.writeFileSync(path.join(d, "test.txt"), "hello");
      const reg = new LocalToolRegistry({ cwd: d });
      const result = call(reg, "edit", { path: "test.txt", edits: [] }) as {
        error: { code: string };
      };
      assert.equal(result.error.code, "missing_edits");
    });
  });

  it("rejects missing file", () => {
    withCwd((d) => {
      const reg = new LocalToolRegistry({ cwd: d });
      const result = call(reg, "edit", {
        path: "nonexistent.ts",
        edits: [{ oldText: "x", newText: "y" }],
      }) as { error: { code: string } };
      assert.equal(result.error.code, "file_not_found");
    });
  });
});

// ========================================================================
// bash
// ========================================================================

describe("Local bash", () => {
  it("unrestricted command succeeds", () => {
    withCwd((d) => {
      const reg = new LocalToolRegistry({
        cwd: d,
        bashPolicy: { mode: "unrestricted" },
      });
      const result = call(reg, "bash", { command: "echo hello" }) as {
        content: { text: string }[];
      };
      assert.ok(result.content[0].text.includes("hello"));
    });
  });

  it("deny mode rejects command with typed error", () => {
    withCwd((d) => {
      const reg = new LocalToolRegistry({
        cwd: d,
        bashPolicy: { mode: "deny" },
      });
      const result = call(reg, "bash", { command: "echo hello" }) as {
        error: { code: string; message: string };
      };
      assert.equal(result.error.code, "disallowed_command");
      assert.ok(result.error.message.includes("deny"));
    });
  });

  it("timeout returns typed error", () => {
    withCwd((d) => {
      const reg = new LocalToolRegistry({
        cwd: d,
        bashPolicy: { mode: "unrestricted" },
      });
      const result = call(reg, "bash", { command: "sleep 10", timeout: 500 }) as {
        error: { code: string };
      };
      assert.equal(result.error.code, "timeout");
    });
  });

  it("returns error for missing command", () => {
    withCwd((d) => {
      const reg = new LocalToolRegistry({ cwd: d });
      const result = call(reg, "bash", {}) as { error: { code: string } };
      assert.equal(result.error.code, "missing_command");
    });
  });

  it("returns error for empty command", () => {
    withCwd((d) => {
      const reg = new LocalToolRegistry({ cwd: d });
      const result = call(reg, "bash", { command: "" }) as { error: { code: string } };
      assert.equal(result.error.code, "empty_command");
    });
  });

  it("captures stdout and exit code for failing command", () => {
    withCwd((d) => {
      fs.writeFileSync(path.join(d, "fail.js"), "process.stdout.write('out\\n'); process.exit(1);");
      const reg = new LocalToolRegistry({
        cwd: d,
        bashPolicy: { mode: "unrestricted" },
      });
      const result = call(reg, "bash", {
        command: "node fail.js",
      }) as { content: { text: string }[]; exitCode: number; details: { exitCode: number } };
      assert.ok(result.content[0].text.includes("out"));
      assert.equal(result.details.exitCode, 1);
    });
  });
});

// ========================================================================
// LocalToolRegistry unknown tool
// ========================================================================

describe("LocalToolRegistry unknown tool", () => {
  it("returns error for unknown tool name", () => {
    withCwd((d) => {
      const reg = new LocalToolRegistry({ cwd: d });
      const result = call(reg, "explode_computer", {}) as {
        error: { code: string; message: string };
      };
      assert.equal(result.error.code, "unknown_tool");
      assert.ok(result.error.message.includes("explode_computer"));
    });
  });
});
