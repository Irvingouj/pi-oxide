// WASM initialization and low-level helpers.
// Kept separate from index.ts to avoid circular imports with internal modules.

import { default as init, initSync, setLogLevel as setWasmLogLevel } from "../pi_host_web.js";
import { setGlobalLogLevel } from "./internal/logger.ts";
import type { LogLevel } from "./types.ts";

let initialized = false;

/** Ensure the WASM module is loaded. Safe to call multiple times. */
export async function ensureInit() {
	if (initialized) return;
	if (typeof process !== "undefined" && process.versions?.node) {
		const { readFileSync } = await import("node:fs");
		const bytes = readFileSync(new URL("../pi_host_web_bg.wasm", import.meta.url));
		initSync({ module: bytes });
	} else {
		await init();
	}
	initialized = true;
}

/** Set the log level for both JS SDK and WASM core. */
export function setLogLevel(level: LogLevel) {
	setGlobalLogLevel(level);
	try {
		setWasmLogLevel(level);
	} catch {
		// WASM may not be initialized yet; level will be set on init
	}
}

export class HostError extends Error {
	code: string;
	constructor(code: string, message: string) {
		super(message);
		this.code = code;
		this.name = "HostError";
	}
}

export function unwrap(result: { ok: boolean; data?: unknown; error?: { code: string; message: string } }): unknown {
	if (!result.ok) {
		throw new HostError(result.error?.code, result.error?.message);
	}
	return result.data;
}

/** Build a successful tool result payload. */
export function toolResult(text: string, opts: { terminate?: boolean; details?: Record<string, unknown> } = {}) {
	const payload: {
		content: Array<{ type: "text"; text: string }>;
		terminate?: boolean;
		details?: Record<string, unknown>;
	} = {
		content: [{ type: "text", text }],
	};
	if (opts.terminate) {
		payload.terminate = true;
	}
	if (opts.details) {
		payload.details = opts.details;
	}
	return payload;
}
