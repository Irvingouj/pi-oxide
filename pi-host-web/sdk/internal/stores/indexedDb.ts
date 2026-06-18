// IndexedDB wrapper — converts internal IndexedDBSessionBackend to public AgentStore.

import type { PersistData } from "../../../pi_host_web.js";
import type { AgentSnapshot, AgentStore } from "../../types.ts";
import { IndexedDBSessionBackend } from "./persistence.ts";

export function indexedDbStore(): AgentStore {
	const backend = new IndexedDBSessionBackend();

	return {
		async loadSession(sessionId: string): Promise<AgentSnapshot | null> {
			const data = await backend.load(sessionId);
			if (!data) return null;
			return { version: 1, data } as AgentSnapshot;
		},

		async saveSession(
			sessionId: string,
			snapshot: AgentSnapshot,
		): Promise<void> {
			await backend.save(sessionId, snapshot.data as unknown as PersistData);
		},

		// Artifact methods are not supported by the raw IndexedDBSessionBackend.
		// If artifact support is needed, use a store that implements them.
	};
}
