import { cpSync, readdirSync, readFileSync, rmSync, writeFileSync } from "fs";
import { dirname, join } from "path";
import { fileURLToPath } from "url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const rootDir = join(__dirname, "..");
const distDir = join(rootDir, "dist");
const pkgDir = join(rootDir, "pkg");

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

// Clean up stale pkg/sdk/ and copy dist to pkg/
rmSync(join(pkgDir, "sdk"), { recursive: true, force: true });
cpSync(distDir, join(pkgDir, "dist"), { recursive: true, force: true });

console.log("Package built successfully.");
