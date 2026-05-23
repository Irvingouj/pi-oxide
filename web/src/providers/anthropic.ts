/**
 * Anthropic Messages API adapter.
 *
 * Converts between Rust agent core message format and the Anthropic Messages API.
 * Works with any Anthropic-compatible endpoint (including Fireworks.ai).
 *
 * This adapter does NOT stream. It makes a single request and returns the full
 * response as chunks + final result, matching the existing AgentHost pattern.
 */

import type { LlmRequest, ProviderResult, AgentMessageShape, ContentBlock, TokenUsage } from "./types.ts";
import type { ToolDefinition } from "../tools/schemas.ts";

// --- Anthropic API types ---

interface AnthropicMessage {
  role: "user" | "assistant";
  content: string | AnthropicContentBlock[];
}

type AnthropicContentBlock =
  | { type: "text"; text: string }
  | { type: "tool_use"; id: string; name: string; input: Record<string, unknown> }
  | { type: "tool_result"; tool_use_id: string; content: string | AnthropicContentBlock[]; is_error?: boolean };

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
  usage: { input_tokens: number; output_tokens: number; cache_creation_input_tokens?: number; cache_read_input_tokens?: number };
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
export function convertMessages(messages: AgentMessageShape[]): AnthropicMessage[] {
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
          const tr = messages[i] as Extract<AgentMessageShape, { role: "tool_result" }>;
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
    usage: { input: 0, output: 0, cache_read: 0, cache_write: 0, total_tokens: 0 },
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

  log.push(`anthropic_request: model=${config.model}, messages=${body.messages.length}, tools=${body.tools.length}`);

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
    const message = errorBody?.error?.message ?? `HTTP ${resp.status}: ${resp.statusText}`;
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
  log.push(`anthropic_response: stop_reason=${data.stop_reason}, content_blocks=${data.content.length}`);

  const { llmResult, chunks } = convertResponse(data, providerName, config.model);
  return { llmResult, chunks, log };
}

// --- Helpers ---

function extractText(content: ContentBlock[]): string {
  return content
    .filter((b) => b.type === "text" && b.text !== undefined)
    .map((b) => b.text!)
    .join("\n");
}
