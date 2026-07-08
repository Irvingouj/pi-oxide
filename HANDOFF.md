# Handoff: pi-host-tui async actor refactoring

## Current State (2026-07-08)

### What works
- Architecture: Two tasks (render at 30fps + actor loop) on Tokio `current_thread` runtime
- `run_async` in `main.rs` spawns render task, awaits actor loop, **does abort render_handle** on exit
- Input polling on a separate `std::thread` via `mpsc::channel`
- `App::submit_text()` correctly stores `pending_llm_context` via `process_directives`
- `RenderSnapshot` + `ArcSwap` for lock-free reads by render task

### What's broken — compilation

**`pi-host-tui/src/app.rs` has an unclosed delimiter** — stale debug-log cruft from earlier edits left the file in an inconsistent state. The `run_actor_loop` function body was partially cleaned via `sed`, but orphaned braces exist and the function lost its entry tracing.

### What's broken — runtime (replay test)

The core bug: **`submit_text()` never calls `stream_sync()` anymore** — my earlier edit accidentally removed it. This means:

1. User presses Enter → `handle_key` returns false → actor loop calls `submit_text(&text)`
2. `submit_text` transitions agent through `start_turn`, gets `StreamLlm { context }` directive
3. **Old behavior**: called `self.llm_client.stream_sync(...)`, collected chunks into `self.pending_chunks` — replay/record compatible because the client respects cassettes
4. **Current behavior**: only stores `self.pending_llm_context = Some(context)` — **never collects chunks**
5. `process_stream_llm_async` checks `pending_chunks.take()` → `None` → falls through to `AsyncLlmClient` which makes real HTTP calls (not replay-friendly)

Result: spinner renders forever, no assistant text appears, test assertion fails.

### Key file: `pi-host-tui/src/app.rs`

- Line 1167: `StreamLlm` / `Summarize` directive handler — **this is where `.stream_sync()` was removed**
- Line 1414: `process_stream_llm_async` — has a `pending_chunks` guard that's never hit because nothing populates it
- `submit_text` (around line 1018) — the `process_directives` loop handles `StreamLlm` at line 1167

The code for collecting chunks (with `usage()`, `stop_reason()`, `tool_calls()`, then `.collect::<Vec<_>>()` into `pending_chunks`) exists in git history but was replaced by just `self.pending_llm_context = Some(context)`.

### Cleanup needed

- Remove stale `trace_log` calls that were added for debugging
- Fix indentation consistency in `run_actor_loop` (the submit-text block has misaligned braces from the sed edit)
- The `run_actor_loop` entry lost its `crate::trace_log("run_actor_loop: starting")` — add it back or leave clean

### The fix (conceptual)

In `submit_text`'s `process_directives` loop, the `StreamLlm` and `Summarize` arms need to:

```rust
// Collect chunks using App's LLM client (respects replay/record features).
let stream = match self.llm_client.stream_sync(
    &context.system_prompt,
    &context.messages,
    &context.tools,
) {
    Ok(s) => s,
    Err(e) => { tracing::error!(?e, "Failed to start LLM sync stream"); continue; }
};

// Collect metadata before consuming iterator
let usage = stream.usage();
let stop_reason = stream.stop_reason().map(|s| s.to_string());
let tool_calls: Vec<CollectedToolCall> = stream.tool_calls()
    .into_iter()
    .map(|tc| CollectedToolCall { id: tc.id, name: tc.name, input: tc.input })
    .collect();

let chunks = stream.collect::<Vec<_>>();
self.pending_llm_context = Some(context);
self.pending_chunks = Some(chunks);
self.pending_stream_usage = usage;
self.pending_stop_reason = stop_reason.unwrap_or_else(|| "end_turn".into());
self.pending_tool_calls = tool_calls;
```

This is what was there before and was accidentally deleted.

### How to verify

```bash
cd /Users/oujunyi/code/pi-oxide
cargo test --package pi-host-tui replay_cassette_multi_turn_offline
```

Expected: assistant text appears (Turn 1: "hello", Turn 2: "Fifty-six"), test passes.
Diagnostic: check `/tmp/pio_debug.log` (file-based debug logging) after removing stale traces.

### Next steps for next agent

1. **Fix compilation**: `cargo check --package pi-host-tui` — the unclosed delimiter at line ~1863 needs resolving. Best approach: `git diff pi-host-tui/src/app.rs` then look at the `run_actor_loop` section for orphaned `}` or missing `)`.
2. **Restore `stream_sync` collection** in `submit_text`'s `StreamLlm`/`Summarize` handler (concept above)
3. **Remove stale trace_log calls** and fs::OpenOptions cruft
4. **Run replay test** to verify

---

*Generated during a stalled edit session where exact-match edits failed due to whitespace drift from earlier partial changes. Recommend reading the actual file from scratch rather than assuming any "oldText" anchors from this doc.*
