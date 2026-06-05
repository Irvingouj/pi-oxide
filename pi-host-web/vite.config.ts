import { resolve } from "node:path";
import { defineConfig } from "vite";
import dts from "vite-plugin-dts";

export default defineConfig({
	build: {
		lib: {
			entry: resolve(__dirname, "sdk/index.ts"),
			formats: ["es"],
			fileName: "index",
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
