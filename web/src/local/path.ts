/**
 * Local filesystem path boundary.
 *
 * Resolves workspace-relative paths against a cwd root directory.
 * Rejects traversal outside cwd, absolute paths outside cwd, and malformed paths.
 */

import * as path from "node:path";

export class LocalPathError extends Error {
  readonly code: string;
  constructor(code: string, message: string) {
    super(message);
    this.name = "LocalPathError";
    this.code = code;
  }
}

/**
 * Resolve and validate a workspace-relative path against cwd.
 * Returns the absolute resolved path.
 * Throws LocalPathError on rejection.
 */
export function resolveLocalPath(cwd: string, raw: unknown): string {
  if (typeof raw !== "string") {
    throw new LocalPathError("missing_path", "path must be a string");
  }

  const p = raw;

  if (p.length === 0) {
    throw new LocalPathError("empty_path", "path must not be empty");
  }

  if (p.includes("\\")) {
    throw new LocalPathError(
      "backslash_path",
      `path must use "/" separators: "${p}"`,
    );
  }

  if (p.startsWith("/")) {
    throw new LocalPathError(
      "absolute_path",
      `path must be workspace-relative, not absolute: "${p}"`,
    );
  }

  // Reject path traversal components
  const segments = p.split("/");
  for (const seg of segments) {
    if (seg === "..") {
      throw new LocalPathError(
        "path_traversal",
        `path must not contain "..": "${p}"`,
      );
    }
    if (seg.length === 0 && segments.length > 1) {
      throw new LocalPathError(
        "empty_segment",
        `path must not contain empty segments: "${p}"`,
      );
    }
  }

  const resolved = path.resolve(cwd, p);

  // Final safety check: resolved path must be inside cwd
  const normalizedCwd = path.resolve(cwd);
  if (!resolved.startsWith(normalizedCwd + path.sep) && resolved !== normalizedCwd) {
    throw new LocalPathError(
      "outside_cwd",
      `path resolves outside cwd: "${p}"`,
    );
  }

  return resolved;
}

/**
 * Check if an absolute path is inside cwd.
 */
export function isInsideCwd(cwd: string, absolutePath: string): boolean {
  const normalizedCwd = path.resolve(cwd);
  return absolutePath.startsWith(normalizedCwd + path.sep) || absolutePath === normalizedCwd;
}
