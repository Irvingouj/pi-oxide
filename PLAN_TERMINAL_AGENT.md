# Plan: Rust Terminal Coding Agent (pi-host-terminal)

## Goal

Build a minimal usable terminal coding agent in Rust, as usable as `../pi` in its raw state. The agent runs in the terminal, uses the existing `pi-core` state machine, streams LLM responses, executes real tools (bash, read, write, edit), and renders everything with ratatui.

## Success Criteria (Must Pass Before Ship)

1. **Start a conversation**: `cargo run -p pi-host-terminal`, type a prompt, get a streaming response
2. **Tool calls rendered**: Agent calls bash/read/write/edit, user sees tool name + args + result
3. **Streaming text**: LLM response streams token-by-token, not batch
4. **Markdown rendered**: Code blocks with syntax highlighting, bold/italic/links styled
5. **Multi-turn**: Agent loops — LLM → tool → LLM → done. User can send another message
6. **Context budget**: Uses the existing Rust projection engine (microcompact + compaction signal)
7. **Graceful exit**: Ctrl+C restores terminal, no garbled state

## Architecture

```
pi-host-terminal/
  src/
    main.rs          — CLI args, tokio runtime, terminal setup
    app.rs           — App state, event loop, render dispatch
    agent_driver.rs  — Drives pi-core Agent: handles actions, feeds events
    llm.rs           — Anthropic API streaming (reqwest + SSE)
    tools.rs         — Tool execution: bash, read, write, edit
    render/
      mod.rs
      chat.rs        — Chat message area (scrollable)
      input.rs       — User input box
      status.rs      — Footer: model, tokens, context %
      markdown.rs    — Markdown → ratatui Text with syntax highlighting
```

**No new abstractions in pi-core.** The terminal host is a pure consumer of the existing Agent API.

## Key Design Decisions

### 1. Reuse pi-host-desktop scaffold

`pi-host-desktop` already exists with `ratatui`, `crossterm`, `reqwest`, `tokio`, `clap` in its Cargo.toml. Rename it or extend it — don't create a new crate from scratch.

### 2. Direct Agent API, no WASM boundary

The terminal host calls `Agent::new()`, `agent.start_turn()`, `agent.feed_llm_chunk()`, etc. directly. No JSON serialization, no envelope parsing. This is the intended use of pi-core.

### 3. tokio::select! event loop

```
tokio::select! {
    _ = render_interval.tick() => terminal.draw(render),
    Some(event) = crossterm_events.next() => handle_input(event),
    Some(chunk) = llm_stream.recv() => agent.feed_llm_chunk(chunk),
}
```

The render loop runs at 30fps (16ms is overkill for text). LLM chunks arrive via a channel from a background reqwest streaming task.

### 4. Markdown via pulldown-cmark + syntect

No external `tui-markdown` crate — we need fine control over rendering (streaming partial markdown, tool call blocks, etc.). Use `pulldown-cmark` to parse, then walk events and produce `ratatui::text::Text` with styled spans. Use `syntect` for code block highlighting.

## Implementation Plan

### Phase 1: Skeleton — app loop + input + empty render

**Files**: `main.rs`, `app.rs`

- `ratatui::init()` / `ratatui::restore()` with panic hook
- tokio event loop with crossterm `EventStream`
- Split layout: chat area (top 80%) + input area (bottom 20%) + status bar (1 line)
- User types text, presses Enter, text appears in chat area
- Ctrl+C quits cleanly

**Test**: Run it, type "hello", see it in chat area, Ctrl+C exits cleanly.

### Phase 2: Agent driver — pi-core integration

**Files**: `agent_driver.rs`

- Construct `Agent` with `AgentOptions` (system prompt, model, tools)
- `start_turn(user_message)` → returns `StreamLlm` action
- Handle `StreamLlm`: call LLM (stubbed for now), call `on_llm_done`
- Handle `ExecuteTools`: call tools (stubbed), call `on_tool_done`
- Handle `Finished`: return control to user
- Wire into app.rs: on Enter, call agent_driver, display events

