/**
 * Anthropic Messages API adapter.
 *
 * Converts between Rust agent core message format and the Anthropic Messages API.
 * Works with any Anthropic-compatible endpoint (including Fireworks.ai).
 *
 * This adapter does NOT stream. It makes a single request and returns the full
 * response as chunks + final result, matching the existing AgentHost pattern.
 */

import type { ToolDefinition } from "../../../pi_host_web.js";
import { isRecord } from "../../internal/util/types.ts";
import type {
  AgentMessageShape,
  ContentBlock,
  LlmRequest,
  ProviderResult,
  TokenUsage,
} from "./types.ts";

// --- Anthropic API types ---

interface AnthropicMessage {
  role: "user" | "assistant";
  content: string | AnthropicContentBlock[];
}

type AnthropicContentBlock =
  | { type: "text"; text: string }
  | {
    type: "tool_use";
    id: string;
    name: string;
    input: Record<string, unknown>;
  }
  | {
    type: "tool_result";
    tool_use_id: string;
    content: string | AnthropicContentBlock[];
    is_error?: boolean;
  };

interface AnthropicTool {
  name: string;
  description: string;
  input_schema: object;
}

interface AnthropicResponse {
  id: string;
  type: "message";
  role: "assistant";
  content: AnthropicContentBlock[];
  model: string;
  stop_reason: "end_turn" | "max_tokens" | "stop_sequence" | "tool_use" | null;
  usage: {
    input_tokens: number;
    output_tokens: number;
    cache_creation_input_tokens?: number;
    cache_read_input_tokens?: number;
  };
}

interface AnthropicError {
  type: "error";
  error: { type: string; message: string };
}

// --- Conversion: Rust messages -> Anthropic messages ---

/**
 * Convert Rust agent messages to Anthropic Messages API format.
 *
 * Anthropic requires that multiple tool_result responses to a single assistant
 * message with multiple tool_use blocks be grouped into ONE user message
 * containing an array of tool_result blocks. Sending separate consecutive
 * user messages each with a single tool_result block triggers an API error:
 *   "messages: Unexpected role change from user to user"
 *
 * See: https://docs.anthropic.com/en/docs/build-with-claude/tool-use#handling-tool-use-and-tool-result-content-blocks
 */
export function convertMessages(
  messages: AgentMessageShape[],
): AnthropicMessage[] {
  const result: AnthropicMessage[] = [];

  let i = 0;
  while (i < messages.length) {
    const msg = messages[i];

    switch (msg.role) {
      case "user": {
        const text = extractText(msg.content);
        result.push({ role: "user", content: text });
        i++;
        break;
      }
      case "assistant": {
        const blocks: AnthropicContentBlock[] = [];
        for (const block of msg.content) {
          if (block.type === "text" && block.text !== undefined) {
            blocks.push({ type: "text", text: block.text });
          } else if (block.type === "tool_call" && block.id && block.name) {
            blocks.push({
              type: "tool_use",
              id: block.id,
              name: block.name,
              input: block.arguments ?? {},
            });
          }
        }
        result.push({ role: "assistant", content: blocks });
        i++;
        break;
      }
      case "tool_result": {
        // Gather consecutive tool_result messages into a single user message.
        // Anthropic requires all tool_results for a given assistant turn to be
        // in one user message with an array of tool_result content blocks.
        const toolResults: AnthropicContentBlock[] = [];
        while (i < messages.length && messages[i].role === "tool_result") {
          const tr = messages[i] as Extract<
            AgentMessageShape,
            { role: "tool_result" }
          >;
          const text = extractText(tr.content);
          toolResults.push({
            type: "tool_result",
            tool_use_id: tr.tool_call_id,
            content: text,
            is_error: tr.is_error,
          });
          i++;
        }
        result.push({ role: "user", content: toolResults });
        break;
      }
    }
  }

  return result;
}

// --- Conversion: Rust tool definitions -> Anthropic tools ---

export function convertTools(tools: ToolDefinition[]): AnthropicTool[] {
  return tools.map((t) => ({
    name: t.name,
    description: `${t.label}: ${t.description}`,
    input_schema: t.parameters,
  }));
}

// --- Conversion: Anthropic response -> Rust LlmResult ---

