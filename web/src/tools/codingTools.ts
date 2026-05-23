/**
 * Typed coding-tool handlers with an in-memory workspace.
 *
 * Two tool surfaces coexist:
 * 1. Legacy handlers (read_file, write_file, list_files, search_files) from Milestone 3.
 * 2. pi-compatible handlers (read, write, edit, ls, grep, find, bash) from Milestone 3.6.
 *
 * Each handler:
 * 1. Validates and parses arguments from the tool call.
 * 2. Executes the operation against the workspace.
 * 3. Returns a payload compatible with Rust's onToolDone (either a ToolResult or ToolError).
 */

import type { ToolCall } from "../wasmBinding.ts";
import type { ToolRegistry } from "../fakeTools.ts";
import { validateWorkspacePath, dirname } from "./path.ts";
import { TOOL_NAMES, PI_TOOL_NAMES } from "./schemas.ts";

// --- In-memory workspace ---

export class MemoryWorkspace {
  private files = new Map<string, string>();

  /** Set a file's content. Creates parent dirs implicitly. */
  writeFile(path: string, content: string): void {
    this.files.set(path, content);
  }

  /** Read a file. Returns undefined if not found. */
  readFile(path: string): string | undefined {
    return this.files.get(path);
  }

  /** Check if a file exists. */
  hasFile(path: string): boolean {
    return this.files.has(path);
  }

  /** List entries in a directory. Returns file paths and subdirectory names. */
  listDir(dirPath: string): { files: string[]; dirs: string[] } {
    const prefix = dirPath.length > 0 ? dirPath + "/" : "";
    const files: string[] = [];
    const dirs = new Set<string>();

    for (const filePath of this.files.keys()) {
      if (dirPath.length > 0 && !filePath.startsWith(prefix)) continue;
      if (dirPath.length === 0 && !filePath.includes("/")) {
        // Root-level file
        files.push(filePath);
        continue;
      }

      const rest = dirPath.length > 0 ? filePath.slice(prefix.length) : filePath;
      const slashIdx = rest.indexOf("/");
      if (slashIdx >= 0) {
        dirs.add(rest.slice(0, slashIdx));
      } else {
        files.push(filePath);
      }
    }

    return { files, dirs: [...dirs].sort() };
  }

  /** Search for files matching a glob-like pattern. */
  search(pattern: string, basePath: string): string[] {
    const regex = globToRegex(pattern);
    const prefix = basePath.length > 0 ? basePath + "/" : "";
    const results: string[] = [];

    for (const filePath of this.files.keys()) {
      if (basePath.length > 0 && !filePath.startsWith(prefix)) continue;
      if (regex.test(filePath)) {
        results.push(filePath);
      }
    }

    return results.sort();
  }

  /** Reset the workspace. */
  clear(): void {
    this.files.clear();
  }
}

/** Convert a simple glob pattern to a RegExp. Supports * and ?. */
function globToRegex(pattern: string): RegExp {
  const escaped = pattern
    .replace(/[.+^${}()|[\]\\]/g, "\\$&")
    .replace(/\*/g, ".*")
    .replace(/\?/g, ".");
  return new RegExp(escaped, "i");
}

// --- Result/error payload builders ---

function okPayload(text: string): object {
  return { content: [{ type: "text", text }] };
}

function errorPayload(code: string, message: string): object {
  return { error: { code, message } };
}

// --- Tool handler type ---

export type CodingToolHandler = (
  args: Record<string, unknown>,
  workspace: MemoryWorkspace
) => object;

// ========================================================================
// Legacy tool handlers (Milestone 3)
// ========================================================================

function handleReadFile(args: Record<string, unknown>, workspace: MemoryWorkspace): object {
  if (typeof args.path !== "string") {
    return errorPayload("missing_argument", "read_file requires a 'path' string argument");
  }

  let path: string;
  try {
    path = validateWorkspacePath(args.path);
  } catch (e: unknown) {
    const pe = e as { code: string; message: string };
    return errorPayload(pe.code, pe.message);
  }

  const content = workspace.readFile(path);
  if (content === undefined) {
    return errorPayload("file_not_found", `file not found: ${path}`);
  }

  return okPayload(content);
}

