/**
 * Browser host — drives the Rust WASM agent lifecycle with browser-native tools.
 *
 * Wraps RealAgentHost with browser tool execution through a BrowserRuntime adapter.
 * Uses Rust context projection via projectContext for model context management.
 *
 * Host-owned — no browser APIs in pi-core.
 */

import {
  RealAgentHost,
  RealLlm,
  type SessionBackend,
  type TraceEntry,
  type ContextProjectionConfig,
} from "../providers/realLlm.ts";
import { BrowserToolRegistry } from "./browserTools.ts";
import { BROWSER_TOOLS } from "./browserTools.ts";
import { MemoryArtifactStore } from "../context/rustProjection.ts";
import type { BrowserRuntime } from "./browserRuntime.ts";
import type { AgentOptions } from "../wasmBinding.ts";

export interface BrowserHostOptions {
  runtime: BrowserRuntime;
  /** If true, browser tools will also be available as model tools. Default: true. */
  includeTools?: boolean;
  /** Optional session persistence backend. */
  sessionBackend?: SessionBackend;
}

export class BrowserHost {
  readonly host: RealAgentHost;
  readonly tools: BrowserToolRegistry;
  readonly llm: RealLlm;
  private readonly sessionBackend?: SessionBackend;

  constructor(options: BrowserHostOptions, llm: RealLlm) {
    this.tools = new BrowserToolRegistry(options.runtime);
    this.llm = llm;
    this.host = new RealAgentHost(llm, this.tools);
    this.sessionBackend = options.sessionBackend;
  }

  async run(options: AgentOptions, userPrompt: string) {
    return this.host.run(options, userPrompt, this.sessionBackend);
  }

  cleanup(handle: number): void {
    this.host.cleanup(handle);
  }
}

/**
 * Build browser AgentOptions with browser tool definitions.
 */
export function browserAgentOptions(
  overrides: Partial<AgentOptions> & Pick<AgentOptions, "system_prompt" | "model">,
): AgentOptions {
  return {
    thinking_level: "off",
    tools: BROWSER_TOOLS,
    ...overrides,
  };
}
