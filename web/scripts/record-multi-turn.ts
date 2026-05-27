/**
 * Record a real multi-turn browser agent conversation.
 *
 * Starts a local HTTP server, opens the browser agent page,
 * intercepts LLM API calls to record responses, sends 10 prompts,
 * and writes recordings to test/fixtures/ for replay tests.
 *
 * Requires FIREWORKS_API_KEY (or ANTHROPIC_API_KEY) env var.
 */

import { chromium } from "playwright";
import { createServer } from "http";
import { readFileSync, mkdirSync, writeFileSync } from "fs";
import { join, dirname } from "path";
import { fileURLToPath } from "url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

const API_KEY = process.env.FIREWORKS_API_KEY || process.env.ANTHROPIC_API_KEY || "";
const BASE_URL = process.env.FIREWORKS_BASE_URL || process.env.ANTHROPIC_BASE_URL || "https://api.fireworks.ai/inference";
const MODEL = process.env.FIREWORKS_MODEL || process.env.ANTHROPIC_MODEL || "accounts/fireworks/routers/kimi-k2p6-turbo";

if (!API_KEY) {
  console.error("SKIP: Set FIREWORKS_API_KEY or ANTHROPIC_API_KEY to record.");
  process.exit(0);
}

const DIST_DIR = join(__dirname, "../dist");
const FIXTURES_DIR = join(__dirname, "../test/fixtures");

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

interface Recording {
  meta: {
    model: string;
    baseUrl: string;
    date: string;
    prompts: string[];
  };
  responses: Array<{
    requestIndex: number;
    promptContext: string;
    response: unknown;
  }>;
}

async function main() {
  mkdirSync(FIXTURES_DIR, { recursive: true });

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

  // Intercept LLM API calls and record responses
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
    const request = route.request();
    const postData = request.postData();
    let promptContext = "";
    try {
      const body = JSON.parse(postData || "{}");
      const lastMsg = body.messages?.at(-1);
      if (lastMsg?.role === "user") {
        promptContext = typeof lastMsg.content === "string"
          ? lastMsg.content
          : JSON.stringify(lastMsg.content).slice(0, 200);
      }
    } catch { /* ignore */ }

    console.log(`[API request ${requestIndex}] ${promptContext.slice(0, 80)}...`);

    // Forward to real API
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

  // Wait for WASM
  await page.waitForSelector("#wasm-status", { state: "visible" });
  await page.waitForFunction(() => {
    const el = document.getElementById("wasm-status");
    return el?.textContent?.includes("Ready") || el?.textContent?.includes("Error");
  }, { timeout: 15000 });

  // Configure API
  await page.fill("#api-key-input", API_KEY);
  await page.fill("#base-url-input", BASE_URL);
  await page.fill("#model-input", MODEL);

  for (let i = 0; i < PROMPTS.length; i++) {
    const prompt = PROMPTS[i];
    console.log(`\n--- Turn ${i + 1}/${PROMPTS.length}: ${prompt} ---`);

    await page.fill("#user-input", prompt);
    await page.click("#send-btn");

    // Wait for send button to be re-enabled (agent finished)
    await page.waitForFunction(
      () => {
        const btn = document.getElementById("send-btn");
        return btn && !btn.disabled && !document.querySelector(".msg-loading");
      },
      undefined,
      { timeout: 120000 }
    );

    // Log current message counts
    const counts = await page.evaluate(() => ({
      user: document.querySelectorAll(".msg-user").length,
      assistant: document.querySelectorAll(".msg-assistant").length,
      tool: document.querySelectorAll(".msg-tool").length,
    }));
    console.log(`  Messages: ${counts.user} user, ${counts.assistant} assistant, ${counts.tool} tool`);
  }

  // Save recording
  const recordingPath = join(FIXTURES_DIR, "multi-turn-recording.json");
  writeFileSync(recordingPath, JSON.stringify(recording, null, 2));
  console.log(`\nRecording saved: ${recordingPath}`);
  console.log(`  Total API calls: ${recording.responses.length}`);
  console.log(`  Total prompts: ${PROMPTS.length}`);

  // Save console logs for debugging
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
