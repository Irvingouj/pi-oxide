/**
 * Replay test for multi-turn browser agent conversation.
 *
 * Starts a local HTTP server, opens the browser agent page,
 * mocks LLM API calls by replaying recorded responses,
 * sends all 10 prompts, and verifies completion.
 */

import { chromium } from "playwright";
import {
	createStaticServer,
	extractPromptContext,
	getServerBaseUrl,
	loadRecording,
	type Recording,
} from "./lib/testHarness.ts";

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
	let recording: Recording;
	try {
		recording = loadRecording("./test/fixtures/multi-turn-recording.json");
	} catch (_e) {
		console.error("SKIP: Recording not found. Run record-multi-turn.ts first.");
		process.exit(0);
	}

	if (recording.responses.length === 0) {
		console.error("SKIP: Recording has no responses.");
		process.exit(0);
	}

	const server = createStaticServer();
	const baseUrl = await getServerBaseUrl(server);

	const browser = await chromium.launch({ headless: true });
	const context = await browser.newContext();
	const page = await context.newPage();

	const consoleLogs: string[] = [];
	const pageErrors: string[] = [];
	page.on("console", (msg) => {
		const text = msg.text();
		consoleLogs.push(`[${msg.type()}] ${text.slice(0, 200)}`);
		if (msg.type() === "error") {
			pageErrors.push(text);
		}
	});
	page.on("pageerror", (err) => {
		pageErrors.push(err.message);
	});

	let replayIndex = 0;
	await page.route("**/v1/messages", async (route) => {
		const promptContext = extractPromptContext(route.request().postData());

		if (replayIndex >= recording.responses.length) {
			console.error(
				`[REPLAY ERROR] Ran out of responses at index ${replayIndex}`,
			);
			await route.fallback();
			return;
		}

		const recorded = recording.responses[replayIndex];
		console.log(`[Replay ${replayIndex}] ${promptContext.slice(0, 80)}...`);
		replayIndex++;

		await route.fulfill({
			status: 200,
			contentType: "application/json",
			body: JSON.stringify(recorded.response),
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
		({ url, model }) => {
			const store = (window as any).__useConfigStore?.getState?.();
			if (store) {
				store.setApiKey("replay-key");
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
			if (keyInput) keyInput.value = "replay-key";
			if (urlInput) urlInput.value = url;
			if (modelInput) modelInput.value = model;
		},
		{ url: recording.meta.baseUrl, model: recording.meta.model },
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
			error: document.querySelectorAll(".msg-error").length,
		}));
		console.log(
			`  Messages: ${counts.user} user, ${counts.assistant} assistant, ${counts.tool} tool, ${counts.error} error`,
		);

		if (counts.error > 0) {
			console.error(`  ERROR: ${counts.error} error message(s) found in UI`);
		}
	}

	const finalCounts = await page.evaluate(() => ({
		user: document.querySelectorAll(".msg-user").length,
		assistant: document.querySelectorAll(".msg-assistant").length,
		tool: document.querySelectorAll(".msg-tool").length,
		error: document.querySelectorAll(".msg-error").length,
	}));

	console.log(`\n=== Replay Results ===`);
	console.log(
		`  Replayed API calls: ${replayIndex} / ${recording.responses.length}`,
	);
	console.log(`  Total user messages: ${finalCounts.user}`);
	console.log(`  Total assistant messages: ${finalCounts.assistant}`);
	console.log(`  Total tool messages: ${finalCounts.tool}`);
	console.log(`  Total error messages: ${finalCounts.error}`);
	console.log(`  Page errors: ${pageErrors.length}`);

	let exitCode = 0;
	if (finalCounts.user !== PROMPTS.length) {
		console.error(
			`  FAIL: Expected ${PROMPTS.length} user messages, got ${finalCounts.user}`,
		);
		exitCode = 1;
	}
	if (finalCounts.error > 0) {
		console.error(`  FAIL: ${finalCounts.error} error messages found`);
		exitCode = 1;
	}
	if (pageErrors.length > 0) {
		console.error(`  FAIL: ${pageErrors.length} page errors occurred`);
		for (const e of pageErrors.slice(0, 5)) {
			console.error(`    - ${e.slice(0, 200)}`);
		}
		exitCode = 1;
	}
	if (replayIndex < recording.responses.length) {
		console.error(
			`  FAIL: Only used ${replayIndex} of ${recording.responses.length} recorded responses`,
		);
		exitCode = 1;
	}
	if (exitCode === 0) {
		console.log(`  PASS: All turns completed successfully`);
	}

	await browser.close();
	server.close();
	process.exit(exitCode);
}

main().catch((e) => {
	console.error(e);
	process.exit(1);
});
