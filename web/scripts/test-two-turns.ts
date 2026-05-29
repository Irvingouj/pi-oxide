/**
 * Test script: verify two independent turns work correctly.
 */

import { readFileSync } from "fs";
import { createServer } from "http";
import { join } from "path";
import { chromium } from "playwright";
import { fileURLToPath } from "url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = join(__filename, "..");

const DIST_DIR = join(__dirname, "../dist");

async function main() {
	const server = createServer((req, res) => {
		const urlPath = (req.url || "/").split("?")[0];
		const filePath = join(DIST_DIR, urlPath === "/" ? "index.html" : urlPath);
		try {
			const data = readFileSync(filePath);
			const ext = filePath.endsWith(".wasm")
				? "application/wasm"
				: filePath.endsWith(".js")
					? "application/javascript"
					: filePath.endsWith(".html")
						? "text/html"
						: filePath.endsWith(".css")
							? "text/css"
							: "application/octet-stream";
			res.writeHead(200, { "Content-Type": ext });
			res.end(data);
		} catch {
			res.writeHead(404);
			res.end("Not found: " + urlPath);
		}
	});

	const baseUrl = await new Promise<string>((resolve) => {
		server.listen(0, () => {
			const addr = server.address() as { port: number };
			resolve(`http://localhost:${addr.port}`);
		});
	});

	const browser = await chromium.launch({ headless: true });
	const page = await browser.newPage();

	const apiCalls: string[] = [];

	page.on("console", (msg) => {
		const text = msg.text().slice(0, 300);
		if (
			text.includes("prompt called") ||
			text.includes("feedLlmChunk") ||
			text.includes("onLlmDone") ||
			text.includes("continueTurn")
		) {
			console.log(`[browser] ${text}`);
		}
	});

	page.on("pageerror", (err) => {
		console.log(`[page error] ${err.message}`);
	});

	let callCount = 0;
	await page.route("**/v1/messages", async (route) => {
		callCount++;
		apiCalls.push(`call-${callCount}`);
		await route.fulfill({
			status: 200,
			contentType: "application/json",
			body: JSON.stringify({
				content: [{ type: "text", text: `Response ${callCount}` }],
				stop_reason: "end_turn",
				id: `msg-${callCount}`,
				model: "test",
				usage: { input_tokens: 10, output_tokens: 5 },
			}),
		});
	});

	await page.goto(baseUrl);
	await page.waitForFunction(
		() => {
			const el = document.getElementById("wasm-status");
			return (
				el?.textContent?.includes("Ready") || el?.textContent?.includes("Error")
			);
		},
		undefined,
		{ timeout: 15000 },
	);

	await page.fill("#api-key-input", "test-key");
	await page.fill("#base-url-input", "https://test.local/inference");
	await page.fill("#model-input", "test-model");

	// Turn 1
	console.log("\n--- Turn 1 ---");
	await page.fill("#user-input", "Hello.");
	await page.click("#send-btn");
	await page.waitForFunction(
		() => {
			const btn = document.getElementById("send-btn");
			return btn && !btn.disabled;
		},
		undefined,
		{ timeout: 30000 },
	);
	const state1 = await page.evaluate(() => ({
		userMsgs: document.querySelectorAll(".msg-user").length,
		assistantMsgs: document.querySelectorAll(".msg-assistant").length,
	}));
	console.log("Turn 1 state:", state1);

	// Turn 2
	console.log("\n--- Turn 2 ---");
	await page.fill("#user-input", "World.");
	await page.click("#send-btn");
	await page.waitForFunction(
		() => {
			const btn = document.getElementById("send-btn");
			return btn && !btn.disabled;
		},
		undefined,
		{ timeout: 30000 },
	);
	const state2 = await page.evaluate(() => ({
		userMsgs: document.querySelectorAll(".msg-user").length,
		assistantMsgs: document.querySelectorAll(".msg-assistant").length,
		errorMsgs: document.querySelectorAll(".msg-error").length,
		allText: Array.from(document.querySelectorAll(".msg")).map((el) =>
			el.textContent?.slice(0, 80),
		),
	}));
	console.log("Turn 2 state:", state2);

	console.log("\nAPI calls made:", apiCalls.length);
	for (const call of apiCalls) {
		console.log(`  ${call}`);
	}

	if (apiCalls.length !== 2) {
		console.error("\nFAIL: Expected 2 API calls, got", apiCalls.length);
		process.exit(1);
	}

	if (state2.userMsgs !== 2) {
		console.error("\nFAIL: Expected 2 user messages, got", state2.userMsgs);
		process.exit(1);
	}

	if (state2.errorMsgs > 0) {
		console.error(
			"\nFAIL: Found",
			state2.errorMsgs,
			"error messages:",
			state2.allText,
		);
		process.exit(1);
	}

	console.log("\nPASS: Two turns completed successfully.");

	await browser.close();
	server.close();
}

main().catch((e) => {
	console.error(e);
	process.exit(1);
});
