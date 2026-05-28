/**
 * Record a real multi-turn browser agent conversation.
 *
 * Starts a local HTTP server, opens the browser agent page,
 * intercepts LLM API calls to record responses, sends 10 prompts,
 * and writes recordings to test/fixtures/ for replay tests.
 *
 * Requires FIREWORKS_API_KEY (or ANTHROPIC_API_KEY) env var.
 */

import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { chromium } from "playwright";
import {
	createStaticServer,
	ensureFixturesDir,
	extractPromptContext,
	FIXTURES_DIR,
	getServerBaseUrl,
	type Recording,
} from "./lib/testHarness.ts";

const API_KEY =
	process.env.FIREWORKS_API_KEY || process.env.ANTHROPIC_API_KEY || "";
const BASE_URL =
	process.env.FIREWORKS_BASE_URL ||
	process.env.ANTHROPIC_BASE_URL ||
	"https://api.fireworks.ai/inference";
const MODEL =
	process.env.FIREWORKS_MODEL ||
	process.env.ANTHROPIC_MODEL ||
	"accounts/fireworks/routers/kimi-k2p6-turbo";

if (!API_KEY) {
	console.error("SKIP: Set FIREWORKS_API_KEY or ANTHROPIC_API_KEY to record.");
	process.exit(0);
}

const PROMPTS = [
	"Say hello.",
	"Click the counter button (the one that says 'Click me').",
	"What is the counter value now? Check it for me.",
	"Type 'Alice' into the Name input field.",
	"What are the current todo items on the page?",
	"Evaluate the JavaScript expression 2**10 and tell me the result.",
	"Add a new todo item 'Buy groceries' by clicking the Add button.",
	"Get the recent console logs.",
	"Query all buttons on this page and tell me how many there are.",
	"Submit the contact form by clicking the Submit button.",
];

async function main() {
	ensureFixturesDir();

	const server = createStaticServer();
	const baseUrl = await getServerBaseUrl(server);

	const browser = await chromium.launch({ headless: true });
	const context = await browser.newContext();
	const page = await context.newPage();

	const consoleLogs: string[] = [];
	page.on("console", (msg) => {
		const text = msg.text();
		consoleLogs.push(`[${msg.type()}] ${text.slice(0, 200)}`);
	});
	page.on("pageerror", (err) => {
		console.log(`[page error] ${err.message}`);
	});

	const recording: Recording = {
		meta: {
			model: MODEL,
			baseUrl: BASE_URL,
			date: new Date().toISOString(),
			prompts: PROMPTS,
		},
		responses: [],
	};

	let requestIndex = 0;
	await page.route("**/v1/messages", async (route) => {
		const promptContext = extractPromptContext(route.request().postData());
		console.log(
			`[API request ${requestIndex}] ${promptContext.slice(0, 80)}...`,
		);

		const response = await route.fetch();
		const body = await response.json();

		recording.responses.push({
			requestIndex: requestIndex++,
			promptContext,
			response: body,
		});

		await route.fulfill({
			status: response.status(),
			contentType: "application/json",
			body: JSON.stringify(body),
		});
	});

	await page.goto(baseUrl);

	await page.waitForFunction(
		() => {
			const el = document.querySelector("span");
			return (
				el?.textContent?.includes("Ready") ||
				el?.textContent?.includes("Session restored")
			);
		},
		{ timeout: 15000 },
	);

	await page.evaluate(
		({ key, url, model }) => {
			const store = (window as any).__useConfigStore?.getState?.();
			if (store) {
				store.setApiKey(key);
				store.setBaseUrl(url);
				store.setModel(model);
			}
			const keyInput = document.getElementById(
				"api-key-input",
			) as HTMLInputElement | null;
			const urlInput = document.getElementById(
				"base-url-input",
			) as HTMLInputElement | null;
			const modelInput = document.getElementById(
				"model-input",
			) as HTMLInputElement | null;
			if (keyInput) keyInput.value = key;
			if (urlInput) urlInput.value = url;
			if (modelInput) modelInput.value = model;
		},
		{ key: API_KEY, url: BASE_URL, model: MODEL },
	);

	for (let i = 0; i < PROMPTS.length; i++) {
		const prompt = PROMPTS[i];
		console.log(`\n--- Turn ${i + 1}/${PROMPTS.length}: ${prompt} ---`);

		await page.fill("#user-input", prompt);
		await page.click("#send-btn");

		await page.waitForFunction(
			() => {
				const panel = document.querySelector("[data-running]");
				return panel && panel.getAttribute("data-running") === "false";
			},
			undefined,
			{ timeout: 120000 },
		);

		const counts = await page.evaluate(() => ({
			user: document.querySelectorAll(".msg-user").length,
			assistant: document.querySelectorAll(".msg-assistant").length,
			tool: document.querySelectorAll(".msg-tool").length,
		}));
		console.log(
			`  Messages: ${counts.user} user, ${counts.assistant} assistant, ${counts.tool} tool`,
		);
	}

	const recordingPath = join(FIXTURES_DIR, "multi-turn-recording.json");
	writeFileSync(recordingPath, JSON.stringify(recording, null, 2));
	console.log(`\nRecording saved: ${recordingPath}`);
	console.log(`  Total API calls: ${recording.responses.length}`);
	console.log(`  Total prompts: ${PROMPTS.length}`);

	const logPath = join(FIXTURES_DIR, "multi-turn-console-logs.txt");
	writeFileSync(logPath, consoleLogs.join("\n"));
	console.log(`Console logs saved: ${logPath}`);

	await browser.close();
	server.close();
}

main().catch((e) => {
	console.error(e);
	process.exit(1);
});
