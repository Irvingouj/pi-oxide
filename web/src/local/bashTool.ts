/**
 * Local bash tool — real shell command execution.
 *
 * Executes commands in cwd with a permission policy.
 */

import { execFileSync, type ExecFileSyncOptions } from "node:child_process";
import * as path from "node:path";

// --- Result/error payload builders ---

function okPayload(text: string, details?: unknown): object {
  const payload: Record<string, unknown> = {
    content: [{ type: "text", text }],
  };
  if (details !== undefined) {
    payload.details = details;
  }
  return payload;
}

function errorPayload(code: string, message: string): object {
  return { error: { code, message } };
}

// --- Bash policy ---

/**
 * Bash execution policy.
 *
 * - `{ mode: "unrestricted" }`: execute any command. The caller is responsible
 *   for safety. This is the honest mode for a local agent that needs real shell.
 * - `{ mode: "deny" }`: reject all commands. Safe default when no shell is wanted.
 *
 * There is no prefix allowlist. Shell command parsing for safety is out of scope
 * for this milestone. Use mode: "unrestricted" in tests and controlled environments.
 */
export type BashPolicy =
  | { mode: "unrestricted" }
  | { mode: "deny" };

// --- Constants ---

/** Maximum output bytes before tail truncation. */
const BASH_MAX_OUTPUT_BYTES = 50_000;

// --- Handler ---

export interface BashArgs {
  command: string;
  timeout?: number;
}

export function handleLocalBash(
  args: Record<string, unknown>,
  cwd: string,
  policy: BashPolicy,
): object {
  if (typeof args.command !== "string") {
    return errorPayload("missing_command", "bash requires a 'command' string argument");
  }

  const command = args.command.trim();

  if (command.length === 0) {
    return errorPayload("empty_command", "bash requires a non-empty command");
  }

  // Policy check
  if (policy.mode !== "unrestricted") {
    return errorPayload(
      "disallowed_command",
      `bash is not allowed (policy mode: "${policy.mode}")`,
    );
  }

  const timeoutMs = typeof args.timeout === "number" ? args.timeout : 30_000;

  const options: ExecFileSyncOptions = {
    cwd,
    timeout: timeoutMs,
    maxBuffer: 10 * 1024 * 1024, // 10MB internal buffer
    encoding: "utf-8",
  };

  let stdout: string;
  let stderr: string;
  let exitCode: number | null = 0;

  try {
    stdout = execFileSync("sh", ["-c", command], options) as string;
    stderr = "";
  } catch (err: unknown) {
    const execErr = err as {
      stdout?: string | Buffer;
      stderr?: string | Buffer;
      status?: number | null;
      signal?: string | null;
      killed?: boolean;
      code?: string;
    };

    stdout = typeof execErr.stdout === "string" ? execErr.stdout : "";
    stderr = typeof execErr.stderr === "string" ? execErr.stderr : "";

    // Timeout: Node sets code to ETIMEDOUT or signal to SIGTERM
    if (execErr.code === "ETIMEDOUT" || execErr.signal === "SIGTERM") {
      return errorPayload(
        "timeout",
        `command timed out after ${timeoutMs}ms: ${command}`,
      );
    }

    exitCode = execErr.status ?? 1;

    // Spawn failure (e.g. command not found)
    if (execErr.status === null && execErr.signal !== "SIGTERM") {
      return errorPayload(
        "spawn_failed",
        `failed to execute command: ${command}: ${stderr || "unknown error"}`,
      );
    }
  }

  // Tail-truncate combined output
  let output = stdout;
  if (stderr.length > 0) {
    output += (output.length > 0 ? "\n" : "") + stderr;
  }

  let truncated = false;
  if (output.length > BASH_MAX_OUTPUT_BYTES) {
    output = output.slice(output.length - BASH_MAX_OUTPUT_BYTES);
    // Cut at first newline to avoid partial lines
    const nlIdx = output.indexOf("\n");
    if (nlIdx >= 0) {
      output = output.slice(nlIdx + 1);
    }
    output = "... (truncated)\n" + output;
    truncated = true;
  }

  if (exitCode !== 0) {
    return {
      content: [{ type: "text", text: output || "(no output)" }],
      details: { exitCode, truncated },
      exitCode,
    };
  }

  const details = truncated ? { exitCode, truncated } : { exitCode: 0 };
  return okPayload(output || "(no output)", details);
}
