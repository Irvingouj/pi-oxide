/**
 * Tool service — creates a tool registry for the agent run config.
 *
 * Pure JS, no React. Wraps browser tool execution into the SDK ToolMap shape.
 */

import type { ToolMap } from "@pi-oxide/pi-host-web";
import type { BrowserRuntime } from "../browser/browserRuntime.ts";
import { BROWSER_TOOLS, executeBrowserTool } from "../browser/browserTools.ts";

const SMART_EXTRACT_THRESHOLD = 5000;

export function createToolRegistry(
	runtime: BrowserRuntime,
	smartExtract?: (text: string, prompt: string) => Promise<string>,
): ToolMap {
	return Object.fromEntries(
		BROWSER_TOOLS.map((t) => [
			t.name,
			async (call: ToolCall) => {
				const result = executeBrowserTool(call, runtime);
				if (
					smartExtract &&
					"content" in result &&
					result.content.length > 0 &&
					result.details &&
					typeof result.details === "object"
				) {
					const details = result.details as Record<string, unknown>;
					const prompt = details.smart_extract_prompt as string | undefined;
					const text = result.content[0].text;
					const originalChars =
						(typeof details.original_chars === "number" && details.original_chars > 0)
							? details.original_chars
							: text.length;
					if (prompt && originalChars > SMART_EXTRACT_THRESHOLD) {
						const summary = await smartExtract(text, prompt);
						return {
							content: [{ type: "text", text: summary }],
							details: {
								content_kind: "generic_text",
								strategy: { type: "keep_full" },
								original_chars: originalChars,
								truncated_by_tool: false,
							},
						};
					}
				}
				return result;
			},
		]),
	);
}

export { BROWSER_TOOLS };
