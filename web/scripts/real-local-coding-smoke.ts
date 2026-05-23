/**
 * Real local coding-agent smoke test — Milestone 6.
 *
 * Proves a functional coding agent on a real computer by giving it a creative
 * multi-file task: build a landing page for the City of Ottawa from scratch
 * and serve it using Python.
 *
 * Uses:
 * - real Anthropic-compatible provider
 * - real local filesystem tools (read, write, edit)
 * - real local bash execution
 * - Rust/WASM context projection via projectContext
 *
 * Requires ANTHROPIC_API_KEY (and optionally ANTHROPIC_BASE_URL, ANTHROPIC_MODEL).
 * Exits with skip (0) when ANTHROPIC_API_KEY is not set.
 * Exits non-zero on failure.
 */

import * as fs from "node:fs";
import * as path from "node:path";
import * as os from "node:os";

import { RealAgentHost, RealLlm } from "../src/providers/realLlm.ts";
import { LocalToolRegistry } from "../src/local/localToolRegistry.ts";
import { PI_CODING_TOOLS } from "../src/tools/schemas.ts";
import { MemoryArtifactStore } from "../src/context/rustProjection.ts";

const apiKey = process.env.ANTHROPIC_API_KEY;
if (!apiKey) {
  console.error("SKIP: ANTHROPIC_API_KEY is not set. Set it to run the real local coding smoke test.");
  process.exit(0);
}

const baseUrl = process.env.ANTHROPIC_BASE_URL ?? "https://api.anthropic.com";
const model = process.env.ANTHROPIC_MODEL ?? "claude-sonnet-4-20250514";

