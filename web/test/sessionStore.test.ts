/**
 * Tests for local session store and file artifact store (Milestone 7).
 */

import { describe, it } from "node:test";
import assert from "node:assert/strict";
import * as fs from "node:fs";
import * as path from "node:path";
import * as os from "node:os";

import {
  LocalSessionStore,
  loadSession,
  reconstructMessages,
  SessionCorruptError,
} from "../src/local/sessionStore.ts";
import { FileArtifactStore } from "../src/local/fileArtifactStore.ts";

// --- Helpers ---

function withTempDir(fn: (dir: string) => Promise<void> | void): Promise<void> {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "pi-oxide-session-"));
  const result = fn(dir);
  if (result instanceof Promise) {
    return result.finally(() => fs.rmSync(dir, { recursive: true, force: true }));
  }
  fs.rmSync(dir, { recursive: true, force: true });
  return Promise.resolve();
}

// ========================================================================
// Session Store
// ========================================================================

describe("LocalSessionStore", () => {
  it("creates session directory and metadata", () => {
    return withTempDir((base) => {
      const dir = path.join(base, "my-session");
      const store = new LocalSessionStore(dir, {
        session_id: "sess-1",
        cwd: "/tmp/project",
        model: "test-model",
      });

      // Directory and files should exist
      assert.ok(fs.existsSync(dir));
      assert.ok(fs.existsSync(path.join(dir, "session.jsonl")));
      assert.ok(fs.existsSync(path.join(dir, "artifacts")));

      // Metadata should be correct
      const meta = store.getMetadata();
      assert.equal(meta.session_id, "sess-1");
      assert.equal(meta.cwd, "/tmp/project");
      assert.equal(meta.model, "test-model");
      assert.ok(meta.created_at > 0);
      assert.ok(meta.updated_at >= meta.created_at);

      store.close();
    });
  });

  it("appends entries without rewriting old entries", () => {
    return withTempDir((base) => {
      const dir = path.join(base, "append-test");
      const store = new LocalSessionStore(dir, {
        session_id: "sess-2",
        cwd: "/tmp",
        model: "test",
      });

      const sizeAfterStart = fs.statSync(path.join(dir, "session.jsonl")).size;

      store.append("user_prompt", { text: "hello" });
      const sizeAfterPrompt = fs.statSync(path.join(dir, "session.jsonl")).size;
      assert.ok(sizeAfterPrompt > sizeAfterStart, "file should grow after append");

      store.append("tool_call", { tool_call_id: "tc-1", tool_name: "bash" });
      const sizeAfterToolCall = fs.statSync(path.join(dir, "session.jsonl")).size;
      assert.ok(sizeAfterToolCall > sizeAfterPrompt, "file should keep growing");

      // Sequence numbers should be strictly increasing
      assert.equal(store.getSeq(), 3); // session_start(seq=1) + user_prompt(seq=2) + tool_call(seq=3)

      store.close();

      // Verify the file has the right number of lines
      const content = fs.readFileSync(path.join(dir, "session.jsonl"), "utf-8");
      const lines = content.trim().split("\n");
      // session_start + user_prompt + tool_call + session_end
      assert.equal(lines.length, 4);
    });
  });

  it("loading session reconstructs entries in order", () => {
    return withTempDir((base) => {
      const dir = path.join(base, "load-test");
      const store = new LocalSessionStore(dir, {
        session_id: "sess-3",
        cwd: "/tmp/project",
        model: "test-model",
      });

      store.append("user_prompt", { role: "user", content: [{ type: "text", text: "read file" }] });
      store.append("tool_call", { tool_call_id: "tc-1", tool_name: "read" });
      store.append("tool_result", { tool_call_id: "tc-1", content: "file content" });
      store.append("assistant_message", { role: "assistant", content: [{ type: "text", text: "done" }] });
      store.close();

      // Load back
      const loaded = loadSession(dir);

      // Metadata
      assert.equal(loaded.metadata.session_id, "sess-3");
      assert.equal(loaded.metadata.cwd, "/tmp/project");
      assert.equal(loaded.metadata.model, "test-model");

      // Entries in order
      assert.ok(loaded.entries.length >= 6, `expected >= 6 entries, got ${loaded.entries.length}`);

      const kinds = loaded.entries.map((e) => e.kind);
      assert.equal(kinds[0], "session_start");
      assert.ok(kinds.includes("user_prompt"));
      assert.ok(kinds.includes("tool_call"));
      assert.ok(kinds.includes("tool_result"));
      assert.ok(kinds.includes("assistant_message"));
      assert.equal(kinds[kinds.length - 1], "session_end");

      // Sequence numbers strictly increasing
      for (let i = 1; i < loaded.entries.length; i++) {
        assert.ok(
          loaded.entries[i].seq > loaded.entries[i - 1].seq,
          `seq ${loaded.entries[i].seq} should be > ${loaded.entries[i - 1].seq}`,
        );
      }
    });
  });

  it("corrupt JSONL returns useful typed error", () => {
    return withTempDir((base) => {
      const dir = path.join(base, "corrupt-test");
      fs.mkdirSync(dir, { recursive: true });

      // Write a corrupt JSONL file
      const sessionFile = path.join(dir, "session.jsonl");
      fs.writeFileSync(sessionFile, [
        JSON.stringify({ seq: 1, kind: "session_start", timestamp: 1, data: { session_id: "s", cwd: "/", model: "m" } }),
        "THIS IS NOT JSON",
        JSON.stringify({ seq: 3, kind: "user_prompt", timestamp: 3, data: {} }),
      ].join("\n") + "\n");

      assert.throws(() => loadSession(dir), (err: unknown) => {
        assert.ok(err instanceof SessionCorruptError);
        assert.equal(err.line, 2);
        assert.ok(err.message.includes("invalid JSON"));
        return true;
      });
    });
  });

  it("missing session_start returns typed error", () => {
    return withTempDir((base) => {
      const dir = path.join(base, "no-start-test");
      fs.mkdirSync(dir, { recursive: true });

      const sessionFile = path.join(dir, "session.jsonl");
      fs.writeFileSync(sessionFile, JSON.stringify({ seq: 1, kind: "user_prompt", timestamp: 1, data: {} }) + "\n");

      assert.throws(() => loadSession(dir), (err: unknown) => {
        assert.ok(err instanceof SessionCorruptError);
        assert.ok(err.message.includes("no session_start"));
        return true;
      });
    });
  });

  it("reconstructMessages extracts messages for AgentOptions", () => {
    return withTempDir((base) => {
      const dir = path.join(base, "reconstruct-test");
      const store = new LocalSessionStore(dir, {
        session_id: "sess-recon",
        cwd: "/tmp",
        model: "test",
      });

      store.append("user_prompt", { role: "user", content: [{ type: "text", text: "hello" }], timestamp: 1 });
      store.append("assistant_message", { role: "assistant", content: [{ type: "text", text: "hi" }], timestamp: 2 });
      store.append("tool_result", { role: "tool_result", tool_call_id: "tc-1", content: "result" });
      store.close();

      const loaded = loadSession(dir);
      const messages = reconstructMessages(loaded.entries);

      assert.ok(messages.length >= 3, `expected >= 3 messages, got ${messages.length}`);

      // First message should be user prompt
      const first = messages[0] as Record<string, unknown>;
      assert.equal(first.role, "user");
    });
  });
});

