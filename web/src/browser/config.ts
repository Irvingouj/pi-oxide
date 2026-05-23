/**
 * Browser config — reads API credentials from DOM inputs with Vite env fallbacks.
 *
 * In dev mode, Vite injects VITE_* env vars. In static serving, only DOM inputs work.
 */

const viteEnv: Record<string, string | undefined> =
  typeof import.meta !== "undefined" && import.meta.env ? (import.meta.env as Record<string, string>) : {};

export function getApiBaseUrl(): string {
  const custom = (document.getElementById("base-url-input") as HTMLInputElement)?.value?.trim();
  return custom || viteEnv.VITE_BASE_URL || "https://api.anthropic.com";
}

export function getApiKey(): string {
  const input = (document.getElementById("api-key-input") as HTMLInputElement)?.value?.trim();
  return input || viteEnv.VITE_API_KEY || "";
}

export function getModel(): string {
  const input = (document.getElementById("model-input") as HTMLInputElement)?.value?.trim();
  return input || viteEnv.VITE_MODEL || "claude-sonnet-4-20250514";
}

/** Pre-fill empty inputs from Vite env vars. */
export function initEnvDefaults(): void {
  const apiKey = document.getElementById("api-key-input") as HTMLInputElement;
  const baseUrl = document.getElementById("base-url-input") as HTMLInputElement;
  const model = document.getElementById("model-input") as HTMLInputElement;
  if (apiKey && !apiKey.value && viteEnv.VITE_API_KEY) apiKey.value = viteEnv.VITE_API_KEY;
  if (baseUrl && !baseUrl.value && viteEnv.VITE_BASE_URL) baseUrl.value = viteEnv.VITE_BASE_URL;
  if (model && !model.value && viteEnv.VITE_MODEL) model.value = viteEnv.VITE_MODEL;
}
