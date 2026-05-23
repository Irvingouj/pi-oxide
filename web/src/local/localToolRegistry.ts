/**
 * Local tool registry for real filesystem/bash operations.
 *
 * Implements the ToolRegistry interface used by AgentHost and RealAgentHost.
 * Dispatches read, write, edit, and bash to local implementations.
 * Uses pi-compatible tool schemas from schemas.ts.
 */

import type { ToolCall } from "../wasmBinding.ts";
import type { ToolRegistry } from "../fakeTools.ts";
import { handleLocalRead, handleLocalWrite, handleLocalEdit } from "./fileTools.ts";
import { handleLocalBash, type BashPolicy } from "./bashTool.ts";

export interface LocalToolRegistryOptions {
  cwd: string;
  bashPolicy?: BashPolicy;
}

const DEFAULT_BASH_POLICY: BashPolicy = {
  mode: "deny",
};

const HANDLER_NAMES = new Set(["read", "write", "edit", "bash"]);

export class LocalToolRegistry implements ToolRegistry {
  readonly cwd: string;
  readonly bashPolicy: BashPolicy;
  public readonly log: string[] = [];

  constructor(options: LocalToolRegistryOptions) {
    this.cwd = options.cwd;
    this.bashPolicy = options.bashPolicy ?? DEFAULT_BASH_POLICY;
  }

  execute(call: ToolCall): object {
    const args = (call.arguments ?? {}) as Record<string, unknown>;

    let result: object;
    switch (call.name) {
      case "read":
        result = handleLocalRead(args, this.cwd);
        break;
      case "write":
        result = handleLocalWrite(args, this.cwd);
        break;
      case "edit":
        result = handleLocalEdit(args, this.cwd);
        break;
      case "bash":
        result = handleLocalBash(args, this.cwd, this.bashPolicy);
        break;
      default:
        result = {
          error: {
            code: "unknown_tool",
            message: `no handler for tool: ${call.name}`,
          },
        };
        break;
    }

    const isError = "error" in result;
    this.log.push(
      `tool_result(${call.name}): ${isError ? "[ERROR] " : ""}${JSON.stringify(args)}`,
    );

    return result;
  }
}
