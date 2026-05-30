import type { PersistData } from "@pi-oxide/pi-host-web";
import { create } from "zustand";
import { IndexedDBSessionBackend } from "../browser/persistence.ts";

interface SessionStore {
	restoredState: PersistData | undefined;
	sessionId: string;

	loadSession: (sessionId: string) => Promise<void>;
	saveSession: (sessionId: string, state: PersistData) => Promise<void>;
}

const backend = new IndexedDBSessionBackend();

function isPersistData(v: unknown): v is PersistData {
	return (
		typeof v === "object" &&
		v !== null &&
		Array.isArray((v as Record<string, unknown>).entries) &&
		typeof (v as Record<string, unknown>).budget === "object"
	);
}

export const useSessionStore = create<SessionStore>((set) => ({
	restoredState: undefined,
	sessionId: "browser-default-session",

	loadSession: async (sessionId) => {
		try {
			const loaded = await backend.load(sessionId);
			if (loaded && isPersistData(loaded)) {
				set({ restoredState: loaded });
			} else if (loaded) {
				console.warn("Session state missing entries/budget fields, clearing");
				const empty: PersistData = {
					entries: [],
					leaf_id: "",
					name: "",
					projection_state: {
						tools: {},
						current_turn: 0,
						last_api_usage: null,
						turns_since_compaction: 0,
					},
					artifacts: [],
					budget: {
						max_tool_result_chars: 50000,
						max_context_tokens: 100000,
						microcompact_after_turns: 5,
						compaction_threshold: 0.75,
					},
					system_prompt: "",
				};
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
