// Public store API for the pi-oxide SDK.
// Stores persist opaque snapshots; they do not interpret transcript internals.

import { indexedDbStore as internalIndexedDbStore } from "./internal/stores/indexedDb.ts";
import type { AgentSnapshot, AgentStore } from "./types.ts";

export type { AgentSnapshot, AgentStore } from "./types.ts";

/**
 * Store backed by IndexedDB. Durable across page reloads.
 */
export function indexedDbStore(): AgentStore {
	return internalIndexedDbStore();
}

/**
 * In-memory store. Session data is lost when the page reloads.
 */
export function memoryStore(): AgentStore {
	const sessions = new Map<string, AgentSnapshot>();

	return {
		async loadSession(sessionId: string): Promise<AgentSnapshot | null> {
			return sessions.get(sessionId) ?? null;
		},

		async saveSession(
			sessionId: string,
			snapshot: AgentSnapshot,
		): Promise<void> {
			sessions.set(sessionId, snapshot);
		},
	};
}

/**
 * Store backed by localStorage. Simple but limited by storage quotas.
 */
export function localStorageStore(): AgentStore {
	return {
		async loadSession(sessionId: string): Promise<AgentSnapshot | null> {
			const raw = localStorage.getItem(`pi-oxide-session-${sessionId}`);
			if (!raw) return null;
			return JSON.parse(raw) as AgentSnapshot;
		},

		async saveSession(
			sessionId: string,
			snapshot: AgentSnapshot,
		): Promise<void> {
			localStorage.setItem(
				`pi-oxide-session-${sessionId}`,
				JSON.stringify(snapshot),
			);
		},
	};
}

/**
 * Store backed by an HTTP API.
 * Expects endpoints at `${baseUrl}/sessions/${sessionId}`.
 */
export function httpStore(config: { baseUrl: string }): AgentStore {
	const baseUrl = config.baseUrl.replace(/\/+$/, "");

	return {
		async loadSession(sessionId: string): Promise<AgentSnapshot | null> {
			const resp = await fetch(`${baseUrl}/sessions/${sessionId}`);
			if (!resp.ok) return null;
			return resp.json() as Promise<AgentSnapshot>;
		},

		async saveSession(
			sessionId: string,
			snapshot: AgentSnapshot,
		): Promise<void> {
			await fetch(`${baseUrl}/sessions/${sessionId}`, {
				method: "PUT",
				headers: { "Content-Type": "application/json" },
				body: JSON.stringify(snapshot),
			});
		},
	};
}
