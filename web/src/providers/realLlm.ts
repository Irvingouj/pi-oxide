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
  onToolStarted,
  onToolUpdate,
  onToolCancelled,
  prompt,
  type AgentAction,
  type AgentOptions,
  type ToolCall,
  type ToolExecutionUpdate as ToolExecutionUpdateShape,
  type CancelReason,
} from "../wasmBinding.ts";
import type { ToolRegistry } from "../fakeTools.ts";
import type { ToolRuntime, ToolUpdate } from "../local/toolRuntime.ts";
import type { LlmRequest } from "./types.ts";
import { callAnthropic, type AnthropicConfig } from "./anthropic.ts";
import {
  callProjectContext,
  type ArtifactStore,
  type ContextProjectionBudget,
  type ContextProjectionState,
  MemoryArtifactStore,
} from "../context/rustProjection.ts";

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

export interface ContextProjectionConfig {
  budget: ContextProjectionBudget;
  state: ContextProjectionState;
  artifacts: ArtifactStore;
}

export class RealLlm {
  private config: AnthropicConfig;
  private contextProjection?: ContextProjectionConfig;
  public readonly log: string[] = [];

  constructor(config: AnthropicConfig, contextProjection?: ContextProjectionConfig) {
    this.config = config;
    this.contextProjection = contextProjection;
  }

  async call(request: LlmRequest): Promise<{ chunks: object[]; llmResult: object }> {
    let effectiveRequest = request;

    if (this.contextProjection) {
      const result = callProjectContext(
        request.system_prompt,
        request.messages,
        this.contextProjection.budget,
        this.contextProjection.state,
      );

      // Update state for next turn
      this.contextProjection.state = result.updated_state;

      // Store artifacts for any replacements
      for (const replacement of result.report.replacements) {
        // Find the original tool result to get the full content
        const originalMsg = request.messages.find(
          (m) => m.role === "tool_result" && m.tool_call_id === replacement.tool_call_id,
        );
        if (originalMsg && originalMsg.role === "tool_result") {
          const text = originalMsg.content
            .filter((b) => b.type === "text" && b.text !== undefined)
            .map((b) => b.text!)
            .join("\n");
          this.contextProjection.artifacts.put({
            id: replacement.artifact_id,
            toolName: replacement.tool_name,
            toolCallId: replacement.tool_call_id,
            content: text,
            storedAt: Date.now(),
          });
        }
      }

      this.log.push(
        `context_projection: estimated=${result.report.estimated_tokens} replacements=${result.report.replacements.length} dropped=${result.report.dropped_messages}`,
      );

      effectiveRequest = {
        system_prompt: request.system_prompt,
        messages: result.projected_messages,
        tools: request.tools,
      };
    }

    const result = await callAnthropic(effectiveRequest, this.config);
    this.log.push(...result.log);
    return result;
  }
}

export class RealAgentHost {
  readonly trace: TraceEntry[] = [];
  readonly llm: RealLlm;
  readonly tools: ToolRegistry;
  /** If provided, tool execution uses the async ToolRuntime path with streaming updates. */
  private readonly runtime?: ToolRuntime;

  constructor(llm: RealLlm, tools: ToolRegistry, runtime?: ToolRuntime) {
    this.llm = llm;
    this.tools = tools;
    this.runtime = runtime;
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
        case "cancel_tools":
          return this.handleCancelTools(handle, action.tool_call_ids, action.reason);
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
    calls: ToolCall[],
  ): Promise<AgentAction> {
    if (this.runtime) {
      return this.handleAsyncTools(handle, calls);
    }
    return this.handleSyncTools(handle, calls);
  }

  /**
   * Sync tool execution path — used when no ToolRuntime is provided.
   * Matches the original behavior for backward compatibility.
   */
  private async handleSyncTools(
    handle: number,
    calls: ToolCall[],
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

  /**
   * Async tool execution path — uses ToolRuntime with streaming updates.
   *
   * For each tool call:
   * 1. Call onToolStarted() to notify Rust
   * 2. Execute tool asynchronously through ToolRuntime
   * 3. Streaming updates (stdout/stderr) are forwarded to Rust via onToolUpdate()
   * 4. On completion, call onToolDone() to finalize in Rust
   *
   * Cancellation is handled via cancel_tools action from Rust.
   */
  private async handleAsyncTools(
    handle: number,
    calls: ToolCall[],
  ): Promise<AgentAction> {
    // Wire the runtime's streaming updates into Rust via onToolUpdate
    this.runtime!.hostUpdateListener = (update: ToolUpdate) => {
      const rustUpdate: ToolExecutionUpdateShape = {
        tool_call_id: update.toolCallId,
        stream: update.stream,
        chunk: update.chunk,
        sequence: update.sequence,
        timestamp: Date.now(),
      };
      const updateOutput = onToolUpdate(handle, rustUpdate);
      for (const event of updateOutput.events) {
        this.log("event", event.type, event);
      }
    };

    // Start all tools concurrently
    const toolPromises = calls.map(async (call) => {
      // 1. Notify Rust that this tool has started
      const startedOutput = onToolStarted(handle, call.id);
      for (const event of startedOutput.events) {
        this.log("event", event.type, event);
      }

      // 2. Execute the tool asynchronously
      const toolResultPayload = await this.runtime!.execute(call);
      this.log("host", "tool_done", { tool_call_id: call.id, tool_name: call.name, payload: toolResultPayload });

      return { call, payload: toolResultPayload };
    });

    // Wait for all tools to complete
    const results = await Promise.all(toolPromises);

    // Clear the host listener now that tools are done
    this.runtime!.hostUpdateListener = undefined;

    // Feed results into Rust in order
    let lastActions: AgentAction[] = [];
    for (const { call, payload } of results) {
      const step = onToolDone(handle, call.id, payload);
      for (const event of step.events) {
        this.log("event", event.type, event);
      }
      lastActions = step.actions;
    }

    // Process any pending actions
    return this.processActions(handle, lastActions);
  }

  /**
   * Handle cancel_tools action from Rust.
   * Cancels running tools via ToolRuntime (if available) and notifies Rust.
   */
  private async handleCancelTools(
    handle: number,
    toolCallIds: string[],
    reason: CancelReason,
  ): Promise<AgentAction> {
    let lastActions: AgentAction[] = [];

    for (const id of toolCallIds) {
      // Cancel via runtime if available
      if (this.runtime) {
        this.runtime.cancel(id);
      }

      // Notify Rust that this tool was cancelled
      const step = onToolCancelled(handle, id, reason);
      for (const event of step.events) {
        this.log("event", event.type, event);
      }
      lastActions = step.actions;
    }

    return this.processActions(handle, lastActions);
  }
}
