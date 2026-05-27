/**
 * Workspace-root-relative path validation for coding tools.
 *
 * Rules:
 * - All paths are workspace-root-relative (e.g. "src/main.rs", "README.md").
 * - No leading slash — these are not absolute host paths.
 * - No path traversal ("..").
 * - No empty path segments.
 * - No empty string.
 * - No backslashes — paths must use forward slashes.
 */

export class PathError extends Error {
	readonly code: string;
	constructor(code: string, message: string) {
		super(message);
		this.name = "PathError";
		this.code = code;
	}
}

/**
 * Validate a workspace-root-relative path.
 * Returns the normalized path string or throws PathError.
 */
export function validateWorkspacePath(raw: unknown): string {
	if (typeof raw !== "string") {
		throw new PathError(
			"invalid_path_type",
			`path must be a string, got ${typeof raw}`,
		);
	}

	const path = raw;

	if (path.length === 0) {
		throw new PathError("empty_path", "path must not be empty");
	}

	// Reject backslashes — workspace paths use forward slashes only
	if (path.includes("\\")) {
		throw new PathError(
			"backslash_path",
			`path must use "/" separators and be workspace-root-relative: "${path}"`,
		);
	}

	// Reject absolute-looking paths (leading slash or drive letter)
	if (path.startsWith("/") || /^[A-Za-z]:/.test(path)) {
		throw new PathError(
			"absolute_path",
			`path must be workspace-root-relative, not absolute: "${path}"`,
		);
	}

	// Reject path traversal
	const segments = path.split("/");
	for (const seg of segments) {
		if (seg === "..") {
			throw new PathError(
				"path_traversal",
				`path must not contain "..": "${path}"`,
			);
		}
		if (seg.length === 0 && segments.length > 1) {
			throw new PathError(
				"empty_segment",
				`path must not contain empty segments: "${path}"`,
			);
		}
	}

	// Reject trailing slash for file paths (allow for directory-only listing)
	// We don't enforce this universally — tools decide.

	return path;
}

/**
 * Get the directory portion of a path.
 * "src/main.rs" → "src"
 * "README.md" → ""
 */
export function dirname(path: string): string {
	const idx = path.lastIndexOf("/");
	return idx >= 0 ? path.slice(0, idx) : "";
}