async function main() {
  console.log("Real local coding-agent smoke test — Milestone 6");
  console.log("Task: Build an Ottawa landing page and serve it with Python");
  console.log(`  base_url: ${baseUrl}`);
  console.log(`  model:    ${model}`);
  console.log();

  // Create empty project directory
  const projectDir = fs.mkdtempSync(path.join(os.tmpdir(), "pi-oxide-ottawa-"));
  console.log(`Project dir: ${projectDir}`);
  console.log();

  // Set up tools — real local filesystem + bash
  const registry = new LocalToolRegistry({
    cwd: projectDir,
    bashPolicy: { mode: "unrestricted" },
  });

  // Set up LLM with Rust context projection
  const artifacts = new MemoryArtifactStore();
  const llm = new RealLlm(
    { apiKey, baseUrl, model },
    {
      budget: {
        max_tool_result_chars: 50_000,
        max_context_tokens: 100_000,
        default_preview_chars: 2000,
      },
      state: { replacements: {} },
      artifacts,
    },
  );

  const host = new RealAgentHost(llm, registry);

  const options = {
    system_prompt:
      "You are a coding agent working on a real filesystem. You can read and write files, " +
      "apply edits, and run shell commands. All file paths are relative to the project root. " +
      "Create files, then use bash to verify your work. Keep responses concise.",
    model: {
      id: model,
      name: model,
      api: "anthropic",
      provider: "anthropic",
      reasoning: false,
      context_window: 128000,
      max_tokens: 4096,
    },
    thinking_level: "off" as const,
    tools: PI_CODING_TOOLS,
  };

  const prompt =
    "Create a landing page for the City of Ottawa. The page should include:\n" +
    "- A nice HTML file (index.html) with an Ottawa-themed design (parliament, Rideau Canal, etc.)\n" +
    "- A CSS file for styling\n" +
    "- Some real content about Ottawa (population, attractions, history)\n\n" +
    "Once you've created the files, start a local HTTP server using Python so the page can be viewed:\n" +
    "  python3 -m http.server 8421 &\n" +
    "Then use curl to fetch the page and verify it loads correctly:\n" +
    "  curl -s http://localhost:8421/index.html | head -20\n" +
    "Kill the server when done: kill $!";

  try {
    const result = await host.run(options, prompt);

    // --- Print trace ---
    console.log("=== Trace ===");
    for (const entry of result.trace) {
      if (entry.phase === "action") {
        console.log(`  [action] ${entry.type}`);
      } else if (entry.phase === "event") {
        console.log(`  [event]  ${entry.type}`);
      } else {
        const data = entry.data as Record<string, unknown>;
        if (entry.type === "tool_done") {
          const payload = data.payload as Record<string, unknown> | undefined;
          const isError = payload && "error" in payload;
          const content = payload?.content as Array<{ text: string }> | undefined;
          const preview = content?.[0]?.text?.slice(0, 120).replace(/\n/g, "\\n") ?? "";
          console.log(`  [host]   ${entry.type} (${data.tool_name as string})${isError ? " [ERROR]" : ""}${preview ? " -> " + preview : ""}`);
        } else {
          console.log(`  [host]   ${entry.type}`);
        }
      }
    }
    console.log("=== End Trace ===");
    console.log();

    // --- Print context projection logs ---
    const projectionLogs = llm.log.filter((l) => l.startsWith("context_projection:"));
    if (projectionLogs.length > 0) {
      console.log("Context projection:");
      for (const log of projectionLogs) {
        console.log(`  ${log}`);
      }
    }
    console.log();

    // --- Print artifact store ---
    console.log(`Artifacts stored: ${artifacts["store"] instanceof Map ? artifacts["store"].size : "N/A"}`);
    console.log();

    console.log(`Terminal action: ${result.terminalAction.type}`);
    console.log();

    // --- Print files created ---
    console.log("Files created:");
    function listDir(dir: string, prefix = "") {
      const entries = fs.readdirSync(dir, { withFileTypes: true });
      for (const entry of entries) {
        if (entry.name.startsWith(".") || entry.name === "node_modules") continue;
        const fullPath = path.join(dir, entry.name);
        if (entry.isDirectory()) {
          console.log(`  ${prefix}${entry.name}/`);
          listDir(fullPath, prefix + "  ");
        } else {
          const size = fs.statSync(fullPath).size;
          console.log(`  ${prefix}${entry.name} (${size} bytes)`);
        }
      }
    }
    listDir(projectDir);
    console.log();

    // --- Deterministic verification ---

    const toolDoneEntries = result.trace.filter(
      (e) => e.phase === "host" && e.type === "tool_done",
    );
    const toolNames = toolDoneEntries.map(
      (e) => (e.data as { tool_name: string }).tool_name,
    );

    let failed = false;

    // 1. Terminal action is finished
    if (result.terminalAction.type !== "finished") {
      console.error(`❌ FAIL: terminal action is ${result.terminalAction.type}, expected finished`);
      failed = true;
    }

    // 2. Trace includes write (must create files)
    if (!toolNames.includes("write")) {
      console.error("❌ FAIL: trace does not include a write tool call");
      failed = true;
    }

    // 3. Trace includes bash (must run server + curl)
    if (!toolNames.includes("bash")) {
      console.error("❌ FAIL: trace does not include a bash tool call");
      failed = true;
    }

    // 4. index.html exists and has real content
    const indexPath = path.join(projectDir, "index.html");
    if (fs.existsSync(indexPath)) {
      const html = fs.readFileSync(indexPath, "utf-8");
      const hasOttawa = html.toLowerCase().includes("ottawa");
      const hasHtml = html.includes("<html") || html.includes("<!DOCTYPE");
      const hasBody = html.includes("<body") || html.includes("<body>");
      const size = html.length;

      console.log(`index.html: ${size} bytes, has <html>: ${hasHtml}, has <body>: ${hasBody}, mentions Ottawa: ${hasOttawa}`);

      if (!hasHtml) {
        console.error("❌ FAIL: index.html doesn't look like valid HTML");
        failed = true;
      }
      if (!hasOttawa) {
        console.error("❌ FAIL: index.html doesn't mention Ottawa");
        failed = true;
      }
      if (hasHtml && hasOttawa) {
        console.log("✅ index.html is valid HTML about Ottawa");
      }
    } else {
      console.error("❌ FAIL: index.html was not created");
      failed = true;
    }

    // 5. Context projection ran
    if (projectionLogs.length > 0) {
      console.log(`✅ Context projection ran (${projectionLogs.length} time(s))`);
    } else {
      console.error("❌ FAIL: context projection did not run");
      failed = true;
    }

    console.log();
    console.log(`Tools used: ${toolNames.join(", ")}`);
    console.log(`Tool calls: ${toolNames.length}`);

    host.cleanup(result.handle);

    // Kill any leftover python servers
    try {
      const { execFileSync } = await import("node:child_process");
      execFileSync("sh", ["-c", "pkill -f 'python3 -m http.server 8421' 2>/dev/null || true"], { timeout: 3000 });
    } catch {
      // ignore
    }

    if (failed) {
      console.error();
      console.error("❌ SMOKE TEST FAILED");
      process.exit(1);
    }

    console.log();
    console.log("✅ SMOKE TEST PASSED");
  } catch (err) {
    console.error("❌ ERROR:", err);
    process.exit(1);
  } finally {
    // Clean up
    fs.rmSync(projectDir, { recursive: true, force: true });
  }
}

main();