function handleListFiles(args: Record<string, unknown>, workspace: MemoryWorkspace): object {
  let dirPath = "";
  if (args.path !== undefined && args.path !== "") {
    try {
      dirPath = validateWorkspacePath(args.path as string);
    } catch (e: unknown) {
      const pe = e as { code: string; message: string };
      return errorPayload(pe.code, pe.message);
    }
  }

  const { files, dirs } = workspace.listDir(dirPath);
  const entries: string[] = [];

  for (const d of dirs) {
    entries.push(d + "/");
  }
  const prefix = dirPath.length > 0 ? dirPath + "/" : "";
  for (const f of files) {
    entries.push(f.slice(prefix.length));
  }

  if (entries.length === 0) {
    return okPayload("(empty directory)");
  }

  return okPayload(entries.join("\n"));
}

function handleSearchFiles(args: Record<string, unknown>, workspace: MemoryWorkspace): object {
  if (typeof args.pattern !== "string" || args.pattern.length === 0) {
    return errorPayload("missing_argument", "search_files requires a 'pattern' string argument");
  }

  let basePath = "";
  if (args.path !== undefined && args.path !== "") {
    try {
      basePath = validateWorkspacePath(args.path as string);
    } catch (e: unknown) {
      const pe = e as { code: string; message: string };
      return errorPayload(pe.code, pe.message);
    }
  }

  const results = workspace.search(args.pattern, basePath);
  if (results.length === 0) {
    return okPayload("(no matches)");
  }

  return okPayload(results.join("\n"));
}

function handleWriteFile(args: Record<string, unknown>, workspace: MemoryWorkspace): object {
  if (typeof args.path !== "string") {
    return errorPayload("missing_argument", "write_file requires a 'path' string argument");
  }
  if (typeof args.content !== "string") {
    return errorPayload("missing_argument", "write_file requires a 'content' string argument");
  }

  let path: string;
  try {
    path = validateWorkspacePath(args.path);
  } catch (e: unknown) {
    const pe = e as { code: string; message: string };
    return errorPayload(pe.code, pe.message);
  }

  workspace.writeFile(path, args.content);
  return okPayload(`wrote ${args.content.length} bytes to ${path}`);
}

// ========================================================================
// pi-compatible tool handlers (Milestone 3.6)
// ========================================================================

/** Validate and return a workspace path from args, or return an error payload. */
function validatePath(
  raw: unknown,
  toolName: string,
): { ok: true; path: string } | { ok: false; error: object } {
  if (typeof raw !== "string") {
    return { ok: false, error: errorPayload("missing_argument", `${toolName} requires a 'path' string argument`) };
  }
  try {
    return { ok: true, path: validateWorkspacePath(raw) };
  } catch (e: unknown) {
    const pe = e as { code: string; message: string };
    return { ok: false, error: errorPayload(pe.code, pe.message) };
  }
}

function handleRead(args: Record<string, unknown>, workspace: MemoryWorkspace): object {
  const pathResult = validatePath(args.path, "read");
  if (!pathResult.ok) return pathResult.error;

  const content = workspace.readFile(pathResult.path);
  if (content === undefined) {
    return errorPayload("file_not_found", `file not found: ${pathResult.path}`);
  }

  // Apply optional offset/limit (1-based line numbers)
  const offset = typeof args.offset === "number" ? args.offset : undefined;
  const limit = typeof args.limit === "number" ? args.limit : undefined;

  if (offset !== undefined || limit !== undefined) {
    const lines = content.split("\n");
    const start = offset !== undefined ? Math.max(1, offset) - 1 : 0;
    const end = limit !== undefined ? start + limit : lines.length;
    const selected = lines.slice(start, end);
    // Show line numbers for partial reads
    const numbered = selected.map((line, i) => `${start + i + 1}: ${line}`).join("\n");
    return okPayload(numbered);
  }

  return okPayload(content);
}

