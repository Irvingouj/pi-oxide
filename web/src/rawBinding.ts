/**
 * Loader bridge: loads the CJS WASM package in an ESM context.
 */
import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
export const raw = require("../pkg/pi_host_web.cjs");
