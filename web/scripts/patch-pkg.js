/**
 * Post-build script for @pi-oxide/pi-host-web.
 *
 * After wasm-pack / wasm-bindgen produces `pkg/`, this script:
 * 1. Copies the SDK files recursively into `pkg/sdk/`
 * 2. Patches `pkg/package.json` to expose the SDK as the main entry point
 *    and raw bindings as the `./raw` subpath export.
 */

import {
	copyFileSync,
	mkdirSync,
	readFileSync,
	writeFileSync,
	readdirSync,
	statSync,
} from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const pkgDir = join(__dirname, "../../pi-host-web/pkg");
const sdkSourceDir = join(__dirname, "../../pi-host-web/sdk");
const sdkDir = join(pkgDir, "sdk");

mkdirSync(sdkDir, { recursive: true });

// Recursively copy SDK files
function copyDir(src, dest) {
	mkdirSync(dest, { recursive: true });
	for (const entry of readdirSync(src, { withFileTypes: true })) {
		const srcPath = join(src, entry.name);
		const destPath = join(dest, entry.name);
		if (entry.isDirectory()) {
			copyDir(srcPath, destPath);
		} else {
			copyFileSync(srcPath, destPath);
		}
	}
}

copyDir(sdkSourceDir, sdkDir);

// Copy rebuilt WASM and bindings so pi-host-web/pkg stays in sync
const webPublicPkg = join(__dirname, "../public/pkg");
copyFileSync(
	join(webPublicPkg, "pi_host_web_bg.wasm"),
	join(pkgDir, "pi_host_web_bg.wasm"),
);
copyFileSync(
	join(webPublicPkg, "pi_host_web_bg.wasm.d.ts"),
	join(pkgDir, "pi_host_web_bg.wasm.d.ts"),
);
copyFileSync(
	join(webPublicPkg, "pi_host_web.js"),
	join(pkgDir, "pi_host_web.js"),
);
copyFileSync(
	join(webPublicPkg, "pi_host_web.d.ts"),
	join(pkgDir, "pi_host_web.d.ts"),
);

// Patch package.json
const pkgJsonPath = join(pkgDir, "package.json");
const pkg = JSON.parse(readFileSync(pkgJsonPath, "utf-8"));

pkg.exports = {
	".": {
		import: "./sdk/index.ts",
		types: "./sdk/index.ts",
	},
	"./raw": {
		import: "./pi_host_web.js",
		types: "./pi_host_web.d.ts",
	},
	"./react": {
		import: "./sdk/react/index.ts",
		types: "./sdk/react/index.ts",
	},
	"./package.json": "./package.json",
};

// Ensure SDK files are included in the published tarball
function collectFiles(dir, base) {
	const files = [];
	for (const entry of readdirSync(dir, { withFileTypes: true })) {
		const rel = base ? `${base}/${entry.name}` : entry.name;
		if (entry.isDirectory()) {
			files.push(...collectFiles(join(dir, entry.name), rel));
		} else {
			files.push(rel);
		}
	}
	return files;
}

const sdkFiles = collectFiles(sdkDir, "sdk");
for (const f of sdkFiles) {
	if (!pkg.files.includes(f)) {
		pkg.files.push(f);
	}
}

writeFileSync(pkgJsonPath, JSON.stringify(pkg, null, 2) + "\n");
console.log("SDK patched into", pkgDir);

// Also sync to web/node_modules so the dev build uses the latest WASM
const nodeModulesPkg = join(__dirname, "../node_modules/@pi-oxide/pi-host-web");
try {
	const nodeModulesPkgJson = join(nodeModulesPkg, "package.json");
	const nmPkg = JSON.parse(readFileSync(nodeModulesPkgJson, "utf-8"));
	// Update exports
	nmPkg.exports = pkg.exports;
	writeFileSync(nodeModulesPkgJson, JSON.stringify(nmPkg, null, 2) + "\n");
	copyFileSync(
		join(webPublicPkg, "pi_host_web_bg.wasm"),
		join(nodeModulesPkg, "pi_host_web_bg.wasm"),
	);
	copyFileSync(
		join(webPublicPkg, "pi_host_web.js"),
		join(nodeModulesPkg, "pi_host_web.js"),
	);
	copyFileSync(
		join(webPublicPkg, "pi_host_web.d.ts"),
		join(nodeModulesPkg, "pi_host_web.d.ts"),
	);
	// Sync all SDK files to node_modules
	const nmSdkDir = join(nodeModulesPkg, "sdk");
	copyDir(sdkDir, nmSdkDir);
	console.log("SDK synced to node_modules", nodeModulesPkg);
} catch (e) {
	console.warn("Could not sync to node_modules:", e.message);
}