function handleWrite(args: Record<string, unknown>, workspace: MemoryWorkspace): object {
  const pathResult = validatePath(args.path, "write");
  if (!pathResult.ok) return pathResult.error;

  if (typeof args.content !== "string") {
    return errorPayload("missing_argument", "write requires a 'content' string argument");
  }

  workspace.writeFile(pathResult.path, args.content);
  return okPayload(`wrote ${args.content.length} bytes to ${pathResult.path}`);
}

function handleEdit(args: Record<string, unknown>, workspace: MemoryWorkspace): object {
  const pathResult = validatePath(args.path, "edit");
  if (!pathResult.ok) return pathResult.error;

  if (!Array.isArray(args.edits) || args.edits.length === 0) {
    return errorPayload("missing_argument", "edit requires a non-empty 'edits' array argument");
  }

  const content = workspace.readFile(pathResult.path);
  if (content === undefined) {
    return errorPayload("file_not_found", `file not found: ${pathResult.path}`);
  }

  let current = content;
  for (const edit of args.edits as { oldText?: unknown; newText?: unknown }[]) {
    if (typeof edit.oldText !== "string") {
      return errorPayload("missing_argument", "each edit must have an 'oldText' string");
    }
    if (typeof edit.newText !== "string") {
      return errorPayload("missing_argument", "each edit must have a 'newText' string");
    }

    const idx = current.indexOf(edit.oldText);
    if (idx === -1) {
      return errorPayload(
        "edit_not_found",
        `oldText not found in ${pathResult.path}: "${edit.oldText}"`,
      );
    }

    current = current.slice(0, idx) + edit.newText + current.slice(idx + edit.oldText.length);
  }

  workspace.writeFile(pathResult.path, current);
  return okPayload(`edited ${pathResult.path}`);
}

function handleLs(args: Record<string, unknown>, workspace: MemoryWorkspace): object {
  let dirPath = "";
  if (args.path !== undefined && args.path !== "") {
    const pathResult = validatePath(args.path, "ls");
    if ("error" in pathResult && !pathResult.ok) return pathResult.error;
    if ("path" in pathResult) dirPath = pathResult.path;
  }

  const { files, dirs } = workspace.listDir(dirPath);
  const entries: string[] = [];

  for (const d of dirs) {
    entries.push(d + "/");
  }
  const prefix = dirPath.length > 0 ? dirPath + "/" : "";
  for (const f of files) {
    entries.push(f.slice(prefix.length));
  }

  if (entries.length === 0) {
    return okPayload("(empty directory)");
  }

  // Apply optional limit
  const limit = typeof args.limit === "number" ? args.limit : undefined;
  const limited = limit !== undefined ? entries.slice(0, limit) : entries;
  return okPayload(limited.join("\n"));
}

