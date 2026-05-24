/**
 * Browser app — wires WASM, agent loop, LLM, tools, persistence, and UI.
 *
 * This is the browser-specific equivalent of RealAgentHost, but optimized
 * for the inline browser experience with DOM rendering and IndexedDB persistence.
 */

import { ensureInit } from "./wasm.ts";
import {
  createAgent,
  prompt,
  feedLlmChunk,
  onLlmDone,
  onToolDone,
  onToolStarted,
  projectContext,
  type StepOutput,
  type AgentAction,
} from "./wasmBinding.ts";
import { initEnvDefaults } from "./config.ts";
import { callLlm } from "./browserLlm.ts";
import { LiveBrowserRuntime } from "./liveRuntime.ts";
import { executeBrowserTool } from "./browserTools.ts";
import { persistMessage, persistArtifact } from "./persistence.ts";
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

// --- Agent loop ---

async function agentLoop(
  handle: number,
  actions: AgentAction[],
  chatContainer: HTMLElement,
  sendBtn: HTMLButtonElement,
  runtime: BrowserRuntime,
): Promise<void> {
  for (const action of actions) {
    switch (action.type) {
      case "stream_llm": {
        const projected = runProjection(
          action.context.system_prompt,
          action.context.messages,
        );
        const loadingDiv = addMsg(chatContainer, "loading", "Thinking...");

        try {
          const data = await callLlm(
            action.context.system_prompt,
            projected as Parameters<typeof callLlm>[1],
            action.context.tools as Parameters<typeof callLlm>[2],
          );
          loadingDiv.remove();

          const content = data.content
            .filter((b) => b.type === "text" || b.type === "tool_use")
            .map((b) => {
              if (b.type === "text") return { type: "text", text: b.text };
              return { type: "tool_call", id: b.id, name: b.name, arguments: b.input || {} };
            });

          if (content.length === 0) content.push({ type: "text", text: "" });

          const stopReason = data.stop_reason === "tool_use" ? "tool_use" : "end_turn";

          const textParts = data.content
            .filter((b) => b.type === "text")
            .map((b) => b.text)
            .join("\n");
          if (textParts) {
            addText(chatContainer, "assistant", textParts);
            persistMessage("assistant", textParts);
          }

          // Feed start chunk then final result
          feedLlmChunk(handle, {
            kind: "start",
            content: [{ type: "text", text: "" }],
            api: "anthropic",
            provider: "anthropic",
            model: "test",
            stop_reason: "end_turn",
            error_message: null,
            timestamp: Date.now(),
            usage: { input: 0, output: 0, cache_read: 0, cache_write: 0, total_tokens: 0 },
          });

          const step = onLlmDone(handle, {
            Ok: {
              content,
              api: "anthropic",
              provider: "anthropic",
              model: "test",
              stop_reason: stopReason,
              error_message: null,
              timestamp: Date.now(),
              usage: { input: 0, output: 0, cache_read: 0, cache_write: 0, total_tokens: 0 },
            },
          });
          await agentLoop(handle, step.actions, chatContainer, sendBtn, runtime);
          return;
        } catch (e: unknown) {
          loadingDiv.remove();
          const msg = e instanceof Error ? e.message : String(e);
          addText(chatContainer, "error", `LLM Error: ${msg}`);

          try {
            const step = onLlmDone(handle, {
              Err: { error: { code: "call_failed", message: msg }, aborted: false },
            });
            await agentLoop(handle, step.actions, chatContainer, sendBtn, runtime);
          } catch {
            sendBtn.disabled = false;
          }
        }
        break;
      }

      case "execute_tools": {
        let lastStep: StepOutput | null = null;
        for (const call of action.calls) {
          onToolStarted(handle, call.id);

          const toolDiv = addMsg(
            chatContainer,
            "tool",
            `<span class="tool-name">${call.name}</span>(${Object.entries(call.arguments)
              .map(([k, v]) => `${k}=${JSON.stringify(v)}`)
              .join(", ")})`,
          );

          const toolResult = executeBrowserTool(call, runtime);

          if ("error" in toolResult && toolResult.error) {
            lastStep = onToolDone(handle, call.id, {
              error: { code: toolResult.error.code, message: toolResult.error.message },
            });
          } else {
            const resultStr = JSON.stringify(toolResult, null, 2);
            const resultDiv = document.createElement("div");
            resultDiv.className = "tool-result";
            resultDiv.textContent = resultStr.slice(0, 500);
            toolDiv.appendChild(resultDiv);
            persistArtifact(`tool-${call.id}`, call.name, resultStr);

            lastStep = onToolDone(handle, call.id, {
              content: [{ type: "text", text: resultStr }],
            });
          }
        }
        if (lastStep) {
          await agentLoop(handle, lastStep.actions, chatContainer, sendBtn, runtime);
        }
        return;
      }

      case "finished":
        addText(chatContainer, "assistant", "Done");
        sendBtn.disabled = false;
        return;

      case "wait_for_input":
        sendBtn.disabled = false;
        return;
    }
  }
  sendBtn.disabled = false;
}

// --- Public bootstrap API ---

export interface AppElements {
  chatContainer: HTMLElement;
  userInput: HTMLTextAreaElement;
  sendBtn: HTMLButtonElement;
  statusEl: HTMLElement;
}

export async function bootstrap(els: AppElements): Promise<{
  sendPrompt: (text: string) => Promise<void>;
}> {
  await ensureInit();
  initEnvDefaults();

  const runtime = new LiveBrowserRuntime();

  const systemPrompt =
    "You are a browser automation agent. You can see the current page, " +
    "query elements, click, type, evaluate JavaScript, and read console logs. " +
    "Help the user accomplish tasks in the browser.";

  const handle = createAgent({
    system_prompt: systemPrompt,
    model: {
      id: "browser-model",
      name: "browser",
      api: "anthropic",
      provider: "anthropic",
      reasoning: false,
      context_window: 100000,
      max_tokens: 1024,
    },
    tools: BROWSER_TOOLS,
  });

  els.statusEl.textContent = "Ready";
  els.statusEl.style.color = "#4caf50";
  els.sendBtn.disabled = false;

  let running = false;

  async function sendPrompt(text: string): Promise<void> {
    if (running || !text.trim()) return;
    running = true;
    els.sendBtn.disabled = true;

    addText(els.chatContainer, "user", text);
    persistMessage("user", text);

    const step = prompt(handle, text);
    await agentLoop(handle, step.actions, els.chatContainer, els.sendBtn, runtime);
    running = false;
  }

  return { sendPrompt };
}
