/**
 * Browser config — reads API credentials from DOM inputs with Vite env fallbacks.
 */

/** Read an input value by id, falling back to empty string. */
function readInput(id: string): string {
	const el = document.getElementById(id) as HTMLInputElement | null;
	return el?.value?.trim() ?? "";
}

/** Pre-fill empty inputs from Vite env vars. */
function prefill(id: string, envValue: string | undefined) {
	if (!envValue) return;
	const el = document.getElementById(id) as HTMLInputElement | null;
	if (el && !el.value.trim()) {
		el.value = envValue;
	}
}

function getConfigStore():
	| { apiKey?: string; baseUrl?: string; model?: string }
	| undefined {
	const store = (window as unknown as Record<string, unknown>).__useConfigStore;
	if (typeof store === "object" && store !== null && "getState" in store) {
		return (
			store as {
				getState(): { apiKey?: string; baseUrl?: string; model?: string };
			}
		).getState();
	}
	return undefined;
}

export function getApiKey(): string {
	prefill("api-key-input", import.meta.env.VITE_FIREWORKS_API_KEY);
	const dom = readInput("api-key-input");
	if (dom) return dom;
	return getConfigStore()?.apiKey || "";
}

export function getApiBaseUrl(): string {
	prefill("base-url-input", import.meta.env.VITE_FIREWORKS_BASE_URL);
	const dom = readInput("base-url-input");
	if (dom) return dom;
	return getConfigStore()?.baseUrl || "https://api.fireworks.ai/inference";
}

export function getModel(): string {
	prefill("model-input", import.meta.env.VITE_FIREWORKS_MODEL);
	const dom = readInput("model-input");
	if (dom) return dom;
	return (
		getConfigStore()?.model || "accounts/fireworks/routers/kimi-k2p6-turbo"
	);
}
