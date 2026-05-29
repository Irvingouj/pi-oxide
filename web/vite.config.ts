import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
	plugins: [react()],
	root: ".",
	envDir: "../",
	envPrefix: "VITE_",
	build: {
		outDir: "dist",
	},
	server: {
		port: 5173,
		open: true,
	},
});
