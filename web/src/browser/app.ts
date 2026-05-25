/**
 * Browser app — wires WASM, agent loop, LLM, tools, persistence, and UI.
 *
 * Uses the high-level @pi-oxide/pi-host-web SDK (Agent class) with
 * streaming LLM responses and event-driven DOM rendering.
 */

import {
  Agent,
  projectContext,
  toolResult,
  type AgentEvent,
  type AgentMessage,
  type LlmChunk,
  type SessionState,
} from "@pi-oxide/pi-host-web";
import { callLlm } from "./browserLlm.ts";
import { initEnvDefaults } from "./config.ts";
import { LiveBrowserRuntime } from "./liveRuntime.ts";
import { executeBrowserTool } from "./browserTools.ts";
import { IndexedDBSessionBackend } from "./persistence.ts";
import type { BrowserRuntime } from "./browserRuntime.ts";
import { BROWSER_TOOLS } from "./browserTools.ts";

// --- Context projection ---

let projectionState: { replacements: Record<string, unknown> } = { replacements: {} };
const projectionBudget = {
  max_tool_result_chars: 50000,
  max_context_tokens: 100000,
  default_preview_chars: 2000,
};

function runProjection(systemPrompt: string, messages: unknown[]): unknown[] {
  try {
    const result = projectContext({
      system_prompt: systemPrompt,
      messages,
      budget: projectionBudget,
      state: projectionState,
    });
    projectionState = result.updated_state;
    return result.projected_messages;
  } catch (e) {
    console.warn("projection error:", e);
    return messages;
  }
}

// --- DOM helpers ---

function addMsg(chatContainer: HTMLElement, type: string, html: string): HTMLDivElement {
  const div = document.createElement("div");
  div.className = `msg msg-${type}`;
  div.innerHTML = html;
  chatContainer.appendChild(div);
  chatContainer.scrollTop = chatContainer.scrollHeight;
  return div;
}

function addText(chatContainer: HTMLElement, type: string, text: string): HTMLDivElement {
  const div = document.createElement("div");
  div.className = `msg msg-${type}`;
  div.textContent = text;
  chatContainer.appendChild(div);
  chatContainer.scrollTop = chatContainer.scrollHeight;
  return div;
}

// --- Streaming LLM provider ---

interface BrowserLlmBlock {
  type: string;
  text?: string;
  id?: string;
  name?: string;
  input?: unknown;
}

interface BrowserLlmResponse {
  content: BrowserLlmBlock[];
  stop_reason: string;
}

async function* buildStreamingChunks(
  data: BrowserLlmResponse,
): AsyncGenerator<LlmChunk> {
  // Start chunk
  yield {
    kind: "start",
    content: [{ type: "text", text: "" }],
    api: "anthropic",
    provider: "anthropic",
    model: "browser-model",
    stop_reason: "end_turn",
    error_message: null,
    timestamp: Date.now(),
    usage: { input: 0, output: 0, cache_read: 0, cache_write: 0, total_tokens: 0 },
  };

  // Stream text word-by-word for UX
  for (const block of data.content) {
    if (block.type === "text" && block.text) {
      const words = block.text.split(/(\s+)/);
      for (const word of words) {
        if (word) {
          yield { kind: "text_delta", text: word };
          await new Promise((r) => setTimeout(r, 15));
        }
      }
    }
  }
}

function buildLlmResult(data: BrowserLlmResponse): object {
  const content = data.content
    .filter((b) => b.type === "text" || b.type === "tool_use")
    .map((b) => {
      if (b.type === "text") return { type: "text", text: b.text };
      return { type: "tool_call", id: b.id, name: b.name, arguments: b.input || {} };
    });

  if (content.length === 0) {
    content.push({ type: "text", text: "" });
  }

  const stopReason = data.stop_reason === "tool_use" ? "tool_use" : "end_turn";

  return {
    Ok: {
      content,
      api: "anthropic",
      provider: "anthropic",
      model: "browser-model",
      stop_reason: stopReason,
      error_message: null,
      timestamp: Date.now(),
      usage: { input: 0, output: 0, cache_read: 0, cache_write: 0, total_tokens: 0 },
    },
  };
}

// --- Event-driven DOM renderer ---

class DomRenderer {
  private chatContainer: HTMLElement;
  private sendBtn: HTMLButtonElement;
  private currentTextDiv: HTMLDivElement | null = null;
  private toolCards = new Map<string, HTMLDivElement>();

  constructor(chatContainer: HTMLElement, sendBtn: HTMLButtonElement) {
    this.chatContainer = chatContainer;
    this.sendBtn = sendBtn;
  }

