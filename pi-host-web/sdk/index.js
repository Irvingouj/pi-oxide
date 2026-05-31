/**
 * High-level JS SDK for @pi-oxide/pi-host-web.
 *
 * Hides WASM loading and numeric handles.
 * Supports streaming LLM responses and full agent lifecycle.
 *
 * Import from the package root:
 *   import { ensureInit, toolResult } from "@pi-oxide/pi-host-web";
 */

import {
	default as init,
	initSync,
	createHostAgent,
	destroyHostAgent,
	startTurn,
	hostFeedLlmChunk,
	hostLlmDone,
	hostToolDone,
	hostAcceptCompaction,
	hostContinueTurn,
	getHostStatePersistData,
	restoreHostState,
	restoreHostStateFromJson,
	hostReadArtifact,
	hostSearchArtifacts,
	hostToolCancelled,
	hostAbort,
	getHostAgentPersistData,
	restoreHostAgent,
	createHostState,
	destroyHostState,
	hostSteer,
	hostReset,
	estimateTokens,
	estimateTokensForText,
	setLogLevel,
} from "../pi_host_web.js";

export {
	createHostState,
	destroyHostState,
	createHostAgent,
	destroyHostAgent,
	startTurn,
	hostFeedLlmChunk,
	hostLlmDone,
	hostToolDone,
	hostAcceptCompaction,
	hostContinueTurn,
	getHostStatePersistData,
	restoreHostState,
	restoreHostStateFromJson,
	hostReadArtifact,
	hostSearchArtifacts,
	hostToolCancelled,
	hostAbort,
	getHostAgentPersistData,
	restoreHostAgent,
	hostSteer,
	hostReset,
	estimateTokens,
	estimateTokensForText,
	setLogLevel,
};

let initialized = false;

/** Ensure the WASM module is loaded. Safe to call multiple times. */
export async function ensureInit() {
	if (initialized) return;
	if (typeof process !== "undefined" && process.versions?.node) {
		const { readFileSync } = await import("node:fs");
		const bytes = readFileSync(
			new URL("../pi_host_web_bg.wasm", import.meta.url),
		);
		initSync({ module: bytes });
	} else {
		await init();
	}
	initialized = true;
}

export class HostError extends Error {
	constructor(code, message) {
		super(message);
		this.code = code;
		this.name = "HostError";
	}
}

export function unwrap(result) {
	if (!result.ok) {
		throw new HostError(result.error.code, result.error.message);
	}
	return result.data;
}

/** Build a successful tool result payload. */
export function toolResult(text, opts = {}) {
	const payload = {
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