**Test**: Unit test that creates an Agent, feeds a turn, handles the StreamLlm action.

### Phase 3: LLM streaming — Anthropic API

**Files**: `llm.rs`

- `reqwest` streaming POST to Anthropic Messages API
- Parse SSE events (`event: content_block_delta`, `event: message_stop`, etc.)
- Convert to `LlmChunk` variants, send through `mpsc` channel
- Handle errors, API key from env var `ANTHROPIC_API_KEY`
- Also support `base_url` override (for Fireworks, etc.)

**Test**: Integration test that streams a real response (optional, requires API key). Unit test SSE parsing.

### Phase 4: Tool execution

**Files**: `tools.rs`

Four tools with minimal implementations:

| Tool | Implementation |
|------|---------------|
| **bash** | `tokio::process::Command`, stream stdout/stderr, timeout 120s |
| **read** | `std::fs::read_to_string`, with offset/limit params |
| **write** | `std::fs::write`, creates parent dirs |
| **edit** | String replacement (old_string → new_string), write back |

Tool definitions (`ToolDefinition`) match the JSON Schema shapes from `pi-core/src/tool.rs`.

**Test**: Unit test each tool — bash runs `echo hello`, read reads a temp file, etc.

### Phase 5: Chat rendering — messages + tool calls

**Files**: `render/chat.rs`, `render/markdown.rs`

Chat area is a scrollable list of rendered messages:

```
┌─────────────────────────────────────────────────────┐
│ User: build me a web server in Rust                  │  <- user msg
│                                                      │
│ I'll create a basic Axum web server. Let me set up   │  <- assistant msg
│ the project structure.                               │  (streaming, markdown)
│                                                      │
│ ┌─ bash ──────────────────────────────────────────┐  │  <- tool call
│ │ $ cargo init my-server                           │  │
│ │     Creating binary package                      │  │
│ │ ✓ finished in 0.3s                               │  │
│ └──────────────────────────────────────────────────┘  │
│                                                      │
│ Now let me write the server code...                  │  <- continues
└─────────────────────────────────────────────────────┘
```

- **User messages**: Bold "You:" prefix, text in white
- **Assistant messages**: Rendered as markdown via `render/markdown.rs`
- **Tool calls**: Bordered block showing tool name + args + result
  - During execution: spinner + streaming output
  - On completion: result text (truncated to N lines)
  - On error: red border + error message
- **Scrolling**: Auto-scroll to bottom on new content, manual scroll with mouse/j/k

### Phase 6: Markdown renderer

**Files**: `render/markdown.rs`

Parse markdown with `pulldown-cmark`, render to `ratatui::text::Text`:

| Element | Rendering |
|---------|-----------|
| `**bold**` | `Modifier::BOLD` |
| `*italic*` | `Modifier::ITALIC` |
| `` `code` `` | Yellow fg, DarkGray bg |
| ```` ```lang ... ``` ```` | Syntect-highlighted, DarkGray bg, indented |
| `# heading` | Bold + Cyan fg |
| `- list item` | Bullet with dim prefix |
| `[text](url)` | Blue fg, underlined |
| `> quote` | Dim border `│`, italic |

Streaming optimization: cache rendered Text, only re-parse on change. For streaming partial markdown (incomplete code blocks), the parser handles it gracefully.

### Phase 7: Status bar + context budget

**Files**: `render/status.rs`

Bottom line shows:

```
 ● claude-sonnet-4-20250514 │ in: 12.4k out: 3.2k │ ctx: 47% ██████░░░░ │ tools: 6
```

- Model name from agent state
- Token counts from last `TokenUsage`
- Context percentage: `estimated_tokens / max_context_tokens`, color-coded (green <70%, yellow 70-90%, red >90%)
- Show "COMPACTING..." when `report.needs_compaction` is true

