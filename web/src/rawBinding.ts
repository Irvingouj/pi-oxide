/**
 * Loader bridge: loads the ESM WASM package in Node and initializes it synchronously.
 */

import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";

const require = createRequire(import.meta.url);
const pkgDir = dirname(require.resolve("@pi-oxide/pi-host-web/package.json"));
const wasmPath = join(pkgDir, "pi_host_web_bg.wasm");
const wasmBytes = readFileSync(wasmPath);

const pkg = await import("@pi-oxide/pi-host-web");
pkg.initSync({ module: wasmBytes });

export const raw = pkg;
export const drainTraceLog = pkg.drainTraceLog;
