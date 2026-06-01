// Model exports — AgentModel interface and defineModel() factory.
// Provider-neutral contract. Provider factories live in internal/providers/.

import type { AgentModel, ModelRequest, ModelResponse } from "./types.ts";

export type { AgentModel } from "./types.ts";

/**
 * Create a custom AgentModel from a user-provided generate function.
 * Useful for wrapping arbitrary LLM providers or mocking in tests.
 */
export function defineModel(
  config: {
    id?: string;
    contextWindow?: number;
    maxTokens?: number;
    capabilities?: AgentModel["capabilities"];
    generate: AgentModel["generate"];
    summarize?: AgentModel["summarize"];
  },
): AgentModel {
  return {
    id: config.id ?? "custom-model",
    contextWindow: config.contextWindow ?? 100000,
    maxTokens: config.maxTokens ?? 4096,
    capabilities: {
      vision: config.capabilities?.vision ?? false,
      jsonMode: config.capabilities?.jsonMode ?? true,
      functionCalling: config.capabilities?.functionCalling ?? true,
      streaming: config.capabilities?.streaming ?? true,
    },
    generate: config.generate,
    summarize: config.summarize,
  };
}
