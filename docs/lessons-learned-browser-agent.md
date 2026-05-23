# Lessons Learned: Browser Agent WASM Integration

Notes from building the pi-oxide browser agent — a Rust WASM agent that runs
inside a web page, talks to a real LLM (Fireworks API), and interacts with
the DOM through browser tools.

## 1. "Memory Access Out of Bounds" Was Three Different Bugs

Passing JS objects where Rust expected `&str` (via wasm-bindgen). Each one
looked like the same error but had a different root cause:

- `onToolDone(handle, call.id, payload)` — `payload` was a JS object, not
  `JSON.stringify(payload)`.
- `onLlmDone(handle, errResult)` in the error recovery catch block — same
  problem, raw object instead of string.
- `execBrowserTool` threw an unhandled DOMException from
  `document.querySelector("button:contains('Click me')")` which propagated
  up and hit the error recovery path with the raw-object bug.

**Moral:** In WASM interop, type boundaries are where everything breaks, and
the error message tells you nothing about which side is wrong. Always
`JSON.stringify()` before passing to Rust.

## 2. WASM Tracing Is a Nightmare of Target Incompatibility

We tried multiple approaches to get Rust `tracing` output in the browser:

- `web_sys::console::log_1()` → works in browser, breaks Node (missing
  `__wbg_log_*` import).
- `js_sys::Reflect::get()` → same problem, generates WASM imports the Node
  shim doesn't provide.
- `#[wasm_bindgen(inline_js = "...")]` → only works with `--target web`,
  not `--target nodejs`.
- `tracing-wasm` crate → causes wasm-bindgen panic with
  "invalid binary op I32GtU".

**Solution:** A buffer-based `ConsoleSubscriber` that stores trace messages
in a `thread_local! { RefCell<Vec<String>> }`. No JS imports needed. A
`drainTraceLog()` WASM export lets JS poll and log them. Works identically
in both browser and Node.

**Lesson:** Never call JS from Rust in a way that generates WASM imports —
they'll break one target or the other. Keep it pure Rust, return data, let
JS pull it.

## 3. Homebrew vs Rustup Cargo Causes Silent Build Failures

`cargo` on PATH was Homebrew 1.94, `rustup` was 1.95 with the wasm32 target
installed. The error said "target not installed" even though it was. The fix:

```bash
export PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"
cargo build --target wasm32-unknown-unknown --release
```

**Lesson:** Always verify `which cargo` and `cargo --version` match the
toolchain that has the target. Or just use `rustup run stable cargo build`.

## 4. The WASM Toolchain Is Fragile

- Build with wrong Cargo → no error about versions, just "can't find `core`".
- `wasm-bindgen` versions must match the `wasm-bindgen` crate version.
- Two output targets (`--target web` vs `--target nodejs`) each need
  different JS shims in different directories.
- Every Rust code change requires: rebuild → regenerate web target →
  regenerate nodejs target → retest both.

This is the biggest friction point. A single `build:wasm` npm script that
does all three steps helps.

## 5. Fireworks API Quirks

- **Thinking blocks** come mixed with `tool_use` content blocks. Filter them:
  `.filter(b => b.type === 'text' || b.type === 'tool_use')`.
- **CORS blocks** `x-api-key` and `anthropic-version` headers. Use
  `Authorization: Bearer ${apiKey}` instead.
- **Tool call IDs** look like `functions.browser_get_page:0` — weird but
  works fine as strings.
- **`stop_reason: "tool_use"`** maps to our `tool_use`, everything else is
  `end_turn`.

These aren't documented anywhere obvious. You find them by trial and error.

## 6. Vite Env Injection Is Clever but Subtle

- `import.meta.env.VITE_*` gets replaced at build/dev time by Vite.
- When served statically (our Playwright tests), `import.meta.env` is
  `undefined`. Guard with:
  ```js
  const viteEnv = typeof import.meta !== 'undefined' && import.meta.env
    ? import.meta.env : {};
  ```
- `envDir` in `vite.config.ts` is relative to `root`, not `cwd`. With
  `root: 'public'`, you need `envDir: '../'` to find `.env.development`.
- Don't name your env variable `env` — it collides with other local vars
  (we had one in projection code).

## 7. The Plan Said "No Rust Changes Needed" — Wrong

The original PLAN_BROWSER_AGENT.md said "不需要修改 Rust 代码". In reality
we needed:

- `ConsoleSubscriber` + `drainTraceLog()` for debugging.
- Extensive `tracing::info!` / `debug!` / `error!` calls throughout all WASM
  exports.
- Understanding of the envelope `{ok, data, error}` format from the JS side.

**Lesson:** Plans underestimate integration work by 3-5x. The "glue" between
systems is where the hard bugs live.

## 8. Keep Agent Loop State Machines Simple

The JS agent loop went through several messy iterations. The clean pattern:

```js
async function agentLoop(handle, actions) {
  for (const action of actions) {
    switch (action.type) {
      case 'stream_llm':    // call LLM, feed result, recurse
      case 'execute_tools': // run tools, feed results, recurse
      case 'finished':      // done
    }
  }
}
```

Each action type handles itself and recurses with the new actions. No global
state, no promises to track, no async generators. Simple recursion over a
list.

## 9. Playwright E2E Tests Are Worth Their Weight in Gold

We went from "maybe it works, I can't tell" to "6 tests, all passing, real
LLM calls, 24 seconds." Each test caught a different bug:

| Test | Bug Found |
|------|-----------|
| Click counter | Memory access OOB from passing objects to WASM |
| Type in form | DOMException from invalid CSS selector |
| Eval JS | Serialization issues |
| Describe page | Tool result parsing |
| IndexedDB | Persistence edge cases |

They're the reason we could iterate fast with confidence. Write E2E tests
early, run them often.

## 10. Ship Early, Debug with Real LLM

We spent time trying to reason about the memory bug theoretically. The moment
we added tracing and ran it with the real Fireworks API, the actual crash
path was obvious in the logs:

```
[agent] calling onToolDone id=... payload_len=189
onLlmDone FAILED: memory access out of bounds
```

That one line immediately pointed to `onToolDone` receiving a JS object
instead of a string. Real execution > theoretical reasoning. Every time.
