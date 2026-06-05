// EventMapper — converts raw WASM AgentEvent[] into semantic SDK events.
// Accumulates RunState (messages, toolCalls, artifacts, usage).
// Emits all status states: idle, loading, thinking, calling_model, running_tool, saving, completed, aborted, failed.
// Artifact events from turn_end markers (new API: artifacts tracked via tool result details).
// Usage accumulation from model responses.

import type {
  AgentEvent as RawAgentEvent,
  AgentMessage as WasmAgentMessage,
  Content,
} from "../../pi_host_web.js";
import type {
  AgentMessage,
  AgentContentBlock,
  AgentToolRun,
  AgentArtifactRef,
  AgentStatus,
  AgentRunResult,
  TokenUsage,
} from "../types.ts";

export interface SemanticEvent {
  type: string;
  payload: unknown;
}

export interface RunState {
  messages: AgentMessage[];
  toolCalls: AgentToolRun[];
  artifacts: AgentArtifactRef[];
  usage: TokenUsage;
  text: string;
  currentMessage: AgentMessage | null;
  currentTool: AgentToolRun | null;
}

export class EventMapper {
  createRunState(): RunState {
    return {
      messages: [],
      toolCalls: [],
      artifacts: [],
      usage: { input: 0, output: 0, cache_read: 0, cache_write: 0, total_tokens: 0 },
      text: "",
      currentMessage: null,
      currentTool: null,
    };
  }

  map(rawEvent: RawAgentEvent, state: RunState): SemanticEvent[] {
    const events: SemanticEvent[] = [];

    switch (rawEvent.type) {
      case "agent_start": {
        events.push({ type: "status", payload: { state: "loading", message: "Agent starting..." } as AgentStatus });
        break;
      }

      case "turn_start": {
        events.push({ type: "status", payload: { state: "thinking", message: "Thinking..." } as AgentStatus });
        break;
      }

      case "message_start": {
        const msg = this.convertWasmMessage(rawEvent.message);
        state.currentMessage = msg;
        state.messages.push(msg);
        events.push({ type: "messageStart", payload: msg });
        events.push({ type: "status", payload: { state: "thinking" } as AgentStatus });
        break;
      }

      case "message_update": {
        const delta = rawEvent.delta;
        if (delta.kind === "text_delta" && delta.text) {
          state.text += delta.text;
          events.push({ type: "text", payload: delta.text });
        } else if (delta.kind === "thinking_delta") {
          events.push({ type: "status", payload: { state: "thinking", message: "Thinking..." } as AgentStatus });
        }
        break;
      }

      case "message_end": {
        const msg = this.convertWasmMessage(rawEvent.message);
        state.currentMessage = msg;
        // Update the last message in state
        const idx = state.messages.findIndex((m) => m.id === msg.id);
        if (idx >= 0) {
          state.messages[idx] = msg;
        } else {
          state.messages.push(msg);
        }
        events.push({ type: "messageEnd", payload: msg });
        break;
      }

      case "tool_execution_start": {
        const tool: AgentToolRun = {
          id: rawEvent.tool_call_id,
          name: rawEvent.tool_name,
          title: rawEvent.tool_name,
          input: rawEvent.args ?? {},
          status: "running",
          startedAt: Date.now(),
        };
        state.currentTool = tool;
        state.toolCalls.push(tool);
        events.push({ type: "toolStart", payload: tool });
        events.push({ type: "status", payload: { state: "running_tool", message: `Running ${rawEvent.tool_name}...` } as AgentStatus });
        break;
      }

      case "tool_execution_update": {
        const tool = state.toolCalls.find((t) => t.id === rawEvent.tool_call_id);
        if (tool) {
          tool.output = (tool.output ?? "") + rawEvent.chunk;
          events.push({ type: "toolUpdate", payload: tool });
        }
        break;
      }

      case "tool_execution_end": {
        let tool = state.toolCalls.find((t) => t.id === rawEvent.tool_call_id);
        if (!tool) {
          const toolName = rawEvent.tool_name ?? "unknown";
          tool = {
            id: rawEvent.tool_call_id,
            name: toolName,
            title: toolName,
            input: rawEvent.args ?? {},
            status: rawEvent.is_error ? "failed" : "completed",
            startedAt: Date.now(),
            endedAt: Date.now(),
          };
          state.toolCalls.push(tool);
        }
        tool.status = rawEvent.is_error ? "failed" : "completed";
        tool.endedAt = Date.now();
        // Extract output from result
        const resultText = rawEvent.result.content
          .filter((c): c is { type: "text"; text: string } => c.type === "text")
          .map((c) => c.text)
          .join("\n");
        tool.output = resultText;
        events.push({ type: "toolEnd", payload: tool });
        break;
      }

      case "tool_execution_cancelled": {
        const tool = state.toolCalls.find((t) => t.id === rawEvent.tool_call_id);
        if (tool) {
          tool.status = "cancelled";
          tool.endedAt = Date.now();
          events.push({ type: "toolEnd", payload: tool });
        }
        break;
      }

      case "turn_end": {
        // Extract final message
        const finalMsg = this.convertWasmMessage(rawEvent.message);
        const idx = state.messages.findIndex((m) => m.id === finalMsg.id);
        if (idx >= 0) {
          state.messages[idx] = finalMsg;
        } else {
          state.messages.push(finalMsg);
        }

        // Extract tool results
        for (const tr of rawEvent.tool_results) {
          let tool = state.toolCalls.find((t) => t.id === tr.tool_call_id);
          if (!tool) {
            const toolName = tr.tool_name ?? "unknown";
            tool = {
              id: tr.tool_call_id,
              name: toolName,
              title: toolName,
              input: {},
              status: tr.is_error ? "failed" : "completed",
              startedAt: Date.now(),
              endedAt: Date.now(),
            };
            state.toolCalls.push(tool);
          }
          tool.status = tr.is_error ? "failed" : "completed";
          tool.endedAt = Date.now();
          const resultText = tr.content
            .filter((c): c is { type: "text"; text: string } => c.type === "text")
            .map((c) => c.text)
            .join("\n");
          tool.output = resultText;
        }

        events.push({ type: "status", payload: { state: "completed" } as AgentStatus });
        break;
      }

      case "save_point": {
        events.push({ type: "status", payload: { state: "saving", message: "Saving session..." } as AgentStatus });
        break;
      }

      case "settled": {
        events.push({ type: "status", payload: { state: "completed" } as AgentStatus });
        break;
      }

      case "queue_update": {
        // Debug channel — not a primary semantic event
        break;
      }

      case "agent_end": {
        events.push({ type: "status", payload: { state: "idle" } as AgentStatus });
        break;
      }
    }

    return events;
  }

