import { resolve } from "node:path";
import { defineConfig } from "vite";
import dts from "vite-plugin-dts";

export default defineConfig({
	build: {
		lib: {
			entry: {
				index: resolve(__dirname, "sdk/index.ts"),
				bindings: resolve(__dirname, "sdk/bindings/index.ts"),
			},
			formats: ["es"],
			fileName: (format, entryName) => (entryName === "index" ? "index.js" : `sdk/${entryName}/index.js`),
		},
		rollupOptions: {
			external: ["zod", "zod-to-json-schema", /^node:.*/],
		},
		outDir: "dist",
		emptyOutDir: true,
	},
	plugins: [
		dts({
			include: ["sdk/**/*"],
			exclude: ["node_modules", "dist", "pkg"],
			insertTypesEntry: true,
		}),
	],
});
