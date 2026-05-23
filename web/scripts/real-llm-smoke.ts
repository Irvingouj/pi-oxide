/**
 * Real LLM smoke test — programming task.
 *
 * Creates an in-memory workspace with a buggy TypeScript project, asks the
 * model to fix the bug and run tests, then verifies the fix.
 *
 * Uses the pi-compatible tool surface: read, write, edit, bash.
 *
 * Requires ANTHROPIC_API_KEY (and optionally ANTHROPIC_BASE_URL, ANTHROPIC_MODEL).
 * Exits with skip (0) when ANTHROPIC_API_KEY is not set.
 * Exits non-zero on failure.
 */

import { RealAgentHost, RealLlm } from "../src/providers/realLlm.ts";
import { CodingToolRegistry, MemoryWorkspace } from "../src/tools/codingTools.ts";
import { PI_CODING_TOOLS } from "../src/tools/schemas.ts";

const apiKey = process.env.ANTHROPIC_API_KEY;
if (!apiKey) {
  console.error("SKIP: ANTHROPIC_API_KEY is not set. Set it to run the real LLM smoke test.");
  process.exit(0);
}

const baseUrl = process.env.ANTHROPIC_BASE_URL ?? "https://api.anthropic.com";
const model = process.env.ANTHROPIC_MODEL ?? "claude-sonnet-4-20250514";

async function main() {
  console.log("Real LLM smoke test — programming task");
  console.log(`  base_url: ${baseUrl}`);
  console.log(`  model:    ${model}`);
  console.log();

  // Set up workspace: buggy add function
  const workspace = new MemoryWorkspace();
  workspace.writeFile(
    "package.json",
    JSON.stringify({ name: "calc", scripts: { test: "node --test" } }),
  );
  workspace.writeFile(
    "src/index.ts",
    "export function add(a: number, b: number): number {\n  return a - b;\n}\n",
  );

  console.log("Initial workspace:");
  for (const [path, content] of workspace["files"] as Map<string, string>) {
    console.log(`  ${path}: ${content.length} bytes`);
  }
  console.log();

  // Set up tools and LLM
  const registry = new CodingToolRegistry(workspace);
  const llm = new RealLlm({ apiKey, baseUrl, model });
  const host = new RealAgentHost(llm, registry);

  const options = {
    system_prompt:
      "You are a coding agent. You can read and write files, apply edits, and run tests. " +
      "When asked to fix a bug, read the file first, then apply the fix using the edit tool, " +
      "then run tests to verify. Keep responses concise.",
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

  try {
    const result = await host.run(
      options,
      "The add function in src/index.ts has a bug: it returns a - b instead of a + b. " +
      "Read the file, fix the bug, then run npm test to verify.",
    );

    console.log("=== Trace ===");
    for (const entry of result.trace) {
      if (entry.phase === "action") {
        console.log(`  [action] ${entry.type}`);
      } else if (entry.phase === "event") {
        console.log(`  [event]  ${entry.type}`);
      } else {
        const data = entry.data as Record<string, unknown>;
        if (entry.type === "tool_done") {
          console.log(`  [host]   ${entry.type} (${data.tool_name as string})`);
        } else {
          console.log(`  [host]   ${entry.type}`);
        }
      }
    }
    console.log("=== End Trace ===");
    console.log();

    console.log(`Terminal action: ${result.terminalAction.type}`);
    console.log();

    console.log("Final workspace:");
    for (const [path, content] of workspace["files"] as Map<string, string>) {
      console.log(`  ${path}:`);
      console.log(`    ${content}`);
    }
    console.log();

    // --- Deterministic verification ---

    const toolDoneEntries = result.trace.filter(
      (e) => e.phase === "host" && e.type === "tool_done",
    );
    const toolNames = toolDoneEntries.map(
      (e) => (e.data as { tool_name: string }).tool_name,
    );

    // 1. Terminal action is finished
    if (result.terminalAction.type !== "finished") {
      console.error(`❌ FAIL: terminal action is ${result.terminalAction.type}, expected finished`);
      host.cleanup(result.handle);
      process.exit(1);
    }

    // 2. Trace includes read
    if (!toolNames.includes("read")) {
      console.error("❌ FAIL: trace does not include a read tool call");
      host.cleanup(result.handle);
      process.exit(1);
    }

    // 3. Trace includes edit or write
    if (!toolNames.includes("edit") && !toolNames.includes("write")) {
      console.error("❌ FAIL: trace does not include an edit or write tool call");
      host.cleanup(result.handle);
      process.exit(1);
    }

    // 4. Trace includes bash
    if (!toolNames.includes("bash")) {
      console.error("❌ FAIL: trace does not include a bash tool call");
      host.cleanup(result.handle);
      process.exit(1);
    }

    // 5. Workspace contains the fix
    const srcContent = workspace.readFile("src/index.ts") ?? "";
    if (srcContent.includes("return a + b")) {
      console.log("✅ SUCCESS: src/index.ts was fixed to return a + b");
    } else {
      console.error(`❌ FAIL: src/index.ts content is: "${srcContent}"`);
      console.error("Expected it to contain 'return a + b'");
      host.cleanup(result.handle);
      process.exit(1);
    }

    console.log(`  Tools used: ${toolNames.join(", ")}`);
    host.cleanup(result.handle);
  } catch (err) {
    console.error("❌ ERROR:", err);
    process.exit(1);
  }
}

main();
