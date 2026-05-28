/**
 * Tool service — creates a tool registry for the agent run config.
 *
 * Pure JS, no React. Wraps browser tool execution into the SDK ToolMap shape.
 */

import type { ToolMap } from "@pi-oxide/pi-host-web";
import type { BrowserRuntime } from "../browser/browserRuntime.ts";
import { BROWSER_TOOLS, executeBrowserTool } from "../browser/browserTools.ts";

export function createToolRegistry(runtime: BrowserRuntime): ToolMap {
	return Object.fromEntries(
		BROWSER_TOOLS.map((t) => [
			t.name,
			(call: ToolCall) => executeBrowserTool(call, runtime),
		]),
	);
}

export { BROWSER_TOOLS };
