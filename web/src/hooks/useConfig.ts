/**
 * useConfig hook — reads config from Zustand store and syncs to DOM inputs / services.
 */

import { useConfigStore } from "../stores/configStore.ts";

export function useConfig() {
	return useConfigStore();
}
