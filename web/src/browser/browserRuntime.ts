/**
 * Browser runtime adapter interface and types.
 *
 * Defines the contract between browser tools and the actual browser environment.
 * Tests use FakeBrowserRuntime; real browser uses a LiveBrowserRuntime backed
 * by window/document later.
 *
 * Host-owned — no browser APIs in pi-core.
 */

// --- Snapshot types ---

export interface BrowserPageSnapshot {
  url: string;
  title: string;
  readyState: "loading" | "interactive" | "complete";
  focusedElement: BrowserElementSnapshot | null;
}

export interface BrowserElementSnapshot {
  tag: string;
  text: string;
  attributes: Record<string, string>;
  visible: boolean;
  selector: string;
}

export interface BrowserConsoleEntry {
  level: "log" | "warn" | "error" | "info";
  args: string[];
  timestamp: number;
}

export interface BrowserToolResult {
  ok: boolean;
  data?: unknown;
  error?: { code: string; message: string };
}

// --- Runtime adapter ---

export interface BrowserRuntime {
  getPage(): BrowserPageSnapshot;
  evalJs(source: string): unknown;
  querySelector(selector: string): BrowserElementSnapshot | null;
  querySelectorAll(selector: string): BrowserElementSnapshot[];
  click(selector: string): BrowserToolResult;
  type(selector: string, text: string): BrowserToolResult;
  getConsole(): BrowserConsoleEntry[];
}
