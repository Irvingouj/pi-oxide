/**
 * Post-build script for @pi-oxide/pi-host-web.
 *
 * After wasm-pack / wasm-bindgen and the SDK build produce `pkg/`, this script
 * syncs the built package to `web/node_modules/@pi-oxide/pi-host-web` so the
 * dev build uses the latest WASM + SDK without a separate npm install.
 */

import { cpSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const pkgDir = join(__dirname, "../../pi-host-web/pkg");
const webPublicPkg = join(__dirname, "../public/pkg");

// Copy rebuilt WASM and bindings so pi-host-web/pkg stays in sync
const filesToSync = [
	"pi_host_web_bg.wasm",
	"pi_host_web_bg.wasm.d.ts",
	"pi_host_web.js",
	"pi_host_web.d.ts",
];

for (const file of filesToSync) {
	cpSync(
		join(webPublicPkg, file),
		join(pkgDir, file),
		{ force: true },
	);
}

// Sync to web/node_modules so the dev build uses the latest package
const nodeModulesPkg = join(__dirname, "../node_modules/@pi-oxide/pi-host-web");

cpSync(pkgDir, nodeModulesPkg, { recursive: true, force: true });
console.log("SDK synced to node_modules", nodeModulesPkg);