### Phase 8: Context projection integration

Before each LLM call, run `project()` from `pi-core::context_projection`:

```rust
let output = project(ProjectionInput {
    system_prompt: context.system_prompt,
    messages: context.messages,
    budget: self.budget.clone(),
    state: self.projection_state.clone(),
});
self.projection_state = output.updated_state;
// Feed projected messages to LLM instead of raw messages
```

Feed `output.report.estimated_tokens` to the status bar.

## Dependencies (add to pi-host-desktop/Cargo.toml)

```toml
[dependencies]
pi-core = { path = "../pi-core" }
pi-llm = { path = "../pi-llm" }
tokio = { workspace = true }
ratatui = { workspace = true }
crossterm = { workspace = true }
clap = { workspace = true }
reqwest = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
tracing = { workspace = true }
thiserror = { workspace = true }
pulldown-cmark = "0.13"
syntect = "5.2"
futures = "0.3"
```

## File-by-File Implementation Guide

### `main.rs` (~60 lines)

```rust
use clap::Parser;

#[derive(Parser)]
#[command(name = "pi", about = "Terminal coding agent")]
struct Cli {
    /// Model ID (e.g. claude-sonnet-4-20250514)
    #[arg(long, env = "PI_MODEL", default_value = "claude-sonnet-4-20250514")]
    model: String,
    /// API base URL
    #[arg(long, env = "PI_BASE_URL")]
    base_url: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let mut terminal = ratatui::init();
    let app = App::new(cli.model, cli.base_url);
    let result = app.run(&mut terminal).await;
    ratatui::restore();
    result
}
```

### `app.rs` (~200 lines)

Main event loop:

```rust
pub struct App {
    agent: Agent,
    chat_lines: Vec<ChatLine>,       // rendered chat history
    input: String,                   // user input buffer
    scroll_offset: u16,              // chat scroll position
    streaming_text: String,          // current streaming LLM response
    tool_executions: Vec<ToolExec>,  // active tool executions
    status: StatusInfo,              // token counts, model name
    should_quit: bool,
}

enum ChatLine {
    User(String),
    Assistant(Text<'static>),     // pre-rendered markdown
    ToolCall(ToolCallRender),
    Error(String),
}

pub async fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
    let mut events = EventStream::new();
    let render_interval = tokio::time::interval(Duration::from_millis(33)); // ~30fps

    loop {
        tokio::select! {
            _ = render_interval.tick(), if !self.should_quit => {
                terminal.draw(|f| self.render(f))?;
            }
            Some(Ok(event)) = events.next() => {
                self.handle_event(event).await?;
            }
        }
        if self.should_quit { break; }
    }
    Ok(())
}
```

### `agent_driver.rs` (~150 lines)

Drives the pi-core agent:

```rust
pub struct AgentDriver {
    agent: Agent,
    projection_state: ContextProjectionState,
    budget: ContextProjectionBudget,
}

impl AgentDriver {
    pub fn new(model_id: &str, base_url: Option<&str>) -> Self { ... }

    pub fn start_turn(&mut self, text: &str) -> Vec<Action> {
        let (events, actions) = self.agent.start_turn(AgentMessage::user(text));
        // Return actions for the app to handle
        actions
    }

    pub fn feed_chunk(&mut self, chunk: LlmChunk) -> Vec<AgentEvent> {
        self.agent.feed_llm_chunk(chunk)
    }

    pub fn finish_llm(&mut self, result: LlmResult) -> Vec<Action> {
        let (_, actions) = self.agent.on_llm_done(result);
        actions
    }

    pub fn finish_tool(&mut self, id: &str, result: Result<ToolResult, ToolError>) -> Vec<Action> {
        let (_, actions) = self.agent.on_tool_done(
            ToolCallId::new(id),
            result,
        );
        actions
    }

    pub fn project_context(&mut self, context: &LlmContext) -> ProjectionOutput {
        let output = project(ProjectionInput {
            system_prompt: context.system_prompt.clone(),
            messages: context.messages.clone(),
            budget: self.budget.clone(),
            state: self.projection_state.clone(),
        });
        self.projection_state = output.updated_state.clone();
        output
    }
}
```

