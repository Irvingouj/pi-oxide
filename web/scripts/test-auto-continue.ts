/**
 * Test script: verify auto-continue after tool execution.
 *
 * Mocks LLM responses to trigger a tool call, then verifies that
 * continueTurn() is called and a second LLM API request is made.
 */

import { chromium } from "playwright";
import { createServer } from "http";
import { readFileSync } from "fs";
import { join } from "path";
import { fileURLToPath } from "url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = join(__filename, "..");

const DIST_DIR = join(__dirname, "../dist");

async function main() {
  const server = createServer((req, res) => {
    const urlPath = (req.url || "/").split("?")[0];
    let filePath = join(DIST_DIR, urlPath === "/" ? "index.html" : urlPath);
    try {
      const data = readFileSync(filePath);
      const ext = filePath.endsWith(".wasm") ? "application/wasm" :
                  filePath.endsWith(".js") ? "application/javascript" :
                  filePath.endsWith(".html") ? "text/html" :
                  filePath.endsWith(".css") ? "text/css" : "application/octet-stream";
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

  const apiCalls: Array<{ method: string; url: string; body: unknown }> = [];

  page.on("console", (msg) => {
    console.log(`[browser ${msg.type()}] ${msg.text().slice(0, 300)}`);
  });

  page.on("pageerror", (err) => {
    console.log(`[page error] ${err.message}`);
  });

  // Mock LLM API: first call triggers browser_get_page, second call returns text
  let callCount = 0;
  await page.route("**/v1/messages", async (route) => {
    const request = route.request();
    const postData = request.postData();
    let body: unknown = null;
    try { body = JSON.parse(postData || "{}"); } catch { /* ignore */ }
    apiCalls.push({ method: request.method(), url: request.url(), body });

    callCount++;
    if (callCount === 1) {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          content: [
            { type: "text", text: "I'll get the page state for you." },
            { type: "tool_use", id: "call-1", name: "browser_get_page", input: {} },
          ],
          stop_reason: "tool_use",
          id: "msg-1",
          model: "test",
          usage: { input_tokens: 10, output_tokens: 20 },
        }),
      });
    } else {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          content: [{ type: "text", text: `Page state received. Call count: ${callCount}` }],
          stop_reason: "end_turn",
          id: "msg-2",
          model: "test",
          usage: { input_tokens: 15, output_tokens: 10 },
        }),
      });
    }
  });

  await page.goto(baseUrl);
  await page.waitForFunction(
    () => {
      const el = document.getElementById("wasm-status");
      return el?.textContent?.includes("Ready") || el?.textContent?.includes("Error");
    },
    undefined,
    { timeout: 15000 }
  );

  await page.fill("#api-key-input", "test-key");
  await page.fill("#base-url-input", "https://test.local/inference");
  await page.fill("#model-input", "test-model");

  await page.fill("#user-input", "Describe the page.");
  await page.click("#send-btn");

  await page.waitForFunction(
    () => {
      const btn = document.getElementById("send-btn");
      return btn && !btn.disabled;
    },
    undefined,
    { timeout: 30000 }
  );

  const msgs = await page.evaluate(() =>
    Array.from(document.querySelectorAll(".msg")).map((el) => ({
      cls: el.className,
      text: el.textContent?.slice(0, 100),
    }))
  );
  console.log("Messages:", JSON.stringify(msgs, null, 2));

  console.log("\nAPI calls made:", apiCalls.length);
  for (let i = 0; i < apiCalls.length; i++) {
    console.log(`  Call ${i + 1}: ${apiCalls[i].method} ${apiCalls[i].url}`);
  }

  if (apiCalls.length < 2) {
    console.error("\nFAIL: Expected at least 2 API calls (tool_use + continue), but got", apiCalls.length);
    process.exit(1);
  }

  console.log("\nPASS: Auto-continue works — second API call was made after tool execution.");

  await browser.close();
  server.close();
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
