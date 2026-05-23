/**
 * Local async tool runtime.
 *
 * Manages tool execution lifecycle:
 * - async bash with streaming stdout/stderr
 * - cancellation by tool call id
 * - background job table for long-running commands
 * - file mutations remain serialized per path
 * - cleanup on host shutdown
 *
 * JS host owns all process runtime. Rust core sees only typed callbacks.
 */

import type { ChildProcess } from "node:child_process";
import type { ToolCall } from "../wasmBinding.ts";
import { startStreamingBash, type StreamingBashHandle, type StreamingBashResult } from "./streamingBashTool.ts";
import { handleLocalRead, handleLocalWrite, handleLocalEdit } from "./fileTools.ts";
import { JobTable } from "./jobTable.ts";
import type { BashPolicy } from "./bashTool.ts";
import { resolveLocalPath } from "./path.ts";

// --- Types ---

export interface ToolUpdate {
  toolCallId: string;
  stream: "stdout" | "stderr" | "status";
  chunk: string;
  sequence: number;
}

export interface ToolRuntimeCallbacks {
  onUpdate: (update: ToolUpdate) => void;
}

interface RunningTool {
  toolCallId: string;
  toolName: string;
  handle?: StreamingBashHandle;
}

// --- File mutation queue ---

class PathMutex {
  private readonly locks = new Map<string, Promise<void>>();

  async runExclusive<T>(path: string, fn: () => T): Promise<T> {
    const prev = this.locks.get(path) ?? Promise.resolve();
    let resolve: () => void;
    const next = new Promise<void>((r) => { resolve = r; });
    this.locks.set(path, next);
    await prev;
    try {
      return fn();
    } finally {
      resolve!();
      if (this.locks.get(path) === next) {
        this.locks.delete(path);
      }
    }
  }
}

// --- Tool Runtime ---

export interface ToolRuntimeOptions {
  cwd: string;
  bashPolicy: BashPolicy;
  callbacks: ToolRuntimeCallbacks;
  /** If true, commands like "python3 -m http.server" are tracked as background jobs */
  enableBackgroundJobs?: boolean;
}

export class ToolRuntime {
  readonly cwd: string;
  readonly bashPolicy: BashPolicy;
  private readonly callbacks: ToolRuntimeCallbacks;
  private readonly runningTools = new Map<string, RunningTool>();
  private readonly sequences = new Map<string, number>();
  private readonly pathMutex = new PathMutex();
  private readonly backgroundJobs: JobTable;
  private readonly enableBackgroundJobs: boolean;

  /**
   * Additional update listener set by the host to intercept streaming
   * updates (e.g. to forward them to Rust/WASM). If set, it is called
   * in addition to the construction-time callbacks.
   */
  hostUpdateListener?: (update: ToolUpdate) => void;

  constructor(options: ToolRuntimeOptions) {
    this.cwd = options.cwd;
    this.bashPolicy = options.bashPolicy;
    this.callbacks = options.callbacks;
    this.backgroundJobs = new JobTable();
    this.enableBackgroundJobs = options.enableBackgroundJobs ?? false;
  }

  /**
   * Start executing a tool call asynchronously.
   *
   * Returns a promise that resolves with the final tool result payload
   * (compatible with onToolDone). Updates are emitted via callbacks.
   */
  async execute(call: ToolCall): Promise<object> {
    const args = (call.arguments ?? {}) as Record<string, unknown>;
    const toolCallId = call.id;

    switch (call.name) {
      case "bash":
        return this.executeBash(toolCallId, args);
      case "read":
        return this.executeFileSynced(toolCallId, "read", args, () =>
          handleLocalRead(args, this.cwd),
        );
      case "write":
        return this.executeFileSynced(toolCallId, "write", args, () =>
          handleLocalWrite(args, this.cwd),
        );
      case "edit":
        return this.executeFileSynced(toolCallId, "edit", args, () =>
          handleLocalEdit(args, this.cwd),
        );
      default:
        return {
          error: {
            code: "unknown_tool",
            message: `no handler for tool: ${call.name}`,
          },
        };
    }
  }

  /**
   * Cancel a running tool by tool call id.
   * Returns true if the tool was found and cancelled.
   */
  cancel(toolCallId: string): boolean {
    const running = this.runningTools.get(toolCallId);
    if (running?.handle) {
      running.handle.cancel();
      return true;
    }
    return false;
  }