### `llm.rs` (~200 lines)

Anthropic API streaming:

```rust
pub struct LlmClient {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    model: String,
}

impl LlmClient {
    pub async fn stream(
        &self,
        context: &LlmContext,
        sender: mpsc::UnboundedSender<LlmChunk>,
    ) -> Result<()> {
        let body = self.build_request_body(context);
        let response = self.client.post(url).headers(headers).json(&body).send().await?;

        // Parse SSE stream
        let mut stream = response.bytes_stream();
        while let Some(bytes) = stream.next().await {
            for line in parse_sse_lines(&bytes?) {
                match line.event_type.as_str() {
                    "content_block_delta" => sender.send(LlmChunk::TextDelta { text: ... }),
                    "message_start" => sender.send(LlmChunk::Start { ... }),
                    "message_stop" => break,
                    ...
                }
            }
        }
        Ok(())
    }
}
```

### `tools.rs` (~150 lines)

```rust
pub fn execute_tool(call: &ToolCall) -> Result<ToolResult, ToolError> {
    match call.name.as_str() {
        "bash" => execute_bash(&call.arguments),
        "read" => execute_read(&call.arguments),
        "write" => execute_write(&call.arguments),
        "edit" => execute_edit(&call.arguments),
        _ => Err(ToolError { code: "unknown_tool".into(), message: format!("unknown tool: {}", call.name), details: None }),
    }
}

fn execute_bash(args: &ToolArguments) -> Result<ToolResult, ToolError> {
    let command = args.0.get("command").and_then(|v| v.as_str()).ok_or(...)?;
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .output()
        .map_err(|e| ToolError { code: "exec_failed".into(), message: e.to_string(), details: None })?;
    let text = if output.status.success() {
        String::from_utf8_lossy(&output.stdout).to_string()
    } else {
        format!("exit code: {}\n{}", output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr))
    };
    Ok(ToolResult::text(&text))
}
```

## Testing Strategy

### Unit tests (cargo test -p pi-host-terminal)

1. **Agent driver**: Create Agent, start_turn with test prompt, verify StreamLlm action returned
2. **Tools**: Each tool in isolation — bash echoes, read reads temp file, write writes temp file, edit replaces string
3. **Markdown renderer**: Input markdown string, verify output Text has correct styles
4. **SSE parser**: Feed mock SSE lines, verify correct LlmChunk variants emitted
5. **Context projection**: Feed messages through project(), verify budget enforcement

### Integration test (manual)

1. Start agent, type "what files are in the current directory?", verify bash tool runs and output shown
2. Multi-turn: "create a file called hello.txt with 'world' in it" → verify write + read tools execute
3. Context overflow: send many messages, verify context % rises, compaction triggers
4. Ctrl+C mid-stream: verify terminal restored cleanly

## Risks and Mitigations

| Risk | Mitigation |
|------|-----------|
| Terminal corruption on panic | `ratatui::init()` installs panic hook automatically |
| LLM API changes | SSE parser follows Anthropic's documented event types, not brittle regex |
| Async + sync boundary (pi-core is sync) | Use `tokio::task::spawn_blocking` for Agent calls if needed, but Agent is fast enough to call directly |
| Markdown rendering incomplete | Start with `pulldown-cmark` default features, add edge cases as we encounter them |
| Tool execution safety | Bash is dangerous by design — same model as pi/Claude Code. User consent via input. |

## What This Is NOT

- Not a full IDE integration
- Not a multi-session manager
- Not a replacement for the browser host
- Not configurable themes (for v1)
- Not MCP tool support (for v1)

This is the **minimal viable terminal coding agent** that proves the Rust stack works end-to-end.
