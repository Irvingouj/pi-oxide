# TUI Refactor Plan

**Principles:** ADT first, match first, functional first, simplicity first.

---

## Phase 0 — Foundation: Extract the Agent Transition Pattern

The `take → match → mem::take → transition → put back` dance is repeated 5+ times
across `app.rs`, `llm_stream.rs`, and `tool_runner.rs`. Every call site manually:

1. `self.agent.take().unwrap()`
2. `std::mem::take(&mut self.transcript)`
3. `std::mem::take(&mut self.artifacts)`
4. Match `AgentRuntime` exhaustively
5. Call a transition method
6. `.into_parts()`
7. Put everything back

This is the single highest-leverage change. Everything else builds on it.

### 0.1 — `agent_host.rs`: Host-side agent mediator

New module. Defines the ADT that owns the agent + transcript + artifacts + turn_number
and provides pure transition methods.

```rust
/// Owns the agent runtime and its associated host state.
/// Provides methods that encapsulate the take/transition/restore pattern.
pub struct AgentHost {
    runtime: AgentRuntime,
    transcript: Vec<TrimmedMessage>,
    artifacts: Artifacts,
    turn_number: u32,
}
```

Every transition method has the shape:

```rust
fn feed_transition<T, F>(&mut self, f: F)
where
    F: FnOnce(AgentRuntime, Vec<TrimmedMessage>, Artifacts, u32)
        -> TransitionParts,
{
    let runtime = std::mem::replace(&mut self.runtime, /* needs default */);
    let transcript = std::mem::take(&mut self.transcript);
    let artifacts = std::mem::take(&mut self.artifacts);
    let turn = self.turn_number;
    let (events, actions, runtime, transcript, artifacts, turn_number, markers) =
        f(runtime, transcript, artifacts, turn);
    // ... store back
}
```

Actually simpler — just one method:

```rust
/// Execute a transition and replace host state with the result.
fn with_transition(&mut self, f: impl FnOnce(AgentRuntime, &mut TransitionCtx) -> TransitionParts)
    -> (Vec<AgentEvent>, Vec<AgentAction>);
```

Where `TransitionCtx` borrows the current transcript/artifacts/turn_number so the
closure can pass them to the transition call.

**Key insight:** The transition pattern is data, not control flow. The host says
"here's my current state, here's the operation, give me back the new state and
the actions I need to handle."

### 0.2 — Match the AgentRuntime variants explicitly at each call site

After `AgentHost` extracts the boilerplate, each call site still needs to match
the runtime variant to call the right method. This is fine — it's the correct
place for the match. But the match body becomes:

```rust
match runtime {
    AgentRuntime::Streaming(s) => s.finish_llm(result, ctx),
    AgentRuntime::Compacting(c) => c.abort(ctx),
    other => self.default_fallback(other, ctx),
}
```

No more manual `mem::take` or put-back inside the match.

---

## Phase 1 — Split `app.rs` (2205 lines → 4-5 modules)

### 1.1 — `editor.rs`: Input editing state and operations

Extract from `app.rs`:
- `KillRing` struct and impl
- `delete_word_backward`, `delete_word_forward`, `delete_to_line_end`, etc.
- `move_word_backward`, `move_word_forward`
- `yank`, `yank_pop`
- `last_kill_action`, `last_yank` fields

`App` holds an `Editor` struct. All Ctrl/Alt key handlers delegate to it.

### 1.2 — `commands.rs`: Slash-command parser and dispatcher

Extract from `app.rs`:
- `COMMANDS` constant
- `handle_command()` method (~150 lines of match on command strings)
- `update_suggestions()` / suggestion state

New module with a `Command` ADT:

```rust
pub enum Command {
    Clear,
    Help,
    Quit,
    Model { model_id: Option<String> },
    Session { sub: SessionSub },
    Tokens,
    Undo,
    Config,
    Unknown(String),
}

pub enum SessionSub {
    List,
    Load(Option<String>),
    New,
}
```

`parse_command(input: &str) -> Command` is a pure function.
`execute_command(cmd: Command, app: &mut App, terminal: &mut Terminal)` handles
side effects.

### 1.3 — `scroll.rs`: Scroll intent and application

Already partially clean (`derive_scroll_intent`, `apply_scroll` are pure functions
at module level). Move from `app.rs` to its own file. Already good — just relocate.

### 1.4 — `app.rs` after extraction

Should contain:
- `App` struct definition (fewer fields since Editor is extracted)
- `App::new()`
- `App::run()` — main event loop
- `App::handle_key()` — key routing (delegates to editor, commands, scroll)
- `App::render()` — render coordinator
- `App::submit_prompt()` — turn submission (uses `AgentHost`)
- `App::handle_actions()` — action dispatcher (uses `AgentHost`)
- `App::handle_summarize()` — summarize flow (uses `AgentHost`)

Target: ~600-800 lines.

---

## Phase 2 — Split `llm.rs` (1281 lines → 4 modules)

### 2.1 — `llm/client.rs`: LlmClient struct and methods

