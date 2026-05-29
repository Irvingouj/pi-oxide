import { chromium } from "playwright";

const URL = "http://localhost:5174/";

async function main() {
	const browser = await chromium.launch({ headless: true });
	const context = await browser.newContext();
	const page = await context.newPage();

	const consoleLogs: string[] = [];
	page.on("console", (msg) => {
		const text = msg.text();
		consoleLogs.push(`[${msg.type()}] ${text}`);
	});

	// Mock LLM API to avoid burning credits
	await page.route("**/v1/messages", async (route) => {
		await route.fulfill({
			status: 200,
			contentType: "application/json",
			body: JSON.stringify({
				content: [{ type: "text", text: "Hello from mock LLM" }],
				stop_reason: "end_turn",
				id: "mock-msg-1",
				model: "mock",
				usage: { input_tokens: 10, output_tokens: 5 },
			}),
		});
	});

	// 1. Load page
	await page.goto(URL);
	await page.waitForSelector("#send-btn:not([disabled])", { timeout: 10000 });

	const status1 = await page.locator("#wasm-status").textContent();
	console.log("Initial status:", status1);

	// 2. Send a prompt
	await page.fill("#user-input", "say hi");
	await page.click("#send-btn");

	// Wait for assistant response to appear
	await page.waitForSelector(".msg-assistant", { timeout: 10000 });
	// Also wait for "Done" which means agentLoop finished
	await page.waitForFunction(
		() => {
			const msgs = document.querySelectorAll(".msg-assistant, .msg-error");
			return Array.from(msgs).some((m) => m.textContent?.includes("Done"));
		},
		{ timeout: 10000 },
	);

	// Small delay to let save complete
	await page.waitForTimeout(500);

	// 3. Check IndexedDB for saved session
	const sessionBefore = await page.evaluate(async () => {
		return new Promise((resolve) => {
			const req = indexedDB.open("pi-oxide-browser-agent", 2);
			req.onsuccess = () => {
				const db = req.result;
				const tx = db.transaction("sessions", "readonly");
				const store = tx.objectStore("sessions");
				const getReq = store.get("browser-default-session");
				getReq.onsuccess = () => resolve(getReq.result);
				getReq.onerror = () => resolve(null);
			};
			req.onerror = () => resolve(null);
		});
	});
	console.log("Session before reload:", JSON.stringify(sessionBefore, null, 2));

	// 4. Reload page
	await page.reload();
	await page.waitForSelector("#send-btn:not([disabled])", { timeout: 10000 });

	const status2 = await page.locator("#wasm-status").textContent();
	console.log("Status after reload:", status2);

	// 5. Verify session was restored
	if (status2?.includes("restored")) {
		console.log("✅ Session restored after reload");
	} else {
		console.log("❌ Session NOT restored after reload");
	}

	console.log("\n--- Browser console logs ---");
	for (const log of consoleLogs) {
		console.log(log);
	}

	await browser.close();
}

main().catch((e) => {
	console.error(e);
	process.exit(1);
});
