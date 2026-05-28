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

export function getApiKey(): string {
	prefill("api-key-input", import.meta.env.VITE_FIREWORKS_API_KEY);
	const dom = readInput("api-key-input");
	if (dom) return dom;
	const store = (
		window as unknown as Record<string, unknown>
	).__useConfigStore?.getState?.();
	return (store?.apiKey as string) || "";
}

export function getApiBaseUrl(): string {
	prefill("base-url-input", import.meta.env.VITE_FIREWORKS_BASE_URL);
	const dom = readInput("base-url-input");
	if (dom) return dom;
	const store = (
		window as unknown as Record<string, unknown>
	).__useConfigStore?.getState?.();
	return (store?.baseUrl as string) || "https://api.fireworks.ai/inference";
}

export function getModel(): string {
	prefill("model-input", import.meta.env.VITE_FIREWORKS_MODEL);
	const dom = readInput("model-input");
	if (dom) return dom;
	const store = (
		window as unknown as Record<string, unknown>
	).__useConfigStore?.getState?.();
	return (
		(store?.model as string) || "accounts/fireworks/routers/kimi-k2p6-turbo"
	);
}