Move:
- `LlmClient` struct
- `LlmClient::new()`, `model_id()`, `set_model()`
- `LlmClient::stream_sync()` — delegates to wire format
- `LlmClient::build_body()` — dispatches to wire format
- `LlmBackend` type alias
- `LlmProvider` trait (evaluate whether to keep)

### 2.2 — `llm/anthropic_wire.rs`: Anthropic request/response types

Move:
- `AnthropicRequest`, `AnthropicTool`
- `AnthropicContentBlock`, `AnthropicUserMessage`, `AnthropicAssistantMessage`
- `AnthropicToolResultBlock`, `AnthropicToolResultMessage`
- `convert_messages()` function

### 2.3 — `llm/openai_wire.rs`: OpenAI request/response types

Move:
- `OpenAIRequest`, `OpenAITool`, `OpenAIFunction`
- `OpenAISystemMessage`, `OpenAIUserMessage`, `OpenAIAssistantMessage`
- `OpenAIToolCall`, `OpenAIFunctionCall`, `OpenAIToolResultMessage`
- `convert_messages_openai()` function

### 2.4 — `llm/sse.rs`: SSE stream parsing

Move:
- `LlmStream` struct and `Iterator` impl
- `parse_sse_event()` → typed Anthropic SSE structs
- `parse_openai_sse_line()` → typed OpenAI SSE structs
- `PartialToolCall`, `CollectedToolCall`
- `LlmStreamState` trait (evaluate whether to keep)

**Critical fix:** Parse SSE JSON into typed structs instead of hand-walking
`serde_json::Value`. Define:

```rust
#[derive(serde::Deserialize)]
struct AnthropicContentBlockDelta {
    pub delta: ContentDelta,
}

#[derive(serde::Deserialize)]
#[serde(tag = "type")]
pub enum ContentDelta {
    #[serde(rename = "text_delta")]
    TextDelta { text: String },
    #[serde(rename = "input_json_delta")]
    InputJsonDelta { partial_json: String },
}
```

This eliminates 50+ chained `.get().and_then()` calls.

### 2.5 — `llm/mod.rs`: Re-exports and `WireFormat` enum

Keep `WireFormat`, `ModelInfo`, `ModelDiscovery` trait, and re-exports.

---

## Phase 3 — Fix runtime issues

### 3.1 — Render throttling in `llm_stream.rs`

Accumulate chunks for ~50ms before re-rendering. Simple timer-based batching:

```rust
let mut batch_start = std::time::Instant::now();
let mut render_interval = Duration::from_millis(50);

for chunk in stream.by_ref() {
    // ... process chunk ...
    if batch_start.elapsed() >= render_interval {
        let _ = terminal.draw(|f| self.render(f));
        batch_start = std::time::Instant::now();
    }
}
```

### 3.2 — Duplicate `home_dir()`

One canonical implementation. Use `std::env::home_dir()` everywhere (it already
handles `HOME`/`USERPROFILE`).

### 3.3 — `ChatEntry::ToolResult.name` confusion

Rename to `tool_call_id` and add `tool_name` field, or restructure so the
connection between `ToolStart` and `ToolResult` is explicit.

### 3.4 — Markdown Strong/Emphasis no-ops

Either implement styling or remove the dead match arms.

---

## Phase 4 — Polish

### 4.1 — `model_picker.rs`: Cache filtered results

Don't re-allocate on every call. Cache the filtered list and invalidate on
filter change.

### 4.2 — `extension.rs`: Extract bash stdout/stderr reader

The two reader threads are identical except for stream/buffer. Extract to:

```rust
fn read_stream(
    stream: std::process::Stdio,
    label: ToolOutputStream,
    tx: Sender<ToolEvent>,
    seq: Arc<Mutex<u64>>,
    buf: Arc<Mutex<Vec<u8>>>,
    tool_call_id: ToolCallId,
) {
    // ... shared logic ...
}
```

### 4.3 — `tools.rs`: DRY grep dir/file logic

Single `search_file()` function called by both paths.

### 4.4 — Test fixtures

`with_entries_for_test()` and `app_with_agent()` should use `..Default::default()`
or a builder pattern to avoid duplicating the full struct construction.

---

## Execution Order

```
Phase 0.1  AgentHost ADT              (new file, no breakage)
Phase 0.2  Migrate call sites         (app.rs, llm_stream.rs, tool_runner.rs)
Phase 1.1  Editor extraction           (app.rs → editor.rs)
Phase 1.2  Commands extraction         (app.rs → commands.rs)
Phase 1.3  Scroll relocation           (app.rs → scroll.rs)
Phase 1.4  App cleanup                 (remove extracted code)
Phase 2.1  LLM client split            (llm.rs → llm/client.rs)
Phase 2.2  Anthropic wire              (llm.rs → llm/anthropic_wire.rs)
Phase 2.3  OpenAI wire                 (llm.rs → llm/openai_wire.rs)
Phase 2.4  SSE parsing + typed structs (llm.rs → llm/sse.rs)
Phase 2.5  LLM mod.rs                  (re-exports)
Phase 3.1  Render throttling
Phase 3.2  home_dir dedup
Phase 3.3  ToolResult rename
Phase 3.4  Markdown no-ops
Phase 4.x  Polish items
```

Each phase compiles independently. Tests run after each phase.
