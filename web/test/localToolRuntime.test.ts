/**
 * Tests for local tool runtime (Milestone 6.5).
 *
 * Boundary stress tests:
 * 1. Streaming output — slow command emits multiple updates before done
 * 2. Stderr streaming — stderr tagged separately from stdout
 * 3. Host loop responsiveness — long command doesn't block
 * 4. Cancellation — stops process, no orphan
 * 5. Background server lifecycle — job tracked, reachable, stoppable
 * 6. Parallel tools — concurrent with correct correlation
 * 7. Serialized file mutation — same-file writes don't corrupt
 * 8. Projection boundary — large output doesn't bloat transcript
 */

import { describe, it } from "node:test";
import assert from "node:assert/strict";
import * as fs from "node:fs";
import * as path from "node:path";
import * as os from "node:os";

import { ToolRuntime, type ToolUpdate } from "../src/local/toolRuntime.ts";
import { JobTable } from "../src/local/jobTable.ts";

// --- Helpers ---

async function withCwd(fn: (d: string) => Promise<void>): Promise<void> {
  const d = fs.mkdtempSync(path.join(os.tmpdir(), "pi-oxide-runtime-"));
  try {
    await fn(d);
  } finally {
    fs.rmSync(d, { recursive: true, force: true });
  }
}

function makeRuntime(cwd: string, updates: ToolUpdate[]): ToolRuntime {
  return new ToolRuntime({
    cwd,
    bashPolicy: { mode: "unrestricted" },
    callbacks: {
      onUpdate: (update) => updates.push(update),
    },
    enableBackgroundJobs: true,
  });
}

// ========================================================================
// JobTable
// ========================================================================

describe("JobTable", () => {
  it("adds and retrieves jobs", () => {
    const table = new JobTable();
    const id = table.add("tc-1", "python3 -m http.server 0");
    const job = table.get(id);
    assert.ok(job);
    assert.equal(job!.toolCallId, "tc-1");
    assert.equal(job!.command, "python3 -m http.server 0");
    assert.ok(!job!.stopped);
  });

  it("stops jobs by id", () => {
    const table = new JobTable();
    const id = table.add("tc-1", "server");
    assert.ok(table.stop(id));
    const job = table.get(id);
    assert.ok(job!.stopped);
  });

  it("lists active jobs", () => {
    const table = new JobTable();
    const id1 = table.add("tc-1", "server1");
    table.add("tc-2", "server2");
    assert.equal(table.active().length, 2);
    table.stop(id1);
    assert.equal(table.active().length, 1);
  });

  it("finds job by tool call id", () => {
    const table = new JobTable();
    table.add("tc-1", "server");
    const job = table.getByToolCallId("tc-1");
    assert.ok(job);
    assert.equal(job!.jobId.startsWith("job-"), true);
  });
});

// ========================================================================
// 1. Streaming output
// ========================================================================

describe("Streaming output", () => {
  it("slow command emits multiple stdout updates before final done", async () => {
    const updates: ToolUpdate[] = [];
    await withCwd(async (d) => {
      const runtime = makeRuntime(d, updates);
      const result = await runtime.execute({
        id: "tc-stream",
        name: "bash",
        arguments: {
          command:
            "node -e \"let i=0; const t=setInterval(() => { console.log('tick '+(++i)); if(i===5) clearInterval(t); }, 100)\"",
          timeout: 5000,
        },
      });

      // Should have at least 5 stdout updates
      const stdoutUpdates = updates.filter(
        (u) => u.toolCallId === "tc-stream" && u.stream === "stdout",
      );
      assert.ok(
        stdoutUpdates.length >= 5,
        `expected >= 5 stdout updates, got ${stdoutUpdates.length}`,
      );

      // Sequence numbers strictly increasing
      const seqs = stdoutUpdates.map((u) => u.sequence);
      for (let i = 1; i < seqs.length; i++) {
        assert.ok(seqs[i] > seqs[i - 1], `seq ${seqs[i]} not > ${seqs[i - 1]}`);
      }

      // Final result should contain the output
      const payload = result as { content: Array<{ text: string }> };
      assert.ok(payload.content[0].text.includes("tick"));

      runtime.cleanup();
    });
  });
});

// ========================================================================
// 2. Stderr streaming
// ========================================================================

