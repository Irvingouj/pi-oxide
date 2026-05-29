import { create } from "zustand";

export type MessageType = "user" | "assistant" | "tool" | "error" | "steer";

export interface ChatMessage {
	id: string;
	type: MessageType;
	text?: string;
	toolName?: string;
	toolCallId?: string;
	toolResult?: string;
}

interface AgentStore {
	messages: ChatMessage[];
	isRunning: boolean;
	status: string;
	error: string | null;

	addMessage: (msg: ChatMessage) => void;
	appendToLastAssistant: (text: string) => void;
	removeEmptyAssistant: () => void;
	setToolResult: (toolCallId: string, result: string) => void;
	setRunning: (v: boolean) => void;
	setStatus: (s: string) => void;
	setError: (e: string | null) => void;
	clearMessages: () => void;
}

export const useAgentStore = create<AgentStore>((set) => ({
	messages: [],
	isRunning: false,
	status: "Loading WASM...",
	error: null,

	addMessage: (msg) => set((state) => ({ messages: [...state.messages, msg] })),

	appendToLastAssistant: (text) =>
		set((state) => {
			const msgs = [...state.messages];
			const last = msgs[msgs.length - 1];
			if (last && last.type === "assistant") {
				last.text = (last.text ?? "") + text;
			}
			return { messages: msgs };
		}),

	removeEmptyAssistant: () =>
		set((state) => {
			const msgs = [...state.messages];
			const last = msgs[msgs.length - 1];
			if (last && last.type === "assistant" && !last.text) {
				msgs.pop();
			}
			return { messages: msgs };
		}),

	setToolResult: (toolCallId: string, result: string) =>
		set((state) => {
			const msgs = [...state.messages];
			const idx = msgs.findIndex((m) => m.toolCallId === toolCallId);
			if (idx !== -1) {
				msgs[idx] = { ...msgs[idx], toolResult: result };
			}
			return { messages: msgs };
		}),

	setRunning: (v) => set({ isRunning: v }),
	setStatus: (s) => set({ status: s }),
	setError: (e) => set({ error: e }),
	clearMessages: () => set({ messages: [], error: null }),
}));

// Expose for test/debug scripts
if (typeof window !== "undefined") {
	(window as unknown as Record<string, unknown>).__useAgentStore =
		useAgentStore;
}