export function convertResponse(
  resp: AnthropicResponse,
  providerName: string,
  modelId: string,
): { llmResult: object; chunks: object[] } {
  const content: ContentBlock[] = [];

  for (const block of resp.content) {
    if (block.type === "text") {
      content.push({ type: "text", text: block.text });
    } else if (block.type === "tool_use") {
      content.push({
        type: "tool_call",
        id: block.id,
        name: block.name,
        arguments: block.input,
      });
    }
  }

  const stopReason = resp.stop_reason === "tool_use" ? "tool_use" : "end_turn";

  const usage: TokenUsage = {
    input: resp.usage.input_tokens,
    output: resp.usage.output_tokens,
    cache_read: resp.usage.cache_read_input_tokens ?? 0,
    cache_write: resp.usage.cache_creation_input_tokens ?? 0,
    total_tokens: resp.usage.input_tokens + resp.usage.output_tokens,
  };

  const assistantMsg = {
    content,
    api: providerName,
    provider: providerName,
    model: modelId,
    stop_reason: stopReason,
    error_message: null,
    timestamp: Date.now(),
    usage,
  };

  // Build streaming chunks: a Start chunk + TextDelta chunks for each text block
  const chunks: object[] = [];
  chunks.push({
    kind: "start",
    content: [{ type: "text", text: "" }],
    api: providerName,
    provider: providerName,
    model: modelId,
    stop_reason: stopReason,
    error_message: null,
    timestamp: 0,
    usage: {
      input: 0,
      output: 0,
      cache_read: 0,
      cache_write: 0,
      total_tokens: 0,
    },
  });

  for (const block of resp.content) {
    if (block.type === "text" && block.text.length > 0) {
      chunks.push({ kind: "text_delta", text: block.text });
    }
  }

  return {
    llmResult: { Ok: assistantMsg },
    chunks,
  };
}

// --- Main call ---

export interface AnthropicConfig {
  apiKey: string;
  baseUrl: string;
  model: string;
  maxTokens?: number;
}

/**
 * Call the Anthropic Messages API and return a ProviderResult.
 */
export async function callAnthropic(
  request: LlmRequest,
  config: AnthropicConfig,
): Promise<ProviderResult> {
  const log: string[] = [];
  const providerName = "anthropic";

  const body = {
    model: config.model,
    max_tokens: config.maxTokens ?? 4096,
    system: request.system_prompt,
    messages: convertMessages(request.messages),
    tools: convertTools(request.tools),
  };

  log.push(
    `anthropic_request: model=${config.model}, messages=${body.messages.length}, tools=${body.tools.length}`,
  );

  let resp: Response;
  try {
    resp = await fetch(`${config.baseUrl}/v1/messages`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "x-api-key": config.apiKey,
        "anthropic-version": "2023-06-01",
      },
      body: JSON.stringify(body),
    });
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    log.push(`anthropic_network_error: ${msg}`);
    return {
      llmResult: {
        Err: { error: { code: "network_error", message: msg }, aborted: false },
      },
      chunks: [],
      log,
    };
  }

  if (!resp.ok) {
    let errorBody: AnthropicError | null = null;
    try {
      errorBody = (await resp.json()) as AnthropicError;
    } catch {
      // ignore parse failure
    }
    const code = `http_${resp.status}`;
    const message =
      errorBody?.error?.message ?? `HTTP ${resp.status}: ${resp.statusText}`;
    log.push(`anthropic_error: ${code} - ${message}`);
    return {
      llmResult: {
        Err: { error: { code, message }, aborted: false },
      },
      chunks: [],
      log,
    };
  }

  const data = (await resp.json()) as AnthropicResponse;
  log.push(
    `anthropic_response: stop_reason=${data.stop_reason}, content_blocks=${data.content.length}`,
  );

  const { llmResult, chunks } = convertResponse(
    data,
    providerName,
    config.model,
  );
  return { llmResult, chunks, log };
}

// --- SDK factory ---

import type { AgentModel, ModelRequest, ModelResponse } from "../../types.ts";
import { createAgentError } from "../../errors.ts";
import { getLogger } from "../../internal/logger.ts";