  /**
   * Stop a background job by job id.
   */
  stopJob(jobId: string): boolean {
    const job = this.backgroundJobs.get(jobId);
    if (!job || job.stopped) return false;

    // Find the running tool and cancel
    const running = this.runningTools.get(job.toolCallId);
    if (running?.handle) {
      running.handle.cancel();
    }

    return this.backgroundJobs.stop(jobId);
  }

  /** Get the background job table. */
  get jobs(): JobTable {
    return this.backgroundJobs;
  }

  /** Cleanup all running tools and background jobs. */
  cleanup(): void {
    for (const running of this.runningTools.values()) {
      if (running.handle) {
        running.handle.cancel();
      }
    }
    this.runningTools.clear();
    this.backgroundJobs.cleanup();
  }

  // --- Private ---

  private nextSeq(toolCallId: string): number {
    const seq = (this.sequences.get(toolCallId) ?? 0) + 1;
    this.sequences.set(toolCallId, seq);
    return seq;
  }

  private emitUpdate(toolCallId: string, stream: ToolUpdate["stream"], chunk: string): void {
    const update: ToolUpdate = {
      toolCallId,
      stream,
      chunk,
      sequence: this.nextSeq(toolCallId),
    };
    this.callbacks.onUpdate(update);
    this.hostUpdateListener?.(update);
  }

  private async executeBash(
    toolCallId: string,
    args: Record<string, unknown>,
  ): Promise<object> {
    if (typeof args.command !== "string") {
      return { error: { code: "missing_command", message: "bash requires a 'command' string argument" } };
    }

    const command = args.command.trim();
    if (command.length === 0) {
      return { error: { code: "empty_command", message: "bash requires a non-empty command" } };
    }

    if (this.bashPolicy.mode !== "unrestricted") {
      return { error: { code: "disallowed_command", message: `bash is not allowed (policy mode: "${this.bashPolicy.mode}")` } };
    }

    const timeout = typeof args.timeout === "number" ? args.timeout : 30_000;

    const bashHandle = startStreamingBash(command, this.cwd, {
      onStdout: (chunk) => this.emitUpdate(toolCallId, "stdout", chunk),
      onStderr: (chunk) => this.emitUpdate(toolCallId, "stderr", chunk),
    }, timeout);

    const running: RunningTool = {
      toolCallId,
      toolName: "bash",
      handle: bashHandle,
    };
    this.runningTools.set(toolCallId, running);

    // Track as background job if enabled and command looks like a server
    if (this.enableBackgroundJobs && isLikelyServerCommand(command)) {
      const jobId = this.backgroundJobs.add(toolCallId, command);
      this.emitUpdate(toolCallId, "status", `Background job started: ${jobId}`);
    }

    try {
      const result = await bashHandle.result;

      let output = result.stdout;
      if (result.stderr.length > 0) {
        output += (output.length > 0 ? "\n" : "") + result.stderr;
      }

      if (result.cancelled) {
        return {
          content: [{ type: "text", text: output || "(cancelled)" }],
          details: { exitCode: result.exitCode, cancelled: true },
          exitCode: result.exitCode,
        };
      }

      if (result.exitCode !== 0) {
        return {
          content: [{ type: "text", text: output || "(no output)" }],
          details: { exitCode: result.exitCode, cancelled: false },
          exitCode: result.exitCode,
        };
      }

      const details: Record<string, unknown> = { exitCode: result.exitCode };
      return {
        content: [{ type: "text", text: output || "(no output)" }],
        details,
      };
    } finally {
      this.runningTools.delete(toolCallId);
    }
  }

  private async executeFileSynced(
    toolCallId: string,
    toolName: string,
    args: Record<string, unknown>,
    fn: () => object,
  ): Promise<object> {
    const path = args.path;
    if (typeof path === "string" && (toolName === "write" || toolName === "edit")) {
      // Serialize mutations to the same file
      return this.pathMutex.runExclusive(path, fn);
    }
    return fn();
  }
}

function isLikelyServerCommand(command: string): boolean {
  const lower = command.toLowerCase();
  return (
    lower.includes("http.server") ||
    lower.includes("serve ") ||
    lower.includes("-m http.server") ||
    lower.includes("python3 -m http") ||
    lower.includes("node -e") && lower.includes("createserver") ||
    lower.includes("live-server") ||
    lower.includes("webpack serve") ||
    lower.includes("vite")
  );
}
