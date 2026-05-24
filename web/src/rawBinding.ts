/**
 * Loader bridge: loads the ESM WASM package in Node and initializes it synchronously.
 */

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const wasmPath = join(__dirname, "../pkg/pi_host_web_bg.wasm");
const wasmBytes = readFileSync(wasmPath);

const pkg = await import("../pkg/pi_host_web.js");
pkg.initSync({ module: wasmBytes });

export const raw = pkg;
export const drainTraceLog = pkg.drainTraceLog;
