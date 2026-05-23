# Milestone 7: Local Session and Artifact Persistence

Implement this after Milestone 6.5 local tool runtime control.

## Read First

- `CLAUDE.md`
- `ROADMAP.md`
- `LOCAL_TOOL_RUNTIME_SPEC.md`
- `CONTEXT_PROJECTION_SPEC.md`
- `MILESTONE_6_5_LOCAL_TOOL_RUNTIME.md`
- `web/src/providers/realLlm.ts`
- `web/src/context/rustProjection.ts`
- `web/src/local/toolRuntime.ts`
- `web/scripts/real-local-coding-smoke.ts`
- `pi-core/src/message.rs`
- `pi-core/src/context_projection.rs`
- `design.md`

## Goal

Persist local agent sessions and large artifacts so a local run can be inspected and resumed.

Current state:

- real local coding smoke works
- Rust context projection exists
- async `ToolRuntime` is wired into `RealAgentHost`
- bash streaming/tool lifecycle events are observable
- sessions and artifacts are still in-memory/ephemeral

Milestone 7 adds host-owned local persistence while keeping Rust core clean and runtime-neutral.

## Architecture Boundary

Rust core may define generic storage contracts/types, but it must not perform runtime I/O.

Acceptable Rust-side direction:

```rust
pub trait SessionStore {
    type Error;

    fn append(&mut self, entry: SessionEntry) -> Result<(), Self::Error>;
    fn load(&self, session_id: SessionId) -> Result<SessionSnapshot, Self::Error>;
}

pub trait ArtifactStore {
    type Error;

    fn put(&mut self, artifact: ArtifactRecord) -> Result<ArtifactRef, Self::Error>;
    fn get(&self, artifact_id: &str) -> Result<ArtifactRecord, Self::Error>;
}
```

But implementation belongs to host runtimes:

- local machine: filesystem JSONL + artifact files
- browser: IndexedDB or OPFS
- iOS: SQLite or native file storage
- Android: SQLite or app storage
- remote/cloud: service-backed storage

Do not add filesystem, IndexedDB, SQLite, or network assumptions to `pi-core`.

## Scope

### 1. Local Session Store

Add a JS/TS local session store.

Suggested files:

- `web/src/local/sessionStore.ts`
- `web/test/sessionStore.test.ts`

Preferred format: append-only JSONL.

Session metadata:

- `session_id`
- `cwd`
- `model`
- `created_at`
- `updated_at`

Entries:

- user prompt
- assistant message
- tool call
- tool result
- tool streaming update
- context projection report
- artifact reference
- lifecycle event

### 2. Filesystem Artifact Store

Add a filesystem-backed artifact store.

Suggested file:

- `web/src/local/fileArtifactStore.ts`

It should store full tool outputs by artifact ID emitted from Rust projection reports.

Artifact metadata:

- artifact id
- tool name
- tool call id
- content path
- created at
- byte length

### 3. Persistence Wiring

Wire persistence into `RealAgentHost`, `RealLlm`, or a small wrapper around them.

Requirements:

- record trace entries as session entries
- persist context projection reports from `RealLlm`
- persist artifact contents when Rust projection replaces a tool result
- keep existing in-memory artifact store usable for tests
- do not mutate canonical Rust transcript

### 4. Reload Support

Support loading a local session JSONL file.

Requirements:

- load session metadata
- load entries in order
- reconstruct enough messages to pass into `AgentOptions.messages`
- artifact references remain readable
- corrupt JSONL lines produce useful typed errors

### 5. Real Smoke Update

Update `web/scripts/real-local-coding-smoke.ts`:

- create a session directory inside the temp root
- persist the run
- after run, verify:
  - session file exists
  - entries include prompt, tool events, projection reports
  - artifacts directory exists if any projection replacement happened
  - session can be loaded back without throwing

## Tests

Add tests for:

- session store creates metadata
- session store appends entries without rewriting old entries
- loading session reconstructs entries in order
- corrupt JSONL returns useful typed error
- file artifact store writes and reads content by stable id
- artifact metadata includes tool name, tool call id, byte length
- persistence wrapper records tool updates before tool done
- context projection replacement writes full artifact content to disk
- reloaded session can provide `AgentOptions.messages`

## Non-Goals

- Do not implement compaction yet.
- Do not add browser storage yet.
- Do not add UI.
- Do not add cloud sync.
- Do not put runtime-specific storage in `pi-core`.
- Do not make the session format provider-specific unless unavoidable.

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
- session JSONL schema
- artifact directory layout
- reload behavior
- test results
- whether real network smoke was run or skipped
- any deferred persistence behavior

Do not commit.
