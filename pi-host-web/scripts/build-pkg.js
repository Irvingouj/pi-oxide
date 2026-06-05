import { cpSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const rootDir = join(__dirname, "..");
const distDir = join(rootDir, "dist");
const pkgDir = join(rootDir, "pkg");
const sourcePkgJson = join(rootDir, "package.json");

// Strip .ts extensions from .d.ts imports
function stripTsExtensions(dir) {
	for (const entry of readdirSync(dir, { withFileTypes: true })) {
		const fullPath = join(dir, entry.name);
		if (entry.isDirectory()) {
			stripTsExtensions(fullPath);
		} else if (entry.name.endsWith(".d.ts")) {
			let content = readFileSync(fullPath, "utf8");
			const original = content;
			content = content.replace(/from\s+['"](\.{1,2}\/[^'"]+)\.ts['"]/g, "from '$1'");
			content = content.replace(/import\s+['"](\.{1,2}\/[^'"]+)\.ts['"]/g, "import '$1'");
			if (content !== original) {
				writeFileSync(fullPath, content, "utf8");
			}
		}
	}
}

stripTsExtensions(distDir);

// Fix pi_host_web.js relative paths in dist/sdk/ .d.ts files
// Source files are under sdk/ but declarations are emitted to dist/sdk/,
// so every path needs one extra "../" to reach the package root.
function fixWasmImports(dir) {
	for (const entry of readdirSync(dir, { withFileTypes: true })) {
		const fullPath = join(dir, entry.name);
		if (entry.isDirectory()) {
			fixWasmImports(fullPath);
		} else if (entry.name.endsWith(".d.ts")) {
			let content = readFileSync(fullPath, "utf8");
			const original = content;
			content = content.replace(/from\s+['"]((?:\.{2}\/)+pi_host_web\.js)['"]/g, "from '../$1'");
			content = content.replace(/import\s+['"]((?:\.{2}\/)+pi_host_web\.js)['"]/g, "import '../$1'");
			if (content !== original) {
				writeFileSync(fullPath, content, "utf8");
			}
		}
	}
}

fixWasmImports(join(distDir, "sdk"));

// Clean up stale raw source and copy compiled dist to pkg/
rmSync(join(pkgDir, "sdk"), { recursive: true, force: true });
cpSync(distDir, join(pkgDir, "dist"), { recursive: true, force: true });

// Update pkg/package.json with proper exports
const pkgJsonPath = join(pkgDir, "package.json");
const pkg = JSON.parse(readFileSync(pkgJsonPath, "utf-8"));
const sourcePkg = JSON.parse(readFileSync(sourcePkgJson, "utf-8"));

// Collect all files in dist/
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

const distFiles = collectFiles(join(pkgDir, "dist"), "dist");

// Reset files to only the compiled dist and WASM bindings
pkg.files = [
	"pi_host_web_bg.wasm",
	"pi_host_web_bg.wasm.d.ts",
	"pi_host_web.js",
	"pi_host_web.d.ts",
	"README.md",
	"LICENSE",
	...distFiles,
];

pkg.main = "dist/index.js";
pkg.types = "dist/index.d.ts";

pkg.exports = {
	".": {
		import: "./dist/index.js",
		types: "./dist/index.d.ts",
	},
	"./raw": {
		import: "./pi_host_web.js",
		types: "./pi_host_web.d.ts",
	},
	"./package.json": "./package.json",
};

// Add runtime dependencies so consumers get them automatically
pkg.dependencies = {
	zod: sourcePkg.dependencies.zod,
	"zod-to-json-schema": sourcePkg.dependencies["zod-to-json-schema"],
};

writeFileSync(pkgJsonPath, JSON.stringify(pkg, null, 2) + "\n");
console.log("Package built successfully.");
