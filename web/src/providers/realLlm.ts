/**
 * Async agent host that drives the WASM Rust agent with a real LLM provider.
 *
 * The WASM functions are synchronous, but the LLM provider call is async (fetch).
 * This host mirrors AgentHost but wraps the provider call in async/await.
 */

import {
  createAgent,
  destroyAgent,
  feedLlmChunk,
  onLlmDone,
  onToolDone,
  prompt,
  type AgentAction,
  type AgentOptions,
} from "../wasmBinding.ts";
import type { ToolRegistry } from "../fakeTools.ts";
import type { LlmRequest } from "./types.ts";
import { callAnthropic, type AnthropicConfig } from "./anthropic.ts";

export interface TraceEntry {
  phase: "action" | "event" | "host";
  type: string;
  data: unknown;
}

export interface RealRunResult {
  terminalAction: AgentAction;
  trace: TraceEntry[];
  handle: number;
}

export class RealLlm {
  private config: AnthropicConfig;
  public readonly log: string[] = [];

  constructor(config: AnthropicConfig) {
    this.config = config;
  }

  async call(request: LlmRequest): Promise<{ chunks: object[]; llmResult: object }> {
    const result = await callAnthropic(request, this.config);
    this.log.push(...result.log);
    return result;
  }
}

export class RealAgentHost {
  readonly trace: TraceEntry[] = [];
  readonly llm: RealLlm;
  readonly tools: ToolRegistry;

  constructor(llm: RealLlm, tools: ToolRegistry) {
    this.llm = llm;
    this.tools = tools;
  }

  private log(phase: TraceEntry["phase"], type: string, data: unknown): void {
    this.trace.push({ phase, type, data });
  }

  async run(options: AgentOptions, userPrompt: string): Promise<RealRunResult> {
    const handle = createAgent(options);
    this.log("host", "create_agent", { handle });

    this.log("host", "prompt", { text: userPrompt });
    const step = prompt(handle, userPrompt);
    for (const event of step.events) {
      this.log("event", event.type, event);
    }

    const terminalAction = await this.processActions(handle, step.actions);
    return { terminalAction, trace: this.trace, handle };
  }

  cleanup(handle: number): void {
    destroyAgent(handle);
    this.log("host", "destroy_agent", { handle });
  }

  private async processActions(handle: number, actions: AgentAction[]): Promise<AgentAction> {
    for (const action of actions) {
      this.log("action", action.type, action);

      switch (action.type) {
        case "stream_llm":
          return this.handleStreamLlm(handle, action.context);
        case "execute_tools":
          return this.handleExecuteTools(handle, action.calls);
        case "finished":
          return action;
        case "wait_for_input":
          return action;
      }
    }
    return { type: "finished", messages: [] } as AgentAction;
  }

  private async handleStreamLlm(
    handle: number,
    context: { system_prompt: string; messages: unknown[]; tools: unknown[] },
  ): Promise<AgentAction> {
    const request: LlmRequest = {
      system_prompt: context.system_prompt,
      messages: context.messages as import("./types.ts").AgentMessageShape[],
      tools: context.tools as import("../tools/schemas.ts").ToolDefinition[],
    };

    const result = await this.llm.call(request);

    // Feed streaming chunks
    for (const chunk of result.chunks) {
      this.log("host", "feed_llm_chunk", chunk);
      const chunkResult = feedLlmChunk(handle, chunk);
      for (const event of chunkResult.events) {
        this.log("event", event.type, event);
      }
    }

    // Feed final result
    this.log("host", "llm_result", result.llmResult);
    const step = onLlmDone(handle, result.llmResult);
    for (const event of step.events) {
      this.log("event", event.type, event);
    }

    return this.processActions(handle, step.actions);
  }

  private async handleExecuteTools(
    handle: number,
    calls: import("../wasmBinding.ts").ToolCall[],
  ): Promise<AgentAction> {
    let lastActions: AgentAction[] = [];

    for (const call of calls) {
      const toolResultPayload = this.tools.execute(call);
      this.log("host", "tool_done", { tool_call_id: call.id, tool_name: call.name, payload: toolResultPayload });

      const step = onToolDone(handle, call.id, toolResultPayload);
      for (const event of step.events) {
        this.log("event", event.type, event);
      }
      lastActions = step.actions;
    }

    return this.processActions(handle, lastActions);
  }
}
