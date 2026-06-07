import { create } from "zustand";

interface UIStore {
	consoleLines: string[];
	activeDemoTab: string | null;
	isSidebarOpen: boolean;

	addConsoleLine: (line: string) => void;
	clearConsole: () => void;
	setActiveDemoTab: (tab: string | null) => void;
	setSidebarOpen: (open: boolean) => void;
}

export const useUIStore = create<UIStore>((set) => ({
	consoleLines: ["Ready."],
	activeDemoTab: null,
	isSidebarOpen: false,

	addConsoleLine: (line) =>
		set((state) => ({ consoleLines: [...state.consoleLines, line] })),
	clearConsole: () => set({ consoleLines: [] }),
	setActiveDemoTab: (tab) => set({ activeDemoTab: tab }),
	setSidebarOpen: (open) => set({ isSidebarOpen: open }),
}));