function handleGrep(args: Record<string, unknown>, workspace: MemoryWorkspace): object {
  if (typeof args.pattern !== "string" || args.pattern.length === 0) {
    return errorPayload("missing_argument", "grep requires a 'pattern' string argument");
  }

  // Build regex from pattern
  const literal = args.literal === true;
  const ignoreCase = args.ignoreCase !== false; // default true
  const context = typeof args.context === "number" ? args.context : 0;
  const limit = typeof args.limit === "number" ? args.limit : undefined;

  let patternStr = args.pattern;
  if (literal) {
    patternStr = patternStr.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  }
  const flags = ignoreCase ? "gi" : "g";
  let regex: RegExp;
  try {
    regex = new RegExp(patternStr, flags);
  } catch {
    return errorPayload("invalid_pattern", `invalid regex pattern: ${args.pattern}`);
  }

  // Determine which files to search
  const globFilter = typeof args.glob === "string" ? args.glob : undefined;
  const globRegex = globFilter ? globToRegex(globFilter) : null;

  let basePath = "";
  if (args.path !== undefined && args.path !== "") {
    const pathResult = validatePath(args.path, "grep");
    if ("error" in pathResult && !pathResult.ok) return pathResult.error;
    if ("path" in pathResult) basePath = pathResult.path;
  }

  const results: string[] = [];

  // Determine candidate files: if basePath is an exact file, search only that file.
  // Otherwise treat it as a directory prefix.
  const files = workspace["files"] as Map<string, string>;
  let candidateFiles: [string, string][] = [];
  if (basePath.length > 0 && files.has(basePath)) {
    // Exact file path — search only this file
    candidateFiles = [[basePath, files.get(basePath)!]];
  } else {
    // Directory prefix (or empty — search all)
    const prefix = basePath.length > 0 ? basePath + "/" : "";
    for (const [filePath, fileContent] of files) {
      if (basePath.length > 0 && !filePath.startsWith(prefix)) continue;
      if (globRegex && !globRegex.test(filePath)) continue;
      candidateFiles.push([filePath, fileContent]);
    }
  }

  for (const [filePath, fileContent] of candidateFiles) {

    const lines = fileContent.split("\n");
    for (let i = 0; i < lines.length; i++) {
      // Reset lastIndex for global regex
      regex.lastIndex = 0;
      if (regex.test(lines[i])) {
        // Context lines
        const start = Math.max(0, i - context);
        const end = Math.min(lines.length - 1, i + context);
        for (let j = start; j <= end; j++) {
          const marker = j === i ? ">" : " ";
          results.push(`${filePath}:${j + 1}${marker} ${lines[j]}`);
        }
      }

      if (limit !== undefined && results.length >= limit) break;
    }
    if (limit !== undefined && results.length >= limit) break;
  }

  if (results.length === 0) {
    return okPayload("(no matches)");
  }

  return okPayload(results.join("\n"));
}

function handleFind(args: Record<string, unknown>, workspace: MemoryWorkspace): object {
  if (typeof args.pattern !== "string" || args.pattern.length === 0) {
    return errorPayload("missing_argument", "find requires a 'pattern' string argument");
  }

  let basePath = "";
  if (args.path !== undefined && args.path !== "") {
    const pathResult = validatePath(args.path, "find");
    if ("error" in pathResult && !pathResult.ok) return pathResult.error;
    if ("path" in pathResult) basePath = pathResult.path;
  }

  const results = workspace.search(args.pattern, basePath);

  // Apply optional limit
  const limit = typeof args.limit === "number" ? args.limit : undefined;
  const limited = limit !== undefined ? results.slice(0, limit) : results;

  if (limited.length === 0) {
    return okPayload("(no matches)");
  }

  return okPayload(limited.join("\n"));
}

/**
 * Constrained fake bash handler.
 *
 * Only supports a deterministic whitelist of commands. No real shell execution.
 * Supported commands:
 *   - "npm test" / "test": runs a simulated test against MemoryWorkspace fixtures.
 *   - "npm run build" / "build": simulated build — always succeeds.
 */
const ALLOWED_COMMANDS = new Set(["npm test", "test", "npm run build", "build"]);

function handleBash(args: Record<string, unknown>, workspace: MemoryWorkspace): object {
  if (typeof args.command !== "string" || args.command.length === 0) {
    return errorPayload("missing_argument", "bash requires a 'command' string argument");
  }

  const command = args.command.trim();

  if (!ALLOWED_COMMANDS.has(command)) {
    return errorPayload(
      "disallowed_command",
      `command not allowed: "${command}". Allowed: ${[...ALLOWED_COMMANDS].join(", ")}`,
    );
  }

  // Simulate test: look for test fixtures in the workspace
  if (command === "npm test" || command === "test") {
    return simulateTest(workspace);
  }

  // Simulate build: always succeeds
  if (command === "npm run build" || command === "build") {
    return okPayload("Build completed successfully.\n");
  }

  // Should not reach here, but be safe
  return errorPayload("disallowed_command", `command not allowed: "${command}"`);
}