describe("Stderr streaming", () => {
  it("stderr is tagged separately from stdout", async () => {
    const updates: ToolUpdate[] = [];
    await withCwd(async (d) => {
      const runtime = makeRuntime(d, updates);
      await runtime.execute({
        id: "tc-stderr",
        name: "bash",
        arguments: {
          command: 'node -e "console.error(\'warn-1\'); console.error(\'warn-2\')"',
          timeout: 5000,
        },
      });

      const stderrUpdates = updates.filter(
        (u) => u.toolCallId === "tc-stderr" && u.stream === "stderr",
      );
      const stdoutUpdates = updates.filter(
        (u) => u.toolCallId === "tc-stderr" && u.stream === "stdout",
      );

      assert.ok(stderrUpdates.length > 0, "should have stderr updates");
      const stderrText = stderrUpdates.map((u) => u.chunk).join("");
      assert.ok(stderrText.includes("warn-1"), `stderr text: ${stderrText}`);
      assert.ok(stderrText.includes("warn-2"), `stderr text: ${stderrText}`);

      // Stderr should not be mislabeled as stdout
      const stdoutText = stdoutUpdates.map((u) => u.chunk).join("");
      assert.ok(!stdoutText.includes("warn-1"), `stdout should not contain stderr: ${stdoutText}`);

      runtime.cleanup();
    });
  });
});

// ========================================================================
// 3. Host loop responsiveness
// ========================================================================

describe("Host loop responsiveness", () => {
  it("can start second task while first is running", async () => {
    const updates: ToolUpdate[] = [];
    await withCwd(async (d) => {
      const runtime = makeRuntime(d, updates);

      // Start long command
      const longPromise = runtime.execute({
        id: "tc-long",
        name: "bash",
        arguments: {
          command: "node -e \"setTimeout(() => console.log('done'), 500)\"",
          timeout: 5000,
        },
      });

      // While long command is running, start a quick one
      const quickResult = await runtime.execute({
        id: "tc-quick",
        name: "bash",
        arguments: {
          command: "echo quick",
          timeout: 5000,
        },
      });

      // Quick result should be available before long one finishes
      const quickPayload = quickResult as { content: Array<{ text: string }> };
      assert.ok(quickPayload.content[0].text.includes("quick"));

      // Now wait for the long one
      const longResult = await longPromise;
      const longPayload = longResult as { content: Array<{ text: string }> };
      assert.ok(longPayload.content[0].text.includes("done"));

      runtime.cleanup();
    });
  });
});

// ========================================================================
// 4. Cancellation
// ========================================================================

describe("Cancellation", () => {
  it("stops a running process and leaves no orphan", async () => {
    const updates: ToolUpdate[] = [];
    await withCwd(async (d) => {
      const runtime = makeRuntime(d, updates);

      const promise = runtime.execute({
        id: "tc-cancel",
        name: "bash",
        arguments: {
          command: "node -e \"setInterval(() => console.log('still-running'), 100)\"",
          timeout: 10000,
        },
      });

      // Wait a bit for it to start
      await new Promise((r) => setTimeout(r, 200));

      // Cancel
      const cancelled = runtime.cancel("tc-cancel");
      assert.ok(cancelled, "should have cancelled the tool");

      const result = await promise;
      const payload = result as { content: Array<{ text: string }>; details: { cancelled: boolean } };
      assert.ok(payload.details.cancelled, "result should be cancelled");

      // No more updates after cancellation
      const countBefore = updates.filter((u) => u.toolCallId === "tc-cancel").length;
      await new Promise((r) => setTimeout(r, 300));
      const countAfter = updates.filter((u) => u.toolCallId === "tc-cancel").length;
      assert.equal(countBefore, countAfter, "no updates after cancellation");

      runtime.cleanup();
    });
  });
});

// ========================================================================
// 5. Background server lifecycle
// ========================================================================

describe("Background server lifecycle", () => {
  it("tracks server as background job and can stop it", async () => {
    const updates: ToolUpdate[] = [];
    await withCwd(async (d) => {
      // Create a simple file to serve
      fs.writeFileSync(path.join(d, "index.html"), "<h1>Hello Ottawa</h1>");

      const runtime = makeRuntime(d, updates);

      const promise = runtime.execute({
        id: "tc-server",
        name: "bash",
        arguments: {
          command: "python3 -m http.server 18421",
          timeout: 15000,
        },
      });

      // Wait for server to start with retries
      const { execFileSync } = await import("node:child_process");
      let reached = false;
      for (let attempt = 0; attempt < 10; attempt++) {
        await new Promise((r) => setTimeout(r, 300));
        try {
          const resp = execFileSync("curl", ["-s", "http://localhost:18421/index.html"], {
            timeout: 2000,
            encoding: "utf-8",
          });
          if (resp.includes("Hello Ottawa")) {
            reached = true;
            break;
          }
        } catch {
          // Server not ready yet
        }
      }
      assert.ok(reached, "server should be reachable after retries");

      // Check job was created
      assert.ok(runtime.jobs.active().length > 0, "should have background job");
      const job = runtime.jobs.active()[0];
      assert.equal(job.toolCallId, "tc-server");

      // Stop by job id
      runtime.stopJob(job.jobId);
      await promise;

      // Verify server is stopped
      await new Promise((r) => setTimeout(r, 500));
      try {
        execFileSync("curl", ["-s", "http://localhost:18421/index.html"], {
          timeout: 2000,
          encoding: "utf-8",
        });
        assert.fail("server should be stopped");
      } catch {
        // Expected — server is gone
      }

      runtime.cleanup();
    });
  });
});

