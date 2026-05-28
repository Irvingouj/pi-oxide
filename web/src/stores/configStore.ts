import { create } from "zustand";
import { persist } from "zustand/middleware";

const viteEnv: Record<string, string | undefined> =
	typeof import.meta !== "undefined" && import.meta.env
		? (import.meta.env as Record<string, string>)
		: {};

interface ConfigStore {
	apiKey: string;
	baseUrl: string;
	model: string;

	setApiKey: (key: string) => void;
	setBaseUrl: (url: string) => void;
	setModel: (model: string) => void;
}

export const useConfigStore = create<ConfigStore>()(
	persist(
		(set) => ({
			apiKey: viteEnv.VITE_API_KEY ?? "",
			baseUrl: viteEnv.VITE_BASE_URL ?? "",
			model: viteEnv.VITE_MODEL ?? "",

			setApiKey: (apiKey) => set({ apiKey }),
			setBaseUrl: (baseUrl) => set({ baseUrl }),
			setModel: (model) => set({ model }),
		}),
		{
			name: "pi-oxide-config",
		},
	),
);

// Expose for test/debug scripts
if (typeof window !== "undefined") {
	(window as unknown as Record<string, unknown>).__useConfigStore =
		useConfigStore;
}
