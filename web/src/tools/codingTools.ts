/**
 * Typed coding-tool handlers with an in-memory workspace.
 *
 * Each handler:
 * 1. Validates and parses arguments from the tool call.
 * 2. Executes the operation against the workspace.
 * 3. Returns a payload compatible with Rust's onToolDone (either a ToolResult or ToolError).
 */

import type { ToolCall } from "../wasmBinding.ts";
import type { ToolRegistry } from "../fakeTools.ts";
import { validateWorkspacePath, dirname } from "./path.ts";
import { TOOL_NAMES } from "./schemas.ts";

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

// --- Typed argument types ---

interface ReadFileArgs {
  path: string;
}

interface ListFilesArgs {
  path?: string;
}

interface SearchFilesArgs {
  pattern: string;
  path?: string;
}

interface WriteFileArgs {
  path: string;
  content: string;
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

// --- Tool handlers ---

function handleReadFile(args: Record<string, unknown>, workspace: MemoryWorkspace): object {
  // Validate arguments
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
  // path is optional — defaults to root
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

  // Show dirs first with trailing /
  for (const d of dirs) {
    entries.push(d + "/");
  }
  // Show files relative to the listed directory
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

// --- Registry ---

const HANDLERS: Record<string, CodingToolHandler> = {
  [TOOL_NAMES.readFile]: handleReadFile,
  [TOOL_NAMES.listFiles]: handleListFiles,
  [TOOL_NAMES.searchFiles]: handleSearchFiles,
  [TOOL_NAMES.writeFile]: handleWriteFile,
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
