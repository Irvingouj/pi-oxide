# Async Actor Architecture Plan

## Goal

Replace the blocking single-threaded TUI event loop with a multi-task tokio
actor architecture where **rendering never blocks** and **all mutable state is
owned by a single actor task**.

## Current Problem

```
Synchronous main loop:
  loop {
    terminal.draw(render)  ← renders spinner ONCE
    poll events (33ms)
    if key → submit_prompt
      └─ stream_sync()     ← BLOCKS everything. Spinner frozen.
      └─ terminal.draw()   ← only after stream completes
  }
```

`stream_sync` is a blocking `reqwest::blocking` iterator. The main loop
cannot iterate during LLM streaming, so:
- The spinner animation freezes
- Keyboard events are delayed
- Tool execution blocks rendering

## Target Architecture

```
tokio runtime
├── RenderTask (30fps, lock-free reads)
│   └── reads ArcSwap<RenderSnapshot> → never blocks
├── ActorTask (owns App, processes one message at a time)
│   └── reads from mpsc channel
└── InputTask (poll crossterm events async)
    └── sends Key/Submit/Cancel → mpsc → ActorTask
```

### Data Flow

```
User types ──→ InputTask ──→ AppCmd::Key ──→ channel ──→ ActorTask
                                                            │
LLM HTTP ──→ async chunk stream ──→ AppCmd::LlmChunk ──────┤
                                                            │
Bash stdout ──→ async reader ──→ ToolEvent ────────────────┤
                                                            │
         ┌──────────────────────────────────────────────────┘
         │ the actor owns ALL mutable state
         │ it processes one message at a time
         ▼
    App (Actor)
     │
     │  after each mutation, publish a snapshot
     ▼
    ArcSwap<RenderSnapshot>      ← lock-free pointer swap
     │
     │  read at 30fps
     ▼
    RenderTask
```

### No Lock Contention