// ========================================================================
// File Artifact Store
// ========================================================================

describe("FileArtifactStore", () => {
  it("writes and reads content by stable id", () => {
    return withTempDir((base) => {
      const store = new FileArtifactStore(path.join(base, "artifacts"));

      store.put({
        id: "tool-result-tc-1",
        toolName: "read",
        toolCallId: "tc-1",
        content: "file contents here",
        storedAt: Date.now(),
      });

      const record = store.get("tool-result-tc-1");
      assert.ok(record, "artifact should exist");
      assert.equal(record!.content, "file contents here");
      assert.equal(record!.id, "tool-result-tc-1");
    });
  });

  it("stores metadata including tool name, tool call id, byte length", () => {
    return withTempDir((base) => {
      const store = new FileArtifactStore(path.join(base, "artifacts"));

      const now = Date.now();
      store.put({
        id: "tool-result-tc-42",
        toolName: "bash",
        toolCallId: "tc-42",
        content: "A".repeat(1000),
        storedAt: now,
      });

      const meta = store.readMeta("tool-result-tc-42");
      assert.ok(meta, "metadata should exist");
      assert.equal(meta!.toolName, "bash");
      assert.equal(meta!.toolCallId, "tc-42");
      assert.equal(meta!.byteLength, 1000);
      assert.equal(meta!.createdAt, now);

      // Check files exist
      assert.ok(fs.existsSync(path.join(base, "artifacts", "tool-result-tc-42.txt")));
      assert.ok(fs.existsSync(path.join(base, "artifacts", "tool-result-tc-42.meta.json")));
    });
  });

  it("returns undefined for missing artifact", () => {
    return withTempDir((base) => {
      const store = new FileArtifactStore(path.join(base, "artifacts"));
      assert.equal(store.get("nonexistent"), undefined);
    });
  });

  it("lists stored artifact ids", () => {
    return withTempDir((base) => {
      const store = new FileArtifactStore(path.join(base, "artifacts"));

      store.put({ id: "art-1", toolName: "read", toolCallId: "tc-1", content: "a", storedAt: 1 });
      store.put({ id: "art-2", toolName: "bash", toolCallId: "tc-2", content: "b", storedAt: 2 });

      const ids = store.list();
      assert.ok(ids.includes("art-1"));
      assert.ok(ids.includes("art-2"));
      assert.equal(ids.length, 2);
    });
  });

  it("has() checks artifact existence", () => {
    return withTempDir((base) => {
      const store = new FileArtifactStore(path.join(base, "artifacts"));
      assert.ok(!store.has("missing"));

      store.put({ id: "exists-1", toolName: "read", toolCallId: "tc-1", content: "x", storedAt: 1 });
      assert.ok(store.has("exists-1"));
    });
  });
});