/** Simulate running tests by checking workspace for fixture patterns. */
function simulateTest(workspace: MemoryWorkspace): object {
  // Look for a package.json to determine test behavior
  const pkg = workspace.readFile("package.json");
  if (pkg) {
    // If package.json has a test script, simulate running it
    try {
      const parsed = JSON.parse(pkg) as { scripts?: { test?: string } };
      if (parsed.scripts?.test) {
        // Simulate test pass/fail based on source files
        const src = workspace.readFile("src/index.ts") ?? workspace.readFile("src/main.ts");
        if (src && src.includes("export function add")) {
          // Check if add function is correct
          if (src.includes("return a + b")) {
            return okPayload(
              "TAP version 13\n" +
              "# add\n" +
              "ok 1 - add(2, 3) returns 5\n" +
              "# subtract\n" +
              "ok 2 - subtract(5, 3) returns 2\n" +
              "\n1..2\n# tests 2\n# pass 2\n# fail 0\n",
            );
          }
          // Function exists but wrong logic
          return bashError("TAP version 13\n# add\nnot ok 1 - add(2, 3) returns 5\n  ---\n  expected: 5\n  actual: NaN\n  ...\n\n1..1\n# tests 1\n# pass 0\n# fail 1\n");
        }
      }
    } catch {
      // Not valid JSON, fall through
    }
  }

  // Default: always pass
  return okPayload("No tests found. Exiting with code 0.\n");
}

function bashError(stdout: string): object {
  return { content: [{ type: "text", text: stdout }], exitCode: 1 };
}

// ========================================================================
// Registry — all handlers (legacy + pi-compatible)
// ========================================================================

const HANDLERS: Record<string, CodingToolHandler> = {
  // Legacy (Milestone 3)
  [TOOL_NAMES.readFile]: handleReadFile,
  [TOOL_NAMES.listFiles]: handleListFiles,
  [TOOL_NAMES.searchFiles]: handleSearchFiles,
  [TOOL_NAMES.writeFile]: handleWriteFile,
  // pi-compatible (Milestone 3.6)
  [PI_TOOL_NAMES.read]: handleRead,
  [PI_TOOL_NAMES.write]: handleWrite,
  [PI_TOOL_NAMES.edit]: handleEdit,
  [PI_TOOL_NAMES.ls]: handleLs,
  [PI_TOOL_NAMES.grep]: handleGrep,
  [PI_TOOL_NAMES.find]: handleFind,
  [PI_TOOL_NAMES.bash]: handleBash,
};

/**
 * Coding tool registry. Wraps a MemoryWorkspace and dispatches tool calls
 * to typed handlers. Produces payloads compatible with Rust onToolDone.
 */
export class CodingToolRegistry implements ToolRegistry {
  readonly workspace: MemoryWorkspace;
  public readonly log: string[] = [];

  constructor(workspace?: MemoryWorkspace) {
    this.workspace = workspace ?? new MemoryWorkspace();
  }

  /** Execute a tool call. Returns a JSON payload for onToolDone. */
  execute(call: ToolCall): object {
    const handler = HANDLERS[call.name];
    if (!handler) {
      this.log.push(`tool_error(${call.name}): unknown tool`);
      return errorPayload("unknown_tool", `no handler for tool: ${call.name}`);
    }

    const args = (call.arguments ?? {}) as Record<string, unknown>;
    const result = handler(args, this.workspace);

    // Log for traceability
    const isError = "error" in result;
    this.log.push(
      `tool_result(${call.name}): ${isError ? "[ERROR] " : ""}${JSON.stringify(args)}`
    );

    return result;
  }

  /** Check if a tool name is known. */
  has(name: string): boolean {
    return name in HANDLERS;
  }
}