- **Actor**: owns App, no locks needed internally (it's the sole writer)
- **RenderTask**: reads `ArcSwap<RenderSnapshot>` — lock-free, always a
  consistent snapshot, never waits
- **Snapshots**: produced after each actor message is processed

### Message Types (AppCmd)

```rust
enum AppCmd {
    Key(KeyEvent),                          // every keystroke (typing, scrolling, etc.)
    Submit(String),                         // user pressed Enter
    LlmChunk(pi_core::LlmChunk),           // streaming LLM chunk arrived
    LlmError(String),                       // LLM stream error
    ToolStarted(ToolCallId, String),        // async tool began
    ToolUpdate(ToolUpdate),                 // streaming tool output
    ToolDone(ToolCallId, Result<ToolResult, ToolError>),
    ToolCancelled(ToolCallId),
    Command(String),                         // slash command
}
```

### RenderSnapshot

```rust
struct RenderSnapshot {
    entries: Arc<[ChatEntry]>,              // cheap Arc clone
    input_text: String,
    input_cursor_pos: usize,
    show_suggestions: bool,
    suggestions: Vec<String>,
    suggestion_selection: Option<usize>,
    running: bool,
    streaming_start: Option<Instant>,       // for spinner frame calculation
    running_tool_names: Vec<(String, bool)>,// (name, is_running)
    scroll_offset: u16,
    auto_scroll: bool,
    last_chat_area: Rect,
    model_name: String,
    last_usage: Option<(u32, u32, u32)>,
    context_window: u32,
    budget: ContextProjectionBudget,
    thinking_level: ThinkingLevel,
    model_picker: Option<ModelPickerSnapshot>,
    show_quit_prompt: bool,
}
```

The snapshot is small (~500 bytes + Arc pointer). Published via
`ArcSwap::store(Arc::new(snapshot))` after every mutation.

---

## Implementation Steps

### Step 1: Async LLM Client

**Files:** `llm/mod.rs`, `llm/stream.rs`

- Convert `LlmClient` from `reqwest::blocking` to `reqwest` (async)
- `LlmStream` becomes a manual async iterator: `pub async fn next(&mut self) -> Option<LlmChunk>`
- SSE parsing logic is identical; only the I/O layer changes
- `build_body`, `parse_sse_event`, `parse_openai_sse_line` are shared unchanged

```rust
pub struct AsyncLlmClient {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    model: String,
    wire_format: WireFormat,
}

pub struct AsyncLlmStream {
    byte_stream: reqwest::ByteStream,
    buffer: String,
    wire_format: WireFormat,
    // ... collected state
}

impl AsyncLlmStream {
    pub async fn next_chunk(&mut self) -> Option<pi_core::LlmChunk> {
        // Try to parse from buffer; if not enough data, fetch next HTTP chunk
    }
}
```

**Verify:** Unit tests for SSE parsing still pass.

### Step 2: RenderSnapshot and ArcSwap

**Files:** `app.rs` (new snapshot module or inline struct)

- Define `RenderSnapshot` struct with all visible state
- Add `ArcSwap<RenderSnapshot>` field to actor struct
- After every mutation, call `publish_snapshot(&self)`

### Step 3: Actor Core (App)

**Files:** `app.rs`

- Make `App` the actor: it owns all mutable state
- Convert key methods to message handlers:
  - `handle_key(KeyEvent)` → `handle(AppCmd::Key)`
  - `submit_prompt(String)` → `handle(AppCmd::Submit)`
  - `stream_llm(...)` → split into async steps
- Each handler mutates state and publishes a snapshot

```rust
impl App {
    fn handle(&mut self, cmd: AppCmd) -> Option<AgentAction> {
        match cmd {
            AppCmd::Key(key) => {
                self.process_key(key);        // mutate input, entries, etc.
                self.publish_snapshot();
                None
            }
            AppCmd::Submit(text) => {
                self.begin_turn(&text);        // mutate entries, start state machine
                self.publish_snapshot();
                Some(AgentAction::StreamLlm { ... })  // return next action
            }
            AppCmd::LlmChunk(chunk) => {
                self.process_chunk(chunk);     // mutate entries, streaming_text
                self.publish_snapshot();
                None
            }
            AppCmd::ToolDone(id, result) => {
                self.on_tool_done(id, result); // mutate entries, transition state
                self.publish_snapshot();
                Some(AgentAction::...)         // return next action (maybe more streaming)
            }
            // ...
        }
    }
}
```

### Step 4: Actor Loop (message pump)

**Files:** `app.rs`

```rust
async fn actor_loop(
    mut app: App,
    mut rx: tokio::sync::mpsc::Receiver<AppCmd>,
    tx_render: Arc<ArcSwap<RenderSnapshot>>,
) {
    while let Some(cmd) = rx.recv().await {
        let next_action = app.handle(cmd);

        // If the handler returned an agent action, execute it
        if let Some(action) = next_action {
            match action {
                AgentAction::StreamLlm { context } => {
                    let stream = app.llm_client.stream(...).await;
                    // Feed chunks back as AppCmd::LlmChunk
                    while let Some(chunk) = stream.next_chunk().await {
                        rx.send(AppCmd::LlmChunk(chunk)).await;
                    }
                }
                AgentAction::ExecuteTools { calls } => {
                    for call in calls {
                        // Spawn tool execution, results come back as AppCmd::ToolDone
                    }
                }
            }
        }
    }
}
```

**Key insight:** The actor processes the agent state machine **inline** during
message handling. When the state machine says "stream LLM", the actor directly
calls the async LLM client and feeds chunks back into its own message channel.
This keeps everything sequential within the actor — no locks.

### Step 5: Input Task

**Files:** `app.rs` or new `input.rs`

```rust
async fn input_task(tx: tokio::sync::mpsc::Sender<AppCmd>) {
    let mut event_stream = crossterm::event::EventStream::new();
    while let Some(Ok(event)) = event_stream.next().await {
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if key.code == KeyCode::Enter && the editor should submit {
                    tx.send(AppCmd::Submit(text)).await;
                } else {
                    tx.send(AppCmd::Key(key)).await;
                }
            }
            Event::Resize(..) => {
                tx.send(AppCmd::Resize).await;
            }
            _ => {}
        }
    }
}
```

### Step 6: Render Task

**Files:** `app.rs` or new `render.rs`

```rust
async fn render_task(
    terminal: &mut ratatui::DefaultTerminal,
    snapshot: Arc<ArcSwap<RenderSnapshot>>,
) {
    let mut interval = tokio::time::interval(Duration::from_millis(33)); // 30fps
    loop {
        interval.tick().await;
        let snap = snapshot.load_full();  // lock-free Arc clone
        terminal.draw(|f| render_frame(f, &snap)).ok();
    }
}
```

`render_frame` is a pure function that takes `&RenderSnapshot` and produces
the entire UI. No mutable state needed.

### Step 7: Wire It Up in main.rs

**Files:** `main.rs`

```rust
#[tokio::main]
async fn main() -> Result<(), TuiError> {
    // Init terminal, config, etc. (unchanged)

    let (tx, rx) = tokio::sync::mpsc::channel(256);
    let render_snapshot = Arc::new(ArcSwap::from_pointee(RenderSnapshot::default()));

    let app = App::new(...);

    // Publish initial snapshot
    app.publish_snapshot_to(&render_snapshot);

    // Spawn render task
    let render_snap = render_snapshot.clone();
    tokio::spawn(async move {
        render_task(&mut terminal, render_snap).await;
    });

    // Spawn input task
    let tx_input = tx.clone();
    tokio::spawn(async move {
        input_task(tx_input).await;
    });

    // Run actor on the main task (or spawn if needed)
    actor_loop(app, rx, render_snapshot).await;

    ratatui::restore();
    Ok(())
}
```

### Step 8: Async Tool Execution

**Files:** `extension.rs`

- Bash extension already spawns threads → convert to `tokio::task::spawn`
- Tool updates flow through the same channel → `AppCmd::ToolUpdate`
- Completion → `AppCmd::ToolDone`

### Step 9: Refactor render_* to be pure functions

**Files:** `tui/ui/chat.rs`, `tui/ui/input.rs`, `tui/ui/mod.rs`

-`render_chat(&self, ...)` → `render_chat(&RenderSnapshot, ...)`
-`render_input(&mut self, ...)` → `render_input(&RenderSnapshot, ...)`
- `render_status(&self, ...)` → `render_status(&RenderSnapshot, ...)`

These become free functions or methods on `RenderSnapshot`.

---

## Files Changed

| File | Change |
|------|--------|
| `Cargo.toml` | Add `arc-swap`, ensure `reqwest` has async features, remove `reqwest/blocking` |
| `llm/mod.rs` | Add `AsyncLlmClient`, `AsyncLlmStream` |
| `llm/stream.rs` | Add async SSE iteration |
| `app.rs` | Actor loop, handle methods, RenderSnapshot, remove old run() |
| `tui/ui/chat.rs` | `render_chat` takes snapshot, not &App |
| `tui/ui/input.rs` | `render_input` takes snapshot, not &mut App |
| `tui/ui/mod.rs` | `render_frame` pure function |
| `tui/llm_stream.rs` | Removed (logic moves into actor loop) |
| `tui/tool_runner.rs` | Convert to async tool execution |
| `extension.rs` | Async tool event stream (tokio channels) |
| `markdown.rs` | No change (pure function) |
| `main.rs` | `#[tokio::main]`, wire up tasks |

---

## Risks & Mitigations

1. **Snapshot size grows with entries** — entries.drain() can be large.
   Use `Arc<[ChatEntry]>` for cheap clone. For very long conversations, only
   include visible entries in snapshot.

2. **Actor message backlog** — if LLM chunks arrive faster than actor
   processes them, channel buffers fill. Use bounded channel with backpressure.

3. **crossterm EventStream with tokio** — `crossterm::event::EventStream`
   is compatible with tokio, but need to ensure terminal is in raw mode first.

4. **ratatui draw across tasks** — `DefaultTerminal` is `Send + Sync`.
   Drawing from a spawned task is safe.

---

## Success Criteria

- [ ] Spinner animation smooth at 30fps during LLM streaming
- [ ] Keyboard input responsive during streaming (Ctrl+C to cancel)
- [ ] All existing tests pass
- [ ] No regressions in slash commands, scrolling, model picker, tool execution
- [ ] No busy-waiting or unnecessary redraws
- [ ] Snapshot publishes are cheap (mostly value copies + Arc)
