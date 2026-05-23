/**
 * Local session store — append-only JSONL persistence.
 *
 * Records agent trace entries to a `.jsonl` file so a local run can be
 * inspected and resumed. Host-owned — no Rust/Node/process assumptions in pi-core.
 *
 * Directory layout:
 *   <sessionDir>/
 *     session.jsonl       — append-only entries (one JSON object per line)
 *     artifacts/          — full tool outputs replaced by context projection
 */

import * as fs from "node:fs";
import * as path from "node:path";

// --- Types ---

export interface SessionMetadata {
  session_id: string;
  cwd: string;
  model: string;
  created_at: number;
  updated_at: number;
}

export type SessionEntryKind =
  | "session_start"
  | "user_prompt"
  | "assistant_message"
  | "tool_call"
  | "tool_result"
  | "tool_streaming_update"
  | "context_projection_report"
  | "artifact_reference"
  | "lifecycle_event"
  | "session_end";

export interface SessionEntry {
  /** Monotonic sequence number within this session. */
  seq: number;
  /** What kind of entry this is. */
  kind: SessionEntryKind;
  /** Wall-clock timestamp in ms. */
  timestamp: number;
  /** Entry payload — shape depends on `kind`. */
  data: unknown;
}

export class SessionCorruptError extends Error {
  readonly line: number;
  readonly cause_detail: string;
  constructor(line: number, cause: string) {
    super(`Corrupt session line ${line}: ${cause}`);
    this.name = "SessionCorruptError";
    this.line = line;
    this.cause_detail = cause;
  }
}

// --- Session Store ---

export class LocalSessionStore {
  readonly sessionDir: string;
  readonly sessionFile: string;
  readonly artifactsDir: string;
  private metadata: SessionMetadata;
  private seq = 0;
  private fd: number | null = null;

  constructor(sessionDir: string, metadata: Omit<SessionMetadata, "created_at" | "updated_at">) {
    this.sessionDir = sessionDir;
    this.sessionFile = path.join(sessionDir, "session.jsonl");
    this.artifactsDir = path.join(sessionDir, "artifacts");

    fs.mkdirSync(sessionDir, { recursive: true });
    fs.mkdirSync(this.artifactsDir, { recursive: true });

    const now = Date.now();
    this.metadata = {
      ...metadata,
      created_at: now,
      updated_at: now,
    };

    // Open file for append
    this.fd = fs.openSync(this.sessionFile, "a");

    // Write session_start
    this.append({
      kind: "session_start",
      data: this.metadata,
    });
  }

  /** Append an entry to the session log. */
  append(kind: SessionEntryKind, data: unknown): void;
  append(entry: Omit<SessionEntry, "seq" | "timestamp">): void;
  append(kindOrEntry: SessionEntryKind | Omit<SessionEntry, "seq" | "timestamp">, data?: unknown): void {
    const entry: SessionEntry = typeof kindOrEntry === "string"
      ? { seq: ++this.seq, kind: kindOrEntry, timestamp: Date.now(), data }
      : { seq: ++this.seq, timestamp: Date.now(), ...kindOrEntry };

    this.metadata.updated_at = entry.timestamp;

    const line = JSON.stringify(entry) + "\n";
    if (this.fd !== null) {
      fs.writeSync(this.fd, line);
    }
  }

  /** Close the session file and write session_end. */
  close(): void {
    this.append("session_end", { terminal: true });
    if (this.fd !== null) {
      fs.closeSync(this.fd);
      this.fd = null;
    }
  }

  /** Get session metadata. */
  getMetadata(): SessionMetadata {
    return { ...this.metadata };
  }

  /** Get current sequence number. */
  getSeq(): number {
    return this.seq;
  }
}

// --- Session Loader ---

export interface LoadedSession {
  metadata: SessionMetadata;
  entries: SessionEntry[];
}

/**
 * Load a session from a JSONL file.
 *
 * Returns metadata and all entries in order.
 * Corrupt lines produce useful typed errors.
 */
export function loadSession(sessionDir: string): LoadedSession {
  const sessionFile = path.join(sessionDir, "session.jsonl");
  const raw = fs.readFileSync(sessionFile, "utf-8");
  const lines = raw.split("\n").filter((l) => l.trim().length > 0);

  let metadata: SessionMetadata | null = null;
  const entries: SessionEntry[] = [];

  for (let i = 0; i < lines.length; i++) {
    let parsed: SessionEntry;
    try {
      parsed = JSON.parse(lines[i]) as SessionEntry;
    } catch {
      throw new SessionCorruptError(i + 1, `invalid JSON: ${lines[i].slice(0, 100)}`);
    }

    if (!parsed.kind || typeof parsed.kind !== "string") {
      throw new SessionCorruptError(i + 1, `missing or invalid 'kind' field`);
    }

    if (parsed.kind === "session_start") {
      metadata = parsed.data as SessionMetadata;
    }

    entries.push(parsed);
  }

  if (!metadata) {
    throw new SessionCorruptError(0, "no session_start entry found");
  }

  return { metadata, entries };
}

/**
 * Reconstruct agent messages from a loaded session suitable for AgentOptions.messages.
 *
 * Extracts user prompts, assistant messages, tool calls, and tool results
 * in the order they appeared.
 */
export function reconstructMessages(entries: SessionEntry[]): unknown[] {
  const messages: unknown[] = [];

  for (const entry of entries) {
    switch (entry.kind) {
      case "user_prompt":
        messages.push(entry.data);
        break;
      case "assistant_message":
        messages.push(entry.data);
        break;
      case "tool_result":
        messages.push(entry.data);
        break;
      // tool_call entries are inside assistant messages, not standalone
      // tool_streaming_update, context_projection_report, etc. are trace-only
    }
  }

  return messages;
}
