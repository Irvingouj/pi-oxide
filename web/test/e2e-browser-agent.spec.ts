/**
 * End-to-end browser agent test using Playwright.
 *
 * Starts a local HTTP server, opens the browser agent page,
 * configures the Fireworks API, and verifies the agent can
 * interact with the demo page through browser tools.
 */

import { test, expect, type Page } from "@playwright/test";
import { createServer } from "http";
import { readFileSync, copyFileSync, mkdirSync, writeFileSync } from "fs";
import { join, dirname } from "path";
import { execSync } from "child_process";
import { fileURLToPath } from "url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

const FIREWORKS_API_KEY = process.env.FIREWORKS_API_KEY || "";
const FIREWORKS_BASE_URL = process.env.FIREWORKS_BASE_URL || "https://api.fireworks.ai/inference";
const FIREWORKS_MODEL = process.env.FIREWORKS_MODEL || "accounts/fireworks/routers/kimi-k2p6-turbo";

const hasApiKey = FIREWORKS_API_KEY.length > 0;

const PUBLIC_DIR = join(__dirname, "../public");
const WASM_PATH = join(__dirname, "../pkg/pi_host_web_bg.wasm");

let server: ReturnType<typeof createServer>;
let baseUrl: string;

test.beforeAll(async () => {
  // Build web-target WASM into public/pkg/
  const projectRoot = join(__dirname, "../../");
  execSync(`cd "${projectRoot}" && wasm-bindgen --target web --out-dir web/public/pkg target/wasm32-unknown-unknown/release/pi_host_web.wasm`, { stdio: "pipe" });

  // Copy SDK into public/ so Worker tests can import it via relative URL
  const sdkSourceDir = join(__dirname, "../node_modules/@pi-oxide/pi-host-web");
  const sdkDestDir = join(PUBLIC_DIR, "test-sdk");
  mkdirSync(join(sdkDestDir, "sdk"), { recursive: true });
  copyFileSync(join(sdkSourceDir, "pi_host_web.js"), join(sdkDestDir, "pi_host_web.js"));
  copyFileSync(join(sdkSourceDir, "pi_host_web_bg.wasm"), join(sdkDestDir, "pi_host_web_bg.wasm"));
  copyFileSync(join(sdkSourceDir, "sdk/index.js"), join(sdkDestDir, "sdk/index.js"));

  // Worker script that imports the SDK from a real URL (not Blob) so import.meta.url resolves correctly
  writeFileSync(
    join(sdkDestDir, "worker.js"),
    `import { Agent } from "./sdk/index.js";

async function main() {
  try {
    const agent = await Agent.create({
      system_prompt: "test",
      model: {
        id: "test", name: "Test", provider: "test", api: "test",
        reasoning: false, context_window: 4096, max_tokens: 1024,
        capabilities: { vision: false, json_mode: true, function_calling: true, streaming: true },
        cost: { input: 0, output: 0, cache_read: 0, cache_write: 0 },
      },
      tools: [],
    });
    self.postMessage({ success: true });
  } catch (e) {
    self.postMessage({ success: false, error: String(e) });
  }
}
main();\n`
  );

  server = createServer((req, res) => {
    // Strip query string
    const urlPath = (req.url || "/").split("?")[0];
    let filePath = join(PUBLIC_DIR, urlPath === "/" ? "index.html" : urlPath);
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

  await new Promise<void>((resolve) => {
    server.listen(0, () => {
      const addr = server.address() as { port: number };
      baseUrl = `http://localhost:${addr.port}`;
      resolve();
    });
  });
});

test.afterAll(async () => {
  server?.close();
});

test.describe("Browser Agent E2E", () => {
  let page: Page;

  test.beforeEach(async ({ browser: pwBrowser }) => {
    page = await pwBrowser.newPage();

    // Collect console logs
    page.on("console", (msg) => {
      console.log(`[browser ${msg.type()}] ${msg.text().slice(0, 200)}`);
    });
    page.on("pageerror", (err) => {
      console.log(`[page error] ${err.message}`);
    });

    await page.goto(baseUrl);
    // Wait for WASM to load
    await page.waitForSelector("#wasm-status", { state: "visible" });
    await page.waitForFunction(() => {
      const el = document.getElementById("wasm-status");
      return el?.textContent?.includes("ready") || el?.textContent?.includes("Error");
    }, { timeout: 15000 });
  });

  test.afterEach(async () => {
    await page.close();
  });

  test("WASM loads and agent initializes", async () => {
    const status = await page.textContent("#wasm-status");
    expect(status).toContain("ready");
  });

  test("agent can describe the page", async () => {
    // Configure API
    await page.fill("#api-key-input", FIREWORKS_API_KEY);
    await page.fill("#base-url-input", FIREWORKS_BASE_URL);
    await page.fill("#model-input", FIREWORKS_MODEL);

    // Send prompt
    await page.fill("#user-input", "What's on this page? Describe what you see.");
    await page.click("#send-btn");

    // Wait for agent to respond (up to 30s for LLM + tool calls)
    await page.waitForFunction(() => {
      const msgs = document.querySelectorAll(".msg-assistant");
      return msgs.length >= 1 && !document.querySelector(".msg-loading");
    }, { timeout: 60000 });

    // Check that we got an assistant response
    const msgs = await page.locator(".msg-assistant").count();
    expect(msgs).toBeGreaterThanOrEqual(1);

    // Check that tools were used
    const toolMsgs = await page.locator(".msg-tool").count();
    expect(toolMsgs).toBeGreaterThanOrEqual(1);

    // Verify tool names appear
    const toolNames = await page.evaluate(() => {
      return Array.from(document.querySelectorAll(".msg-tool .tool-name")).map(el => el.textContent);
    });
    expect(toolNames.some(n => n?.includes("browser_"))).toBe(true);
  });

  test("agent can click the counter", async () => {
    await page.fill("#api-key-input", FIREWORKS_API_KEY);
    await page.fill("#base-url-input", FIREWORKS_BASE_URL);
    await page.fill("#model-input", FIREWORKS_MODEL);

    // Verify counter starts at 0
    const before = await page.textContent("#counter-display");
    expect(before).toBe("0");

    // Ask agent to click the counter
    await page.fill("#user-input", "Click the counter button (the one that says 'Click me').");
    await page.click("#send-btn");

    // Wait for user message to appear (confirms send worked)
    await page.waitForSelector(".msg-user", { timeout: 5000 });

    // Wait for agent to finish — check for send button being re-enabled
    await page.waitForFunction(() => {
      const btn = document.getElementById("send-btn");
      return btn && !btn.disabled && !document.querySelector(".msg-loading");
    }, { timeout: 90000 });

    // Debug: log all messages
    const allMsgs = await page.evaluate(() => {
      return Array.from(document.querySelectorAll(".msg")).map(el => ({
        cls: el.className,
        text: el.textContent?.slice(0, 200),
      }));
    });
    console.log("All messages:", JSON.stringify(allMsgs, null, 2));

    // Take screenshot for debugging
    await page.screenshot({ path: "test-results/counter-test.png" });

    // Counter should have increased
    const after = await page.textContent("#counter-display");
    expect(Number(after)).toBeGreaterThan(0);
  });

  test("agent can type into a form field", async () => {
    await page.fill("#api-key-input", FIREWORKS_API_KEY);
    await page.fill("#base-url-input", FIREWORKS_BASE_URL);
    await page.fill("#model-input", FIREWORKS_MODEL);

    await page.fill("#user-input", "Type 'Agent Test' into the Name input field.");
    await page.click("#send-btn");

    await page.waitForFunction(() => {
      const done = document.querySelectorAll(".msg-assistant");
      return done.length >= 1 && !document.querySelector(".msg-loading");
    }, { timeout: 60000 });

    // Check the name input has been filled
    const nameValue = await page.inputValue("#demo-name");
    expect(nameValue.toLowerCase()).toContain("agent test");
  });

  test("agent can evaluate JavaScript", async () => {
    await page.fill("#api-key-input", FIREWORKS_API_KEY);
    await page.fill("#base-url-input", FIREWORKS_BASE_URL);
    await page.fill("#model-input", FIREWORKS_MODEL);

    await page.fill("#user-input", "Run this JavaScript: 2**10 and tell me the result.");
    await page.click("#send-btn");

    await page.waitForFunction(() => {
      const done = document.querySelectorAll(".msg-assistant");
      return done.length >= 1 && !document.querySelector(".msg-loading");
    }, { timeout: 60000 });

    // The agent should mention 1024 somewhere
    const texts = await page.evaluate(() => {
      return Array.from(document.querySelectorAll(".msg-assistant, .msg-tool")).map(el => el.textContent);
    });
    const combined = texts.join(" ");
    expect(combined).toContain("1024");
  });

  test("IndexedDB persistence stores messages", async () => {
    await page.fill("#api-key-input", FIREWORKS_API_KEY);
    await page.fill("#base-url-input", FIREWORKS_BASE_URL);
    await page.fill("#model-input", FIREWORKS_MODEL);

    await page.fill("#user-input", "Hello, just say 'Hi back' and nothing else.");
    await page.click("#send-btn");

    await page.waitForFunction(() => {
      const done = document.querySelectorAll(".msg-assistant");
      return done.length >= 1 && !document.querySelector(".msg-loading");
    }, { timeout: 60000 });

    // Check IndexedDB has session entries
    const dbEntries = await page.evaluate(async () => {
      return new Promise((resolve) => {
        const req = indexedDB.open('pi-oxide-browser-agent', 1);
        req.onsuccess = () => {
          const db = req.result;
          const tx = db.transaction('session', 'readonly');
          const store = tx.objectStore('session');
          const getAllReq = store.getAll();
          getAllReq.onsuccess = () => resolve(getAllReq.result);
          getAllReq.onerror = () => resolve([]);
        };
        req.onerror = () => resolve([]);
      });
    });

    expect(dbEntries.length).toBeGreaterThanOrEqual(2); // at least user + assistant
    const roles = (dbEntries as Array<{ role: string }>).map(e => e.role);
    expect(roles).toContain("user");
    expect(roles).toContain("assistant");
  });

});

test("SDK initializes in a Web Worker without hitting node:fs", async ({ page }) => {
  await page.goto(baseUrl);

  const result = await page.evaluate(async (workerUrl) => {
    const worker = new Worker(workerUrl, { type: "module" });

    return new Promise<{ success: boolean; error?: string }>((resolve) => {
      worker.onmessage = (e) => resolve(e.data);
      worker.onerror = (e) => resolve({ success: false, error: e.message });
    });
  }, `${baseUrl}/test-sdk/worker.js`);

  expect(result.success).toBe(true);
});

