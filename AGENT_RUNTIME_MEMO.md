# Agent Runtime Memo

Working memo for the next architecture discussion. This is not the roadmap yet.

## Goal

The immediate product goal should be a minimal functional coding agent on a normal computer.

After that works, the browser agent becomes a host variant of the same model:

```text
Rust core state machine
-> host runtime loop
-> local/browser tools
-> context preparation
-> provider call
-> results fed back into Rust
```

The near-term browser target is still valid, but a real coding agent needs the normal-computer host first because the core workflow depends on real filesystem and command execution:

- read files
- write files
- apply targeted edits
- run bash commands
- manage long conversation/tool context

## Current pi-oxide State

Already done:

- Rust core owns the synchronous agent state machine.
- WASM wrapper exposes typed agent operations.
- JS host loop can drive fake and real LLM providers.
- pi-compatible in-memory tools exist: `read`, `write`, `edit`, `bash`, `grep`, `find`, `ls`.
- Real LLM smoke script uses `PI_CODING_TOOLS`.

Still missing:

- A full real-LLM local coding-agent smoke that uses the real local tools end to end.
- Rust-owned context management before provider calls.
- Typed tool-result metadata for smart projection strategies.
- Persistent sessions/artifacts for tool outputs and compaction.

## What PI Does

Reference root: `../pi/packages/coding-agent/src/core`.

### Tool Surface

PI's default coding tools are defined in `tools/index.ts`:

- default coding set: `read`, `bash`, `edit`, `write`
- read-only set: `read`, `grep`, `find`, `ls`
- all tools: `read`, `bash`, `edit`, `write`, `grep`, `find`, `ls`

This matches the direction already started in `pi-oxide`.

### Tool Design Pattern

Each PI tool is host-owned and operation-injected:

- `read.ts` defines `ReadOperations` with `readFile`, `access`, optional image MIME detection.
- `write.ts` defines `WriteOperations` with `writeFile`, `mkdir`.
- `edit.ts` defines `EditOperations` with `readFile`, `writeFile`, `access`.
- `bash.ts` defines `BashOperations` with `exec(command, cwd, { onData, signal, timeout, env })`.

This is the exact shape we want: core logic can stay runtime-neutral while the host provides local machine behavior.

### Read

PI `read`:

- accepts `path`, `offset`, `limit`
- resolves path relative to cwd
- supports text and images
- truncates text output using shared limits
- recommends offset/limit continuation instead of dumping giant files

Important policy:

- file reads keep the head of content
- large reads are bounded by line and byte limits
- truncation details travel with the result

### Write

PI `write`:

- accepts `path`, `content`
- creates parent directories
- overwrites existing file
- uses a file mutation queue per path

Important policy:

- write is for new files or complete rewrites
- concurrent file mutations must serialize

### Edit

PI `edit`:

- accepts `path`, `edits: [{ oldText, newText }]`
- validates non-empty edits
- matches against original file, not incrementally
- rejects overlapping/nested edits
- preserves line endings/BOM behavior
- returns diff details
- uses a file mutation queue

Important policy:

- targeted edit should be the normal modify path
- write should not be the default for small changes
- diff details are useful for UI, logs, and permission review

### Bash

PI `bash`:

- accepts `command`, optional `timeout`
- shell execution is injected through `BashOperations`
- streams stdout/stderr through `onData`
- supports abort and process-tree kill
- strips/sanitizes output
- truncates tail of output because command failures are usually at the end
- can persist full output to a temp file when large

Important policy:

- bash result should include output, exit code, cancellation, truncation, full output path if any
- streaming matters for UX, but a first local host can start with buffered execution

### Truncation

PI shared truncate limits:

- default max lines: 2000
- default max bytes: 50KB
- file read uses head truncation
- bash uses tail truncation
- grep line length is capped

This is a good first policy for `pi-oxide`.

### Session and Compaction

PI session storage is an append-only tree:

- message entries
- model changes
- thinking level changes
- compaction entries
- branch summaries
- custom entries

Compaction:

- estimates tokens by chars / 4
- uses real assistant usage when available
- triggers when context tokens exceed `contextWindow - reserveTokens`
- default reserve tokens: 16384
- default recent keep budget: 20000
- finds cut points without splitting tool result pairs incorrectly
- summarizes old conversation into a structured summary
- tracks read/modified files across compaction details

Useful lesson:

We do not need full branch tree or LLM summarization on day one, but we should design the local host around append-only session entries and provider projection.

## What Claude Code Does

Reference root: `~/code/claude-code/src`.

Claude Code has a heavier but battle-tested per-turn context pipeline.

### Per-Turn Pipeline

The main loop in `query.ts` does not send raw history directly to the API. It prepares a provider projection every turn:

1. slice after compact boundary
2. apply tool result budget
3. snip old messages if enabled
4. microcompact tool results
5. context collapse if enabled
6. auto-compact if near window
7. normalize messages for API
8. prepend user context
9. append system context
10. assemble stable tool pool
11. call model
12. execute tools
13. add attachments and persist new state

For `pi-oxide`, the key idea is the projection layer, not all feature flags.

### Tool Result Budget

Claude Code's `utils/toolResultStorage.ts` handles large tool results:

- empty tool results become explicit text like `(tool completed with no output)`
- large text tool results are persisted to disk under a session directory
- the context receives a preview plus a full-output path
- replacement decisions are cached in `ContentReplacementState`
- once the model has seen a tool result one way, future turns preserve that same representation for prompt-cache stability

Useful lesson:

Tool result context management must be deterministic. Do not randomly truncate differently every turn.

### Message Normalization

Claude Code's `normalizeMessagesForAPI`:

- removes display-only virtual/progress/system messages
- merges consecutive user messages
- merges assistant fragments by message id
- strips invalid or unavailable tool references
- sanitizes problematic media/tool-result cases
- validates images before the provider request