  buildRunResult(state: RunState, turnResult: { aborted: boolean }): AgentRunResult {
    if (turnResult.aborted) {
      return {
        status: "aborted",
        text: state.text,
        toolCalls: state.toolCalls,
        artifacts: state.artifacts,
        usage: state.usage,
      };
    }

    return {
      status: "completed",
      message: state.currentMessage ?? undefined,
      text: state.text,
      toolCalls: state.toolCalls,
      artifacts: state.artifacts,
      usage: state.usage,
    };
  }

  processMarkers(markers: Array<{ type: string; entry_ids?: string[] }>, state: RunState): SemanticEvent[] {
    const events: SemanticEvent[] = [];
    for (const marker of markers) {
      if (marker.type === "new_artifacts" && marker.entry_ids) {
        for (const id of marker.entry_ids) {
          state.artifacts.push({ id, kind: "text" });
          events.push({ type: "artifact", payload: { id, kind: "text" } });
        }
      }
    }
    return events;
  }

  private convertWasmMessage(msg: WasmAgentMessage): AgentMessage {
    const id = this.generateStableId(msg);
    return {
      id,
      role: msg.role,
      content: msg.content.map((c) => this.convertContent(c)),
      timestamp: msg.timestamp ?? Date.now(),
      tool_call_id: msg.role === "tool_result" ? (msg as unknown as { tool_call_id: string }).tool_call_id : undefined,
    };
  }

  private generateStableId(msg: WasmAgentMessage): string {
    const contentHash = msg.content.map((c) => {
      if (c.type === "text") return `t:${c.text?.slice(0, 20) ?? ""}`;
      if (c.type === "tool_call") return `tc:${c.id ?? ""}:${c.name ?? ""}`;
      if (c.type === "image") return `img:${c.media_type ?? ""}`;
      return (c as { type: string }).type;
    }).join("|");
    return `msg-${msg.role}-${msg.timestamp ?? 0}-${contentHash}`;
  }

  private convertContent(c: Content): AgentContentBlock {
    if (c.type === "text") return { type: "text", text: c.text };
    if (c.type === "tool_call") return { type: "tool_call", id: c.id, name: c.name, arguments: c.arguments };
    if (c.type === "image") return { type: "image", mimeType: c.media_type, data: c.data };
    return { type: "text", text: "" };
  }
}