export function anthropic(config: {
  apiKey: string;
  model: string;
  baseUrl?: string;
  maxTokens?: number;
}): AgentModel {
  const logger = getLogger("anthropic");
  const anthropicConfig: AnthropicConfig = {
    apiKey: config.apiKey,
    baseUrl: config.baseUrl ?? "https://api.anthropic.com",
    model: config.model,
    maxTokens: config.maxTokens,
  };

  return {
    id: config.model,
    contextWindow: 200000,
    maxTokens: config.maxTokens ?? 4096,
    capabilities: {
      vision: config.model.includes("vision") || config.model.startsWith("claude-3-5"),
      jsonMode: true,
      functionCalling: true,
      streaming: true,
    },
    async generate(request: ModelRequest): Promise<ModelResponse> {
      logger.info("Anthropic generate", { model: config.model, messageCount: request.messages.length });
      const llmRequest = {
        system_prompt: request.instructions,
        messages: request.messages.map((msg): AgentMessageShape => {
          const content = msg.content.map((c): ContentBlock => {
            if (c.type === "text") return { type: "text", text: c.text };
            if (c.type === "tool_call") return { type: "tool_call", id: c.id, name: c.name, arguments: isRecord(c.arguments) ? c.arguments : {} };
            if (c.type === "image") return { type: "image", media_type: c.mimeType, data: c.data };
            return { type: "text", text: "" };
          });
          const timestamp = msg.timestamp ?? Date.now();
          if (msg.role === "user") {
            return { role: "user", content, timestamp };
          }
          if (msg.role === "assistant") {
            return {
              role: "assistant",
              content,
              api: "sdk",
              provider: "sdk",
              model: request.tools[0]?.name ?? "sdk-model",
              stop_reason: "end_turn",
              error_message: null,
              timestamp,
              usage: { input: 0, output: 0, cache_read: 0, cache_write: 0, total_tokens: 0 },
            };
          }
          // tool_result
          return {
            role: "tool_result",
            tool_call_id: msg.tool_call_id ?? "",
            tool_name: msg.content.find((c) => c.type === "text")?.text?.slice(0, 50) ?? "unknown",
            content,
            details: {},
            is_error: false,
            timestamp,
          };
        }),
        tools: request.tools.map((t) => ({
          name: t.name,
          label: t.name,
          description: t.description,
          parameters: isRecord(t.inputSchema) ? t.inputSchema : { type: "object", properties: {} },
          execution_mode: "parallel" as const,
        })),
      };

      try {
        const result = await callAnthropic(llmRequest, anthropicConfig);

        if ("Err" in result.llmResult) {
          const err = (result.llmResult as { Err: { error: { code: string; message: string } } }).Err.error;
          logger.warn("Anthropic API error", { code: err.code, message: err.message });
          throw createAgentError(
            err.code === "network_error" ? "model_unavailable" :
              err.code.startsWith("http_401") ? "model_auth_failed" :
                err.code.startsWith("http_429") ? "model_rate_limited" :
                  "model_unavailable",
            err.message,
            { recoverable: err.code === "http_429" },
          );
        }

        const ok = (result.llmResult as { Ok: object }).Ok as {
          content: Array<{ type: string; text?: string; id?: string; name?: string; arguments?: unknown }>;
          stop_reason: string;
          usage?: { input: number; output: number; cache_read: number; cache_write: number; total_tokens: number };
        };

        logger.info("Anthropic response", { stopReason: ok.stop_reason, contentBlocks: ok.content.length });

        return {
          content: ok.content.map((b) => {
            if (b.type === "text") return { type: "text", text: b.text ?? "" };
            if (b.type === "tool_call") return { type: "tool_call", id: b.id ?? "", name: b.name ?? "", arguments: b.arguments ?? {} };
            return { type: "text", text: "" };
          }),
          stopReason: ok.stop_reason === "tool_use" ? "tool_call" : "end",
          usage: ok.usage,
          model: config.model,
          raw: result,
        };
      } catch (e) {
        if (e && typeof e === "object" && "code" in e) throw e;
        logger.error("Anthropic request failed", { error: e instanceof Error ? e.message : String(e) });
        throw createAgentError("model_unavailable", e instanceof Error ? e.message : String(e), { cause: e, recoverable: false });
      }
    },
  };
}

// --- Helpers ---

function extractText(content: ContentBlock[]): string {
  return content
    .filter(
      (b): b is typeof b & { text: string } =>
        b.type === "text" && b.text !== undefined,
    )
    .map((b) => b.text)
    .join("\n");
}
