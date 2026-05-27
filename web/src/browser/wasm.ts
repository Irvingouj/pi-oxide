/**
 * Browser WASM loader — loads the wasm-bindgen --target web output.
 *
 * Provides the same raw API surface as Node's rawBinding.ts,
 * plus the async init() required by the browser WASM target.
 */

import init, * as wasm from "@pi-oxide/pi-host-web";

let initialized = false;

/** Initialize the WASM module. Must be called before any other function. */
export async function ensureInit(): Promise<void> {
	if (!initialized) {
		await init();
		initialized = true;
	}
}

/** Raw WASM exports — same shape as the Node rawBinding. */
export const raw = wasm;
