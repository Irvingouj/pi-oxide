# Milestone 8: Browser Host Native Tools

This milestone pivots the browser direction away from local coding tools.

The browser host should do what browsers do. Do not copy local `read/write/edit/bash` into the browser as the first browser milestone.

## Read First

- `CLAUDE.md`
- `ROADMAP.md`
- `design.md`
- `CONTEXT_PROJECTION_SPEC.md`
- `MILESTONE_7_LOCAL_SESSION_PERSISTENCE.md`
- `web/src/wasmBinding.ts`
- `web/src/context/rustProjection.ts`
- `web/src/providers/realLlm.ts`
- `web/src/tools/schemas.ts`

## Goal

Prove a minimal browser-native agent host:

```text
Rust core + WASM lifecycle
-> browser host tools
-> JS eval / DOM inspect / event dispatch / console observation
-> fake or real provider loop
```

Rust core remains runtime-neutral. Browser APIs stay in the JS browser host.

## Browser-Native Tool Surface

First-pass browser tools should be browser-native:

- `browser_get_page`
  - return URL, title, ready state, focused element summary
- `browser_eval_js`
  - evaluate JavaScript in the page context
  - return JSON-serializable result or typed error
- `browser_query_selector`
  - query one or many elements
  - return tag, text preview, attributes, visibility-ish metadata
- `browser_click`
  - click an element by selector
- `browser_type`
  - type text into an element by selector
- `browser_console`
  - read collected console logs/errors

Optional if cheap:

- `browser_dispatch_event`
- `browser_get_storage`
- `browser_set_storage`

## Explicit Non-Goals

- No local `read/write/edit/bash` browser clone.
- No bash in browser unless backed by explicit remote/sandbox runner.
- No File System Access API in this milestone.
- No OPFS/IndexedDB persistence in this milestone.
- No UI unless a tiny test harness page is needed.
- No screenshot requirement yet.
- No provider-specific formatting in Rust.
- No JS-owned context projection.

## Suggested Files

- `web/src/browser/browserTools.ts`
- `web/src/browser/browserHost.ts`
- `web/src/browser/consoleCapture.ts`
- `web/test/browserHost.test.ts`

If Node's built-in test runner cannot provide a DOM, keep the DOM dependency minimal and explicit. Prefer testing pure tool logic against a small fake DOM adapter if adding a DOM library would expand scope too much.

## Architecture

Define a browser runtime adapter so tests can use a fake DOM and the real browser can use `window`/`document` later:

```ts
export interface BrowserRuntime {
  getPage(): BrowserPageSnapshot;
  evalJs(source: string): unknown;
  querySelector(selector: string): BrowserElementSnapshot | null;
  querySelectorAll(selector: string): BrowserElementSnapshot[];
  click(selector: string): BrowserToolResult;
  type(selector: string, text: string): BrowserToolResult;
  getConsole(): BrowserConsoleEntry[];
}
```

The host tool registry should execute browser tools through this adapter.

## Tool Result Policy

Browser tool results are still text/JSON payloads to Rust, but include useful typed details:

- selected selector
- matched element count
- result type
- console severity
- truncation flag when text is large

Large DOM text or console output should be bounded before entering the model context. Rust context projection remains the owner of general projection policy.

## Tests

Add network-free tests:

- `browser_get_page` returns URL/title/ready state
- `browser_eval_js` returns JSON-serializable values
- `browser_eval_js` returns typed error for thrown exception
- `browser_query_selector` returns element summaries
- `browser_click` dispatches click through fake runtime
- `browser_type` changes fake input value
- `browser_console` returns captured logs/errors
- unknown browser tool returns typed error
- fake LLM can drive Rust agent loop with browser tools
- context projection still runs for browser tool results
- no browser tool imports or assumptions appear in `pi-core`

## Verification

Run:

```bash
cargo test --workspace
cd web && npm test
```

## Report Back

Report:

- changed files
- browser tool names and schemas
- browser runtime adapter shape
- test strategy fake DOM vs real DOM
- test results
- any intentionally deferred browser capabilities

Do not commit.
