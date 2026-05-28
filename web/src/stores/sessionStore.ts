import type { SessionState } from "@pi-oxide/pi-host-web";
import { create } from "zustand";
import { IndexedDBSessionBackend } from "../browser/persistence.ts";

interface SessionStore {
	restoredState: SessionState | undefined;
	sessionId: string;

	loadSession: (sessionId: string) => Promise<void>;
	saveSession: (sessionId: string, state: SessionState) => Promise<void>;
}

const backend = new IndexedDBSessionBackend();

function isSessionState(v: unknown): v is SessionState {
	return (
		typeof v === "object" &&
		v !== null &&
		Array.isArray((v as Record<string, unknown>).entries)
	);
}

export const useSessionStore = create<SessionStore>((set) => ({
	restoredState: undefined,
	sessionId: "browser-default-session",

	loadSession: async (sessionId) => {
		try {
			const loaded = await backend.load(sessionId);
			if (loaded && isSessionState(loaded)) {
				set({ restoredState: loaded });
			} else if (loaded) {
				console.warn("Session state missing entries field, clearing");
				const empty: SessionState = { entries: [], leaf_id: "", name: "" };
				await backend.save(sessionId, empty);
				set({ restoredState: undefined });
			} else {
				set({ restoredState: undefined });
			}
		} catch {
			set({ restoredState: undefined });
		}
	},

	saveSession: async (sessionId, state) => {
		try {
			await backend.save(sessionId, state);
		} catch (e) {
			console.warn("session save failed:", e);
		}
	},
}));
