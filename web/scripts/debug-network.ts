/**
 * Debug script — log all network requests during one agent turn.
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

  const allConsoleLogs: string[] = [];
  page.on("console", (msg) => {
    const line = `[browser ${msg.type()}] ${msg.text()}`;
    allConsoleLogs.push(line);
    console.log(line.slice(0, 300));
  });

  page.on("request", (req) => {
    console.log(`[REQUEST] ${req.method()} ${req.url().slice(0, 120)}`);
    const postData = req.postData();
    if (postData) {
      try {
        const body = JSON.parse(postData);
        console.log(`  body.tools count: ${body.tools?.length ?? 0}`);
        console.log(`  body.messages count: ${body.messages?.length ?? 0}`);
      } catch {
        console.log(`  postData: ${postData.slice(0, 200)}`);
      }
    }
  });

  page.on("response", (res) => {
    console.log(`[RESPONSE] ${res.status()} ${res.url().slice(0, 120)}`);
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

  await page.fill("#api-key-input", "fpk_8H7DNMY3mtYi1TarDJjg7F");
  await page.fill("#base-url-input", "https://api.fireworks.ai/inference");
  await page.fill("#model-input", "accounts/fireworks/routers/kimi-k2p6-turbo");

  await page.fill("#user-input", "Click the counter button.");
  await page.click("#send-btn");

  await page.waitForFunction(
    () => {
      const btn = document.getElementById("send-btn");
      return btn && !btn.disabled;
    },
    undefined,
    { timeout: 300000 }
  );

  const msgs = await page.evaluate(() =>
    Array.from(document.querySelectorAll(".msg")).map((el) => ({
      cls: el.className,
      text: el.textContent?.slice(0, 100),
    }))
  );
  console.log("Messages:", JSON.stringify(msgs, null, 2));

  console.log("\n--- All console logs ---");
  for (const log of allConsoleLogs) {
    console.log(log);
  }

  await browser.close();
  server.close();
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
