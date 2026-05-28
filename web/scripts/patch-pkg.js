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

// Copy rebuilt WASM and bindings so pi-host-web/pkg stays in sync
const webPublicPkg = join(__dirname, "../public/pkg");
copyFileSync(
  join(webPublicPkg, "pi_host_web_bg.wasm"),
  join(pkgDir, "pi_host_web_bg.wasm")
);
copyFileSync(
  join(webPublicPkg, "pi_host_web_bg.wasm.d.ts"),
  join(pkgDir, "pi_host_web_bg.wasm.d.ts")
);
copyFileSync(
  join(webPublicPkg, "pi_host_web.js"),
  join(pkgDir, "pi_host_web.js")
);
copyFileSync(
  join(webPublicPkg, "pi_host_web.d.ts"),
  join(pkgDir, "pi_host_web.d.ts")
);

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

// Also sync to web/node_modules so the dev build uses the latest WASM
const nodeModulesPkg = join(__dirname, "../node_modules/@pi-oxide/pi-host-web");
try {
  const nodeModulesPkgJson = join(nodeModulesPkg, "package.json");
  const nmPkg = JSON.parse(readFileSync(nodeModulesPkgJson, "utf-8"));
  // Update exports if needed
  if (!nmPkg.exports || !nmPkg.exports["."]) {
    nmPkg.exports = pkg.exports;
    writeFileSync(nodeModulesPkgJson, JSON.stringify(nmPkg, null, 2) + "\n");
  }
  copyFileSync(
    join(webPublicPkg, "pi_host_web_bg.wasm"),
    join(nodeModulesPkg, "pi_host_web_bg.wasm")
  );
  copyFileSync(
    join(webPublicPkg, "pi_host_web.js"),
    join(nodeModulesPkg, "pi_host_web.js")
  );
  copyFileSync(
    join(webPublicPkg, "pi_host_web.d.ts"),
    join(nodeModulesPkg, "pi_host_web.d.ts")
  );
  copyFileSync(
    join(sdkDir, "index.js"),
    join(nodeModulesPkg, "sdk/index.js")
  );
  copyFileSync(
    join(sdkDir, "index.d.ts"),
    join(nodeModulesPkg, "sdk/index.d.ts")
  );
  console.log("SDK synced to node_modules", nodeModulesPkg);
} catch (e) {
  console.warn("Could not sync to node_modules:", e.message);
}
