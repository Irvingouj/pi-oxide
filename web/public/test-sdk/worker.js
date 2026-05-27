import { Agent } from "./sdk/index.js";

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
main();
