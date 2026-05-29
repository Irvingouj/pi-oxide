import { defineConfig } from "@playwright/test";

export default defineConfig({
	testDir: "./test",
	testMatch: "e2e-*.spec.ts",
	timeout: 90000,
	retries: 0,
	use: {
		headless: true,
		actionTimeout: 10000,
	},
	projects: [{ name: "chromium", use: { browserName: "chromium" } }],
});
