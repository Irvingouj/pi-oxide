# Milestone 6.5: Local Tool Runtime Control

Implement this after Milestone 6 real local coding smoke.

## Read First

- `CLAUDE.md`
- `ROADMAP.md`
- `LOCAL_TOOL_RUNTIME_SPEC.md`
- `AGENT_RUNTIME_MEMO.md`
- `web/src/providers/realLlm.ts`
- `web/src/local/bashTool.ts`
- `web/src/local/localToolRegistry.ts`
- `pi-core/src/events.rs`
- `pi-core/src/agent.rs`
- `pi-host-web/src/lib.rs`

## Goal

Move local tool execution from smoke-test grade synchronous execution toward a real host runtime:

```text
Rust core = synchronous event reducer
JS host = async tool/process runtime
WASM boundary = typed callbacks from JS into Rust
```

Rust core must not execute processes, read files, own Node APIs, or assume async runtime. It should only track tool lifecycle and emit typed events/actions.

## Required Architecture

### Rust Core

Core should understand async tool lifecycle as state, not as runtime.

Add typed concepts equivalent to:

```rust
pub enum ToolRunStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

pub enum ToolOutputStream {
    Stdout,
    Stderr,
    Status,
}

pub struct ToolExecutionUpdate {
    pub tool_call_id: ToolCallId,
    pub stream: ToolOutputStream,
    pub chunk: String,
    pub sequence: u64,
    pub timestamp: u64,
}
```

Add synchronous APIs equivalent to:

```rust
pub fn on_tool_started(&mut self, tool_call_id: ToolCallId) -> Vec<AgentEvent>;

pub fn on_tool_update(&mut self, update: ToolExecutionUpdate) -> Vec<AgentEvent>;

pub fn on_tool_cancelled(
    &mut self,
    tool_call_id: ToolCallId,
    reason: CancelReason,
) -> (Vec<AgentEvent>, Vec<AgentAction>);
```

Existing `on_tool_done` remains the final completion callback.

### Events

Expose typed events such as:

```rust
AgentEvent::ToolExecutionStart { ... }
AgentEvent::ToolExecutionUpdate { tool_call_id, stream, chunk, sequence, timestamp }
AgentEvent::ToolExecutionEnd { ... }
AgentEvent::ToolExecutionCancelled { tool_call_id, reason }
```

Do not put every stdout/stderr chunk into the canonical model transcript yet. First version should stream chunks to trace/UI-facing events only. Final tool result remains the thing fed back into the model.

### Cancellation

Cancellation is requested in core and executed by host.

Core may expose an action or API equivalent to:

```rust
AgentAction::CancelTools {
    tool_call_ids: Vec<ToolCallId>,
    reason: CancelReason,
}
```

Host performs the actual process termination:

```text
SIGTERM -> grace period -> process-tree SIGKILL
```

Core records/report cancellation as typed lifecycle state.

### Background Jobs

Background jobs are host-owned.

Core may see a typed reference:

```rust
pub struct BackgroundJobRef {
    pub job_id: String,
    pub tool_call_id: ToolCallId,
    pub command_label: String,
}
```

Do not rely on shell `$!` across tool calls. A long-running server should be tracked by the host runtime job table and stopped by job id.

## JS Runtime Target

Implement a local async tool runtime around the existing local tools.

Suggested files:

- `web/src/local/toolRuntime.ts`
- `web/src/local/jobTable.ts`
- `web/src/local/streamingBashTool.ts`
- `web/test/localToolRuntime.test.ts`

The runtime should support:

- async `startTool(call)` returning internal job/run handle
- stdout/stderr callbacks for bash
- completion callback that eventually calls `onToolDone`
- cancellation by tool call id
- background job table for long-running commands
- cleanup on host shutdown/test failure

The host loop should not block while a tool is running.

## WASM Boundary

Expose functions equivalent to:

```text
onToolStarted(handle, toolCallIdJson)
onToolUpdate(handle, updateJson)
onToolCancelled(handle, toolCallIdJson, reasonJson)
```

All input JSON must parse into typed Rust structs. Return the same envelope style as existing exports:

```json
{ "ok": true, "data": { "events": [], "actions": [] } }
```

Errors must be concrete and useful.

## Boundary Stress Tests

Push the runtime to its boundary. These tests are the point of this milestone.

### 1. Streaming Output

Run a command that prints slowly:

```bash
node -e "let i=0; const t=setInterval(() => { console.log('tick '+(++i)); if(i===5) clearInterval(t); }, 100)"
```

Assert:

- host receives at least 5 stdout updates before final done
- update sequence numbers are strictly increasing
- final result contains the full or bounded final output
- Rust emits `ToolExecutionUpdate` events
- host trace shows updates before `tool_done`

### 2. Stderr Streaming

Run:

```bash
node -e "console.error('warn-1'); console.error('warn-2')"
```

Assert:

- stderr chunks are tagged as `stderr`
- stderr does not get mislabeled as stdout
- final result still reports exit code correctly

### 3. Host Loop Responsiveness

Start a long command:

```bash
node -e "setTimeout(() => console.log('done'), 1000)"
```

While it runs, inject a steering/follow-up or host-level stop signal.

Assert:

- the host loop can process the second event before the tool finishes
- no synchronous blocking wait freezes the runtime

### 4. Cancellation

Start:

```bash
node -e "setInterval(() => console.log('still-running'), 100)"
```

Cancel after a few updates.

Assert:

- process exits before natural completion
- result is typed as cancelled/aborted
- no further stdout updates after cancellation settles
- no orphan process remains

### 5. Background Server Lifecycle

Start a server through the runtime, not shell `$!`:

```bash
python3 -m http.server 0
```

or a deterministic Node server that prints its selected port.

Assert:

- runtime creates a `job_id`
- job appears in job table
- curl/fetch can reach it while running
- stop by `job_id` works
- job disappears or marks stopped
- no server remains after cleanup

### 6. Parallel Tools

Start two independent slow commands.

Assert:

- both run concurrently
- updates interleave
- each tool call id keeps its own sequence
- both final results are correlated with the correct tool call id

### 7. Serialized File Mutation Still Holds

While bash runs concurrently, issue two writes/edits to the same file.

Assert:

- file mutations remain serialized per path
- no partial/corrupt file content
- unrelated bash streaming continues

### 8. Projection Boundary

Generate a large streaming bash output.

Assert:

- streaming chunks go to events/trace
- final tool result is still projected by Rust context projection if oversized
- partial chunks do not bloat canonical transcript

## Non-Goals

- Do not build UI.
- Do not implement browser remote execution.
- Do not persist background jobs across process restart.
- Do not put Node process logic in Rust.
- Do not feed partial stdout/stderr into the LLM mid-tool yet.

## Verification

Run:

```bash
cargo test --workspace
cd web && npm test
```

If a real smoke key exists, also run:

```bash
cd web && ANTHROPIC_API_KEY=... npm run smoke:real-local-coding
```

## Report Back

Report:

- changed files
- Rust lifecycle types/API added
- WASM export names and envelope shape
- JS runtime files added
- which boundary stress tests pass
- any intentionally deferred lifecycle behavior
- test results

Do not commit.
