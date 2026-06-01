/**
 * Live browser runtime — implements BrowserRuntime with real DOM APIs.
 *
 * Wraps window/document for the browser tool execution layer.
 * Includes console capture so browser_console tool can read intercepted logs.
 */

import type {
	BrowserConsoleEntry,
	BrowserElementSnapshot,
	BrowserPageSnapshot,
	BrowserRuntime,
	BrowserToolResult,
} from "./browserRuntime.ts";
import { getLogger } from "../../internal/logger.ts";

// --- Console capture ---

const consoleEntries: BrowserConsoleEntry[] = [];
const origConsole = {
	log: console.log.bind(console),
	warn: console.warn.bind(console),
	error: console.error.bind(console),
	info: console.info.bind(console),
};

function captureConsole(
	level: BrowserConsoleEntry["level"],
	args: unknown[],
): void {
	consoleEntries.push({ level, args: args.map(String), timestamp: Date.now() });
}

// Intercept console methods
console.log = (...a: unknown[]) => {
	captureConsole("log", a);
	origConsole.log(...a);
};
console.warn = (...a: unknown[]) => {
	captureConsole("warn", a);
	origConsole.warn(...a);
};
console.error = (...a: unknown[]) => {
	captureConsole("error", a);
	origConsole.error(...a);
};
console.info = (...a: unknown[]) => {
	captureConsole("info", a);
	origConsole.info(...a);
};

// --- Element snapshot helper ---

function snapshotElement(
	el: Element,
	selector: string,
): BrowserElementSnapshot {
	const text = (el.textContent || "").trim().slice(0, 500);
	const attributes: Record<string, string> = {};
	for (const a of Array.from(el.attributes)) {
		attributes[a.name] = a.value;
	}
	const style = window.getComputedStyle(el);
	const visible =
		style.display !== "none" &&
		style.visibility !== "hidden" &&
		style.opacity !== "0";
	return {
		tag: el.tagName.toLowerCase(),
		text,
		attributes,
		visible,
		selector,
	};
}

// --- LiveBrowserRuntime ---

export class LiveBrowserRuntime implements BrowserRuntime {
	private logger = getLogger("browser-runtime");

	getPage(): BrowserPageSnapshot {
		const ae = document.activeElement;
		const focused = ae && ae !== document.body ? snapshotElement(ae, "") : null;
		this.logger.debug("getPage", { url: location.href, title: document.title });
		return {
			url: location.href,
			title: document.title,
			readyState: document.readyState as BrowserPageSnapshot["readyState"],
			focusedElement: focused,
		};
	}

	evalJs(source: string): unknown {
		this.logger.debug("evalJs", { sourceLength: source.length });
		return new Function(source)();
	}

	querySelector(selector: string): BrowserElementSnapshot | null {
		const el = document.querySelector(selector);
		this.logger.debug("querySelector", { selector, found: !!el });
		return el ? snapshotElement(el, selector) : null;
	}

	querySelectorAll(selector: string): BrowserElementSnapshot[] {
		const elements = Array.from(document.querySelectorAll(selector));
		this.logger.debug("querySelectorAll", { selector, count: elements.length });
		return elements.map((el) => snapshotElement(el, selector));
	}

	click(selector: string): BrowserToolResult {
		this.logger.debug("click", { selector });
		const el = document.querySelector(selector);
		if (!el) {
			this.logger.warn("click failed: element not found", { selector });
			return {
				ok: false,
				error: {
					code: "element_not_found",
					message: `No element matches: ${selector}`,
				},
			};
		}
		(el as HTMLElement).click();
		return { ok: true };
	}

	type(selector: string, text: string): BrowserToolResult {
		this.logger.debug("type", { selector, textLength: text.length });
		const el = document.querySelector(selector);
		if (!el) {
			this.logger.warn("type failed: element not found", { selector });
			return {
				ok: false,
				error: {
					code: "element_not_found",
					message: `No element matches: ${selector}`,
				},
			};
		}
		if (
			!(el instanceof HTMLInputElement) &&
			!(el instanceof HTMLTextAreaElement)
		) {
			this.logger.warn("type failed: not an input", { selector, tag: el.tagName });
			return {
				ok: false,
				error: {
					code: "not_input",
					message: `Element is not an input or textarea: ${selector}`,
				},
			};
		}
		el.value = text;
		el.dispatchEvent(new Event("input", { bubbles: true }));
		return { ok: true };
	}

	getConsole(): BrowserConsoleEntry[] {
		const entries = [...consoleEntries];
		this.logger.debug("getConsole", { count: entries.length });
		return entries;
	}
}
