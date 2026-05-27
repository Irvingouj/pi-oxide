/**
 * Local filesystem tools: read, write, edit.
 *
 * Operate on real files inside a cwd boundary.
 */

import * as fs from "node:fs";
import * as path from "node:path";
import { type LocalPathError, resolveLocalPath } from "./path.ts";

// --- Constants ---

/** Maximum lines returned from read before head-truncation. */
const READ_MAX_LINES = 2000;
/** Maximum bytes returned from read. */
const READ_MAX_BYTES = 50_000;

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

// --- Helpers ---

function validatePath(
	raw: unknown,
	cwd: string,
	_toolName: string,
): { ok: true; absolute: string } | { ok: false; error: object } {
	try {
		const absolute = resolveLocalPath(cwd, raw);
		return { ok: true, absolute };
	} catch (e: unknown) {
		const pe = e as LocalPathError;
		return { ok: false, error: errorPayload(pe.code, pe.message) };
	}
}

// --- read ---

export interface ReadArgs {
	path: string;
	offset?: number;
	limit?: number;
}

export function handleLocalRead(
	args: Record<string, unknown>,
	cwd: string,
): object {
	if (typeof args.path !== "string") {
		return errorPayload(
			"missing_path",
			"read requires a 'path' string argument",
		);
	}

	const pathResult = validatePath(args.path, cwd, "read");
	if (!pathResult.ok) return pathResult.error;

	let stat: fs.Stats;
	try {
		stat = fs.statSync(pathResult.absolute);
	} catch {
		return errorPayload("file_not_found", `file not found: ${args.path}`);
	}

	if (stat.isDirectory()) {
		return errorPayload(
			"is_directory",
			`path is a directory, not a file: ${args.path}`,
		);
	}

	let content: string;
	try {
		content = fs.readFileSync(pathResult.absolute, "utf-8");
	} catch {
		return errorPayload("read_error", `failed to read file: ${args.path}`);
	}

	// Apply offset/limit (1-based line numbers)
	const offset = typeof args.offset === "number" ? args.offset : undefined;
	const limit = typeof args.limit === "number" ? args.limit : undefined;
	const lines = content.split("\n");

	let selected: string[];
	let startLine: number;
	if (offset !== undefined || limit !== undefined) {
		startLine = offset !== undefined ? Math.max(1, offset) - 1 : 0;
		const end = limit !== undefined ? startLine + limit : lines.length;
		selected = lines.slice(startLine, end);
	} else {
		startLine = 0;
		selected = lines;
	}

	// Head truncation for large output
	let truncated = false;
	let truncationInfo: { totalLines: number; shownLines: number } | undefined;
	if (selected.length > READ_MAX_LINES) {
		truncationInfo = { totalLines: lines.length, shownLines: READ_MAX_LINES };
		selected = selected.slice(0, READ_MAX_LINES);
		truncated = true;
	}

	// Byte truncation
	let text = selected
		.map((line, i) => `${startLine + i + 1}: ${line}`)
		.join("\n");
	if (text.length > READ_MAX_BYTES) {
		truncated = true;
		text = text.slice(0, READ_MAX_BYTES);
	}

	if (truncated) {
		text += `\n... (truncated)`;
	}

	const details = truncationInfo
		? { truncated: true, ...truncationInfo }
		: undefined;
	return okPayload(text, details);
}

// --- write ---

export interface WriteArgs {
	path: string;
	content: string;
}

export function handleLocalWrite(
	args: Record<string, unknown>,
	cwd: string,
): object {
	if (typeof args.path !== "string") {
		return errorPayload(
			"missing_path",
			"write requires a 'path' string argument",
		);
	}
	if (typeof args.content !== "string") {
		return errorPayload(
			"missing_content",
			"write requires a 'content' string argument",
		);
	}

	const pathResult = validatePath(args.path, cwd, "write");
	if (!pathResult.ok) return pathResult.error;

	try {
		// Create parent directories
		const dir = path.dirname(pathResult.absolute);
		fs.mkdirSync(dir, { recursive: true });
		fs.writeFileSync(pathResult.absolute, args.content, "utf-8");
	} catch (err) {
		const msg = err instanceof Error ? err.message : String(err);
		return errorPayload(
			"write_error",
			`failed to write file: ${args.path}: ${msg}`,
		);
	}

	return okPayload(`wrote ${args.content.length} bytes to ${args.path}`);
}

// --- edit ---

export interface EditArgs {
	path: string;
	edits: Array<{ oldText: string; newText: string }>;
}

export function handleLocalEdit(
	args: Record<string, unknown>,
	cwd: string,
): object {
	if (typeof args.path !== "string") {
		return errorPayload(
			"missing_path",
			"edit requires a 'path' string argument",
		);
	}
	if (!Array.isArray(args.edits) || args.edits.length === 0) {
		return errorPayload(
			"missing_edits",
			"edit requires a non-empty 'edits' array argument",
		);
	}

	const pathResult = validatePath(args.path, cwd, "edit");
	if (!pathResult.ok) return pathResult.error;

	// Read original file
	let original: string;
	try {
		fs.statSync(pathResult.absolute);
	} catch {
		return errorPayload("file_not_found", `file not found: ${args.path}`);
	}
	try {
		original = fs.readFileSync(pathResult.absolute, "utf-8");
	} catch {
		return errorPayload(
			"read_error",
			`failed to read file for editing: ${args.path}`,
		);
	}

	// Validate edits against the original file
	const applied: Array<{
		oldText: string;
		newText: string;
		occurrences: number;
	}> = [];
	for (const edit of args.edits as Array<{
		oldText?: unknown;
		newText?: unknown;
	}>) {
		if (typeof edit.oldText !== "string") {
			return errorPayload(
				"missing_oldText",
				"each edit must have an 'oldText' string",
			);
		}
		if (edit.oldText.length === 0) {
			return errorPayload("empty_oldText", "edit oldText must not be empty");
		}
		if (typeof edit.newText !== "string") {
			return errorPayload(
				"missing_newText",
				"each edit must have a 'newText' string",
			);
		}

		// Check against original file for ambiguity
		const count = countOccurrences(original, edit.oldText);
		if (count === 0) {
			return errorPayload(
				"edit_not_found",
				`oldText not found in ${args.path}: "${edit.oldText}"`,
			);
		}
		if (count > 1) {
			return errorPayload(
				"ambiguous_edit",
				`oldText found ${count} times in ${args.path}, which is ambiguous: "${edit.oldText}"`,
			);
		}
		applied.push({
			oldText: edit.oldText,
			newText: edit.newText,
			occurrences: count,
		});
	}

	// Apply edits sequentially on a copy
	let current = original;
	for (const edit of applied) {
		const idx = current.indexOf(edit.oldText);
		current =
			current.slice(0, idx) +
			edit.newText +
			current.slice(idx + edit.oldText.length);
	}

	try {
		fs.writeFileSync(pathResult.absolute, current, "utf-8");
	} catch (err) {
		const msg = err instanceof Error ? err.message : String(err);
		return errorPayload(
			"write_error",
			`failed to write edited file: ${args.path}: ${msg}`,
		);
	}

	const diffLines = applied
		.map((e) => `  -${e.oldText}\n  +${e.newText}`)
		.join("\n");

	return okPayload(`edited ${args.path}`, {
		edits: applied.length,
		diff: diffLines,
	});
}

function countOccurrences(text: string, search: string): number {
	let count = 0;
	let idx = text.indexOf(search, 0);
	while (idx !== -1) {
		count++;
		idx = text.indexOf(search, idx + search.length);
	}
	return count;
}
