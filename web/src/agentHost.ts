/**
 * Agent host loop that drives the WASM Rust agent.
 *
 * The host:
 * 1. Calls prompt() to start a turn.
 * 2. Inspects returned AgentActions.
 * 3. For stream_llm: uses a fake LLM to produce a response, feeds it via onLlmDone.
 * 4. For execute_tools: uses fake tool handlers, feeds results via onToolDone.
 * 5. Repeats until finished or wait_for_input.
 *
 * Every action and event is logged to a trace array for assertions.
 */

import {
  createAgent,
  destroyAgent,
  feedLlmChunk,
  followUp,
  onLlmDone,
  onToolDone,
  prompt,
  reset,
  state,
  steer,
  type AgentAction,
  type AgentEvent,
  type AgentOptions,
} from "./wasmBinding.ts";
import { FakeLlm } from "./fakeLlm.ts";
import { FakeToolRegistry } from "./fakeTools.ts";

export interface TraceEntry {
  phase: "action" | "event" | "host";
  type: string;
  data: unknown;
}

export interface AgentRunResult {
  /** The final action that terminated the loop. */
  terminalAction: AgentAction;
  /** Ordered trace of every action, event, and host decision. */
  trace: TraceEntry[];
  /** The agent handle (for further inspection). */
  handle: number;
}

export class AgentHost {
  readonly trace: TraceEntry[] = [];
  readonly llm: FakeLlm;
  readonly tools: FakeToolRegistry;
  /** Optional hook called when stream_llm is encountered, before the LLM responds.
   *  Receives the agent handle. Use to inject followUp/steer mid-run. */
  onStreamLlm?: (handle: number) => void;

  constructor(llm: FakeLlm, tools: FakeToolRegistry) {
    this.llm = llm;
    this.tools = tools;
  }

  private log(phase: TraceEntry["phase"], type: string, data: unknown): void {
    this.trace.push({ phase, type, data });
  }

  /** Run the agent loop until it terminates. */
  run(options: AgentOptions, userPrompt: string): AgentRunResult {
    const handle = createAgent(options);
    this.log("host", "create_agent", { handle });

    return this.runPrompt(handle, userPrompt);
  }

  /** Run a single prompt on an existing agent handle. */
  runPrompt(handle: number, userPrompt: string): AgentRunResult {
    this.log("host", "prompt", { text: userPrompt });
    const step = prompt(handle, userPrompt);

    for (const event of step.events) {
      this.log("event", event.type, event);
    }

    const terminalAction = this.processActions(handle, step.actions);
    return { terminalAction, trace: this.trace, handle };
  }

  /** Send a steering message and continue the loop. */
  runSteer(handle: number, message: object): AgentRunResult {
    this.log("host", "steer", message);
    const output = steer(handle, message);
    for (const event of output.events) {
      this.log("event", event.type, event);
    }
    // Steering doesn't produce actions; it queues for later.
    return { terminalAction: { type: "wait_for_input", mode: "any" } as AgentAction, trace: this.trace, handle };
  }

  /** Queue a follow-up message. */
  runFollowUp(handle: number, message: object): void {
    this.log("host", "follow_up", message);
    followUp(handle, message);
  }

  /** Reset the agent and run a new prompt. */
  resetAndRun(handle: number, userPrompt: string): AgentRunResult {
    this.log("host", "reset", null);
    reset(handle);
    return this.runPrompt(handle, userPrompt);
  }

  /** Clean up an agent handle. */
  cleanup(handle: number): void {
    destroyAgent(handle);
    this.log("host", "destroy_agent", { handle });
  }

  // --- Private internals ---

  private processActions(handle: number, actions: AgentAction[]): AgentAction {
    for (const action of actions) {
      this.log("action", action.type, action);

      switch (action.type) {
        case "stream_llm":
          return this.handleStreamLlm(handle);
        case "execute_tools":
          return this.handleExecuteTools(handle, action.calls);
        case "finished":
          return action;
        case "wait_for_input":
          return action;
      }
    }

    // No actions: should not happen in normal flow, but handle gracefully.
    return { type: "finished", messages: [] } as AgentAction;
  }

  private handleStreamLlm(handle: number): AgentAction {
    // Allow host to inject follow-up/steer before LLM responds.
    if (this.onStreamLlm) {
      this.onStreamLlm(handle);
    }

    const resp = this.llm.next();

    // Stream chunks: feed each chunk to the agent and log events.
    const chunks = this.llm.buildChunks(resp);
    for (const chunk of chunks) {
      this.log("host", "feed_llm_chunk", chunk);
      const chunkResult = feedLlmChunk(handle, chunk);
      for (const event of chunkResult.events) {
        this.log("event", event.type, event);
      }
    }

    // Final result: complete the stream via onLlmDone.
    const llmResult = this.llm.buildLlmResult(resp);
    this.log("host", "llm_result", llmResult);

    const step = onLlmDone(handle, llmResult);
    for (const event of step.events) {
      this.log("event", event.type, event);
    }

    return this.processActions(handle, step.actions);
  }

  private handleExecuteTools(handle: number, calls: import("./wasmBinding.ts").ToolCall[]): AgentAction {
    // Execute all tools, feed results one by one.
    // The last onToolDone may produce new actions (another stream_llm or finished).
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

// --- Helper to build standard test agent options ---

export function defaultAgentOptions(overrides?: Partial<AgentOptions>): AgentOptions {
  return {
    system_prompt: "You are a test agent.",
    model: {
      id: "test-model",
      name: "Test",
      api: "test",
      provider: "test",
      reasoning: false,
      context_window: 4096,
      max_tokens: 1024,
    },
    thinking_level: "off",
    tools: [],
    ...overrides,
  };
}
