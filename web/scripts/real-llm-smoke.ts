/**
 * Real LLM smoke test.
 *
 * Creates an in-memory workspace with a file, asks the model to read it
 * and make a small change, then prints the trace and final workspace contents.
 *
 * Requires ANTHROPIC_API_KEY (and optionally ANTHROPIC_BASE_URL, ANTHROPIC_MODEL).
 * Exits non-zero on failure.
 */

import { RealAgentHost, RealLlm } from "../src/providers/realLlm.ts";
import { CodingToolRegistry, MemoryWorkspace } from "../src/tools/codingTools.ts";
import { CODING_TOOLS } from "../src/tools/schemas.ts";

const apiKey = process.env.ANTHROPIC_API_KEY;
if (!apiKey) {
  console.error("SKIP: ANTHROPIC_API_KEY is not set. Set it to run the real LLM smoke test.");
  process.exit(0);
}

const baseUrl = process.env.ANTHROPIC_BASE_URL ?? "https://api.anthropic.com";
const model = process.env.ANTHROPIC_MODEL ?? "claude-sonnet-4-20250514";

async function main() {
  console.log(`Real LLM smoke test`);
  console.log(`  base_url: ${baseUrl}`);
  console.log(`  model:    ${model}`);
  console.log();

  // Set up workspace
  const workspace = new MemoryWorkspace();
  workspace.writeFile("hello.txt", "Hello, World!");
  workspace.writeFile("README.md", "# Smoke Test\n\nA simple project for testing.");
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
      "You are a coding agent. You can read and write files in the workspace. " +
      "When asked to modify a file, read it first, then write the updated version. " +
      "Keep responses concise.",
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
    tools: CODING_TOOLS,
  };

  try {
    const result = await host.run(
      options,
      "Read hello.txt and change 'World' to 'Rust'."
    );

    console.log("=== Trace ===");
    for (const entry of result.trace) {
      if (entry.phase === "action") {
        console.log(`  [action] ${entry.type}`);
      } else if (entry.phase === "event") {
        console.log(`  [event]  ${entry.type}`);
      } else {
        console.log(`  [host]   ${entry.type}`);
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

    // Verify the change was made
    const content = workspace.readFile("hello.txt") ?? "";
    if (content.includes("Hello, Rust!")) {
      console.log("✅ SUCCESS: hello.txt was updated to 'Hello, Rust!'");
    } else {
      console.error(`❌ FAIL: hello.txt content is: "${content}"`);
      console.error("Expected it to contain 'Hello, Rust!'");
      host.cleanup(result.handle);
      process.exit(1);
    }

    host.cleanup(result.handle);
  } catch (err) {
    console.error("❌ ERROR:", err);
    process.exit(1);
  }
}

main();
