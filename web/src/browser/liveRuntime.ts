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
	for (const a of el.attributes) {
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
	getPage(): BrowserPageSnapshot {
		const ae = document.activeElement;
		const focused = ae && ae !== document.body ? snapshotElement(ae, "") : null;
		return {
			url: location.href,
			title: document.title,
			readyState: document.readyState as BrowserPageSnapshot["readyState"],
			focusedElement: focused,
		};
	}

	evalJs(source: string): unknown {
		return new Function(source)();
	}

	querySelector(selector: string): BrowserElementSnapshot | null {
		const el = document.querySelector(selector);
		return el ? snapshotElement(el, selector) : null;
	}

	querySelectorAll(selector: string): BrowserElementSnapshot[] {
		return Array.from(document.querySelectorAll(selector)).map((el) =>
			snapshotElement(el, selector),
		);
	}

	click(selector: string): BrowserToolResult {
		const el = document.querySelector(selector);
		if (!el) {
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
		const el = document.querySelector(selector);
		if (!el) {
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
		return [...consoleEntries];
	}
}