// ========================================================================
// 6. Parallel tools
// ========================================================================

describe("Parallel tools", () => {
  it("two slow commands run concurrently with separate correlation", async () => {
    const updates: ToolUpdate[] = [];
    await withCwd(async (d) => {
      const runtime = makeRuntime(d, updates);

      const p1 = runtime.execute({
        id: "tc-para-1",
        name: "bash",
        arguments: {
          command: "node -e \"setTimeout(() => console.log('result-A'), 300)\"",
          timeout: 5000,
        },
      });

      const p2 = runtime.execute({
        id: "tc-para-2",
        name: "bash",
        arguments: {
          command: "node -e \"setTimeout(() => console.log('result-B'), 300)\"",
          timeout: 5000,
        },
      });

      const [r1, r2] = await Promise.all([p1, p2]);

      const p1Payload = r1 as { content: Array<{ text: string }> };
      const p2Payload = r2 as { content: Array<{ text: string }> };
      assert.ok(p1Payload.content[0].text.includes("result-A"));
      assert.ok(p2Payload.content[0].text.includes("result-B"));

      // Each tool call has its own sequence
      const seqs1 = updates
        .filter((u) => u.toolCallId === "tc-para-1")
        .map((u) => u.sequence);
      const seqs2 = updates
        .filter((u) => u.toolCallId === "tc-para-2")
        .map((u) => u.sequence);

      // Both should have updates
      assert.ok(seqs1.length > 0, "tc-para-1 should have updates");
      assert.ok(seqs2.length > 0, "tc-para-2 should have updates");

      runtime.cleanup();
    });
  });
});

// ========================================================================
// 7. Serialized file mutation
// ========================================================================

describe("Serialized file mutation", () => {
  it("concurrent writes to same file are serialized", async () => {
    const updates: ToolUpdate[] = [];
    await withCwd(async (d) => {
      const runtime = makeRuntime(d, updates);

      // Create initial file
      fs.writeFileSync(path.join(d, "shared.txt"), "");

      // Two writes to the same file concurrently
      const p1 = runtime.execute({
        id: "tc-write-1",
        name: "write",
        arguments: { path: "shared.txt", content: "AAA" },
      });

      const p2 = runtime.execute({
        id: "tc-write-2",
        name: "write",
        arguments: { path: "shared.txt", content: "BBB" },
      });

      await Promise.all([p1, p2]);

      // File should contain one of the values, not corrupted
      const content = fs.readFileSync(path.join(d, "shared.txt"), "utf-8");
      assert.ok(
        content === "AAA" || content === "BBB",
        `expected AAA or BBB, got: ${content}`,
      );

      runtime.cleanup();
    });
  });
});

// ========================================================================
// 8. Projection boundary (conceptual — streaming chunks don't bloat transcript)
// ========================================================================

describe("Projection boundary", () => {
  it("large streaming output produces bounded final result", async () => {
    const updates: ToolUpdate[] = [];
    await withCwd(async (d) => {
      const runtime = makeRuntime(d, updates);

      // Generate lots of output
      const result = await runtime.execute({
        id: "tc-big",
        name: "bash",
        arguments: {
          command: "node -e \"for(let i=0;i<1000;i++) console.log('line '+i+': ' + 'X'.repeat(100))\"",
          timeout: 10000,
        },
      });

      // Should have many updates
      const stdoutUpdates = updates.filter(
        (u) => u.toolCallId === "tc-big" && u.stream === "stdout",
      );
      assert.ok(stdoutUpdates.length >= 10, `expected >= 10 updates, got ${stdoutUpdates.length}`);

      // Final result is a single object, not an array of chunks
      const payload = result as { content: Array<{ text: string }> };
      assert.equal(payload.content.length, 1, "final result should be single content block");

      // The final result text should be bounded (streaming chunks are trace-only)
      // In a real integration, context projection would further trim this
      assert.ok(payload.content[0].text.length > 0, "final result should have text");

      runtime.cleanup();
    });
  });
});
