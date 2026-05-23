/**
 * Fake tool executor for testing the host loop.
 *
 * Tools are registered by name and return deterministic results.
 */

import type { ToolCall } from "./wasmBinding.ts";

export interface FakeToolResult {
  /** Text content to return. */
  text: string;
  /** If true, the tool result signals an error. */
  isError?: boolean;
  /** If true, signals termination. */
  terminate?: boolean;
}

export type FakeToolHandler = (call: ToolCall) => FakeToolResult;

export class FakeToolRegistry {
  private handlers = new Map<string, FakeToolHandler>();
  public readonly log: string[] = [];

  /** Register a fake tool handler. */
  register(name: string, handler: FakeToolHandler): void {
    this.handlers.set(name, handler);
  }

  /** Execute a tool call and return the JSON payload for onToolDone. */
  execute(call: ToolCall): object {
    const handler = this.handlers.get(call.name);
    if (!handler) {
      this.log.push(`tool_error(${call.name}): no handler registered`);
      return {
        error: {
          code: "unknown_tool",
          message: `no handler for tool: ${call.name}`,
        },
      };
    }

    const result = handler(call);
    this.log.push(
      `tool_result(${call.name}): ${result.text}${result.isError ? " [ERROR]" : ""}${result.terminate ? " [TERMINATE]" : ""}`
    );

    if (result.isError) {
      return {
        error: {
          code: "tool_error",
          message: result.text,
        },
      };
    }

    const payload: Record<string, unknown> = {
      content: [{ type: "text", text: result.text }],
    };
    if (result.terminate) {
      payload.terminate = true;
    }
    return payload;
  }
}

/** Helper: register a simple text-returning tool. */
export function textTool(name: string, text: string): [string, FakeToolHandler] {
  return [name, () => ({ text })];
}

/** Helper: register a tool that returns an error. */
export function errorTool(name: string, message: string): [string, FakeToolHandler] {
  return [name, () => ({ text: message, isError: true })];
}