Useful lesson:

Provider adapters should not receive arbitrary internal transcript. They should receive normalized provider-ready messages.

### Tool Pool

Claude Code has a central `tools.ts`:

- one source of truth for built-in tools
- filters by permission/deny rules before model sees tools
- merges MCP tools with built-ins
- keeps order stable for prompt caching

Useful lesson:

Even minimal local host should have a single tool registry boundary with permission filtering before provider call.

## What We Should Build First

Do not copy Claude Code's full system.

Build a smaller PI-like local host with a Claude-Code-like projection layer.

### Milestone A: Local Machine Host

Add a Node/local host package under `web` or rename toward `host-js` later.

Functional target:

```text
user prompt
-> Rust agent emits StreamLlm
-> JS host calls Anthropic
-> model calls read/edit/bash
-> JS host executes tools on real cwd
-> Rust receives tool results
-> model continues
-> task finishes
```

Tool implementation:

- `read`: real filesystem, cwd-confined by default, offset/limit, head truncation
- `write`: real filesystem, cwd-confined, creates parents, serialized per path
- `edit`: real filesystem, exact replacement, rejects missing/ambiguous/overlapping edits, returns diff
- `bash`: real shell, cwd-confined, timeout, abort, output tail truncation, no destructive auto-allow by default

Current local `bash` is only smoke-test grade. The runtime model still needs:

- streaming stdout/stderr so the host/UI can observe partial output
- explicit background process lifecycle instead of shell-owned orphan processes
- signal/abort support beyond timeout termination
- async tool execution so the host loop remains responsive while tools run

Safety:

- default deny writes outside cwd
- default deny bash until permission mode is explicit
- allow a simple non-interactive mode for tests
- log every tool call with args and result summary

### Milestone B: Rust Context Projection Layer

Add Rust-side context preparation before provider calls.

Initial modules:

- `pi-core/src/context_projection.rs`
- `pi-core/src/context_strategy.rs`
- `pi-core/src/context_metadata.rs`
- `pi-host-web` projection export
- JS wrapper only for calling WASM and storing artifacts

Behavior:

- estimate tokens by chars / 4
- keep canonical Rust transcript unchanged
- produce provider projection from `LlmContext`
- replace oversized tool results with deterministic previews
- return stable artifact IDs and replacement reports
- preserve exact replacement decisions across turns
- leave provider-specific message merging to the provider adapter

First artifact store:

- local filesystem for normal-computer host
- in-memory for tests
- later IndexedDB/OPFS for browser

The artifact store remains host-owned because it is runtime I/O. The decision to replace, keep, head-trim, tail-trim, or drop belongs in Rust because it is portable agent behavior.

### Milestone B.5: Local Tool Runtime Control

Before treating the local host as production-grade, build a proper tool runtime.

Runtime target:

- `bash` streams stdout/stderr as updates.
- background processes are tracked as jobs with IDs.
- jobs can be stopped explicitly and cleaned up on shutdown.
- user stop can abort in-flight tool execution.
- timeout first attempts graceful termination, then escalates.
- tool execution is async in the host; Rust core stays synchronous and receives updates/results.

This is host runtime work. It must not introduce Node, shell, process, or signal assumptions into `pi-core`.

### Milestone C: Minimal Session Persistence

Add append-only local session storage.

First version:

- session metadata: cwd, model, created time
- entries: message, tool result artifact, compaction summary later
- reload session into `AgentOptions.messages`
- no branch tree yet

### Milestone D: Manual Compaction

Only after local agent works.

First version:

- `/compact` or host command
- summarize older transcript into a user-visible summary message
- keep last N estimated tokens
- preserve recent tool calls/results

No proactive auto-compact until manual compaction is reliable.

## Proposed Architecture Boundary

Rust core should own:

- typed messages
- state machine phase
- action/event protocol
- tool call/result correlation
- context projection policy
- typed tool-result metadata
- deterministic token estimation and replacement reports
- session/storage traits only if runtime-neutral

JS local host should own:

- filesystem
- shell
- permission policy
- provider calls
- artifact storage
- session persistence

Browser host should later swap:

- real filesystem -> File System Access API / OPFS / virtual workspace
- shell -> remote runner / sandbox / no bash unless backed by service
- artifact store -> IndexedDB/OPFS

## Main Design Decision Needed

We need to choose the next implementation path:

1. Build a new `local` JS host next to `web`, using the existing WASM/Rust interface.
2. Keep it under `web/src` for speed, but name modules `local*`.
3. Create a Rust native host instead of JS for local machine.

Recommendation: use JS local host first, but move portable context projection into Rust.

Reason:

- current provider/tool host loop is already JS
- Anthropic adapter is JS
- filesystem/bash tooling is straightforward in Node
- browser host can share most provider/context logic later
- Rust core stays clean
- context policy remains portable across local and browser hosts

## Minimal Acceptance Test

The first real functional-agent test should run on a normal computer:

```text
fixture project:
  package.json
  src/index.ts with add(a, b) returning a - b
  test file or bash test command

agent prompt:
  Fix the add function and run tests.

required trace:
  read src/index.ts
  edit src/index.ts or write src/index.ts
  bash npm test
  finished

required final state:
  src/index.ts contains return a + b
  bash result reports tests passing
```

This is the point where we can honestly say we have a minimal functional coding agent on the machine.

## Roadmap Implication

The current roadmap should be adjusted from:

```text
browser-first JS driven agent
```

to:

```text
Rust core + JS host architecture
1. functional local-machine coding agent
2. Rust-projected, context-managed local sessions
3. browser host using the same protocol and projection ideas
```

The browser remains the product direction, but local-machine functionality is the proving ground for the agent loop.
