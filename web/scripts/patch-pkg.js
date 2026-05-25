/**
 * Post-build script for @pi-oxide/pi-host-web.
 *
 * After wasm-pack / wasm-bindgen produces `pkg/`, this script:
 * 1. Copies the SDK files into `pkg/sdk/`
 * 2. Patches `pkg/package.json` to expose the SDK as the main entry point
 *    and raw bindings as the `./raw` subpath export.
 */

import { copyFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const pkgDir = join(__dirname, "../../pi-host-web/pkg");
const sdkSourceDir = join(__dirname, "../../pi-host-web/sdk");
const sdkDir = join(pkgDir, "sdk");

mkdirSync(sdkDir, { recursive: true });

// Copy SDK files
copyFileSync(join(sdkSourceDir, "index.js"), join(sdkDir, "index.js"));
copyFileSync(join(sdkSourceDir, "index.d.ts"), join(sdkDir, "index.d.ts"));

// Patch package.json
const pkgJsonPath = join(pkgDir, "package.json");
const pkg = JSON.parse(readFileSync(pkgJsonPath, "utf-8"));

pkg.exports = {
  ".": {
    import: "./sdk/index.js",
    types: "./sdk/index.d.ts",
  },
  "./raw": {
    import: "./pi_host_web.js",
    types: "./pi_host_web.d.ts",
  },
  "./package.json": "./package.json",
};

// Ensure SDK files are included in the published tarball
const sdkFiles = ["sdk/index.js", "sdk/index.d.ts"];
for (const f of sdkFiles) {
  if (!pkg.files.includes(f)) {
    pkg.files.push(f);
  }
}

writeFileSync(pkgJsonPath, JSON.stringify(pkg, null, 2) + "\n");
console.log("SDK patched into", pkgDir);