  onEvent(event: AgentEvent) {
    switch (event.type) {
      case "message_start": {
        this.currentTextDiv = addText(this.chatContainer, "assistant", "");
        break;
      }

      case "message_update": {
        const delta = event.delta as Record<string, unknown>;
        if (delta.kind === "text_delta" && typeof delta.text === "string") {
          if (this.currentTextDiv) {
            this.currentTextDiv.textContent += delta.text;
          } else {
            this.currentTextDiv = addText(this.chatContainer, "assistant", delta.text);
          }
        }
        this.chatContainer.scrollTop = this.chatContainer.scrollHeight;
        break;
      }

      case "message_end": {
        this.currentTextDiv = null;
        break;
      }

      case "tool_execution_start": {
        const toolDiv = addMsg(
          this.chatContainer,
          "tool",
          `<span class="tool-name">${(event as any).tool_name}</span> <span class="tool-id">${(event as any).tool_call_id}</span>`,
        );
        this.toolCards.set((event as any).tool_call_id, toolDiv);
        break;
      }

      case "tool_execution_end": {
        const toolDiv = this.toolCards.get((event as any).tool_call_id);
        const result = (event as any).result as { content?: Array<{ text?: string }> };
        const text = result.content?.map((c) => c.text).join("\n") ?? "";
        if (toolDiv) {
          const resultDiv = document.createElement("div");
          resultDiv.className = "tool-result";
          resultDiv.textContent = text.slice(0, 500);
          toolDiv.appendChild(resultDiv);
        }
        break;
      }

      case "finished":
      case "wait_for_input": {
        this.sendBtn.disabled = false;
        break;
      }
    }
  }
}

// --- Public bootstrap API ---

export interface AppElements {
  chatContainer: HTMLElement;
  userInput: HTMLTextAreaElement;
  sendBtn: HTMLButtonElement;
  statusEl: HTMLElement;
}

const SESSION_ID = "browser-default-session";

export async function bootstrap(els: AppElements): Promise<{
  sendPrompt: (text: string) => Promise<void>;
}> {
  initEnvDefaults();

  const runtime = new LiveBrowserRuntime();
  const sessionBackend = new IndexedDBSessionBackend();
  const renderer = new DomRenderer(els.chatContainer, els.sendBtn);

  const systemPrompt =
    "You are a browser automation agent. You can see the current page, " +
    "query elements, click, type, evaluate JavaScript, and read console logs. " +
    "Help the user accomplish tasks in the browser.";

  // Try to restore previous session state
  let restoredState: SessionState | undefined;
  try {
    const loaded = await sessionBackend.load(SESSION_ID);
    if (loaded) {
      restoredState = loaded;
      els.statusEl.textContent = "Session restored";
    }
  } catch {
    // No previous session — start fresh
  }

  const agent = await Agent.create({
    system_prompt: systemPrompt,
    model: {
      id: "browser-model",
      name: "browser",
      api: "anthropic",
      provider: "anthropic",
      reasoning: false,
      context_window: 100000,
      max_tokens: 1024,
      capabilities: { vision: false, json_mode: true, function_calling: true, streaming: true },
      cost: { input: 0, output: 0, cache_read: 0, cache_write: 0 },
    },
    tools: BROWSER_TOOLS,
    session_id: SESSION_ID,
    session_state: restoredState,
  });

  els.statusEl.textContent = restoredState ? "Session restored" : "Ready";
  els.statusEl.style.color = "#4caf50";
  els.sendBtn.disabled = false;

  let running = false;

  async function sendPrompt(text: string): Promise<void> {
    if (running || !text.trim()) return;
    running = true;
    els.sendBtn.disabled = true;

    addText(els.chatContainer, "user", text);

    try {
      await agent.run(text, {
        llm: {
          async call(context) {
            const projected = runProjection(
              context.system_prompt,
              context.messages as unknown[],
            );
            const data = await callLlm(
              context.system_prompt,
              projected as Parameters<typeof callLlm>[1],
              context.tools as Parameters<typeof callLlm>[2],
            );
            return {
              chunks: buildStreamingChunks(data),
              result: Promise.resolve(buildLlmResult(data) as any),
            };
          },
        },
        tools: {
          async [BROWSER_TOOLS[0].name](call: any) {
            const result = executeBrowserTool(call, runtime);
            if ("error" in result && result.error) {
              return { error: result.error };
            }
            return toolResult(JSON.stringify(result, null, 2).slice(0, 500));
          },
        },
        onEvent: (event) => renderer.onEvent(event),
      });
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      addText(els.chatContainer, "error", `Error: ${msg}`);
      els.sendBtn.disabled = false;
    }

    // Persist session state after the turn completes
    try {
      const state = agent.getSessionState();
      await sessionBackend.save(SESSION_ID, state);
    } catch (e) {
      console.warn("session save failed:", e);
    }

    running = false;
  }

  return { sendPrompt };
}
