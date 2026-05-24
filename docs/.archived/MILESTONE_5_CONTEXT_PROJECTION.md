# Milestone 5: Rust Context Projection Engine

Implement this milestone next.

## Read First

- `CLAUDE.md`
- `ROADMAP.md`
- `AGENT_RUNTIME_MEMO.md`
- `CONTEXT_PROJECTION_SPEC.md`
- `pi-core/src/message.rs`
- `pi-core/src/context.rs`
- `pi-core/src/tool.rs`
- `pi-host-web/src/lib.rs`
- `web/src/providers/realLlm.ts`
- `web/src/providers/anthropic.ts`
- `web/src/providers/types.ts`
- future wrapper file, if needed: `web/src/context/rustProjection.ts`

## Goal

Move context-management policy into Rust as a runtime-neutral projection engine.

The host still owns model calls, files, shell, artifact storage, and provider formatting. Rust owns the deterministic decision about what context is sent to the model.

```text
canonical Rust LlmContext
-> Rust project_context()
-> provider-neutral projected LlmContext + projection report
-> JS provider adapter
-> Anthropic request
```

This milestone is not about LLM summarization. It is about typed metadata, deterministic tool-result budgeting, and safe recent-window trimming.

## Why Rust

Tools are effectively text in/text out for the model, but the text has meaning:

- `read` output should keep the head.
- `bash` output should keep the tail.
- `edit` output should preserve the diff.
- `grep`/`find`/`ls` should keep bounded result previews.

That policy is portable agent behavior, not a browser or Node implementation detail. Therefore it belongs in Rust. The host may store full artifacts later, but Rust should emit stable artifact IDs and replacement reports.

## Constraints

- Keep `pi-core` runtime-neutral.
- No Tokio, filesystem, shell, browser API, HTTP, or provider assumptions in `pi-core`.
- Do not add UI.
- Do not implement summarizer compaction.
- Do not run real network smoke unless explicitly asked.
- Do not commit.
- Preserve existing JS context projection tests while migrating the policy boundary.
- Do not restore the old JS-side projection policy. JS may only add a thin wrapper that calls the Rust/WASM projection API.

## Required Rust Concepts

Add concrete typed strategy/metadata types. Exact names may vary, but the domain should be equivalent to:

```rust
pub enum ContextStrategy {
    KeepFull,
    Head { max_chars: usize },
    Tail { max_chars: usize },
    HeadTail { head_chars: usize, tail_chars: usize },
    DropIfOld,
}

pub enum ContentKind {
    FileRead,
    CommandOutput,
    Diff,
    SearchResults,
    DirectoryListing,
    GenericText,
}

pub struct ToolResultContext {
    pub content_kind: ContentKind,
    pub strategy: ContextStrategy,
    pub original_chars: usize,
    pub truncated_by_tool: bool,
    pub path: Option<String>,
    pub exit_code: Option<i32>,
}

pub struct ContextProjectionBudget {
    pub max_tool_result_chars: usize,
    pub max_context_tokens: usize,
    pub default_preview_chars: usize,
}

pub struct ContextReplacement {
    pub tool_call_id: String,
    pub tool_name: String,
    pub artifact_id: String,
    pub original_chars: usize,
    pub preview_chars: usize,
    pub strategy: ContextStrategy,
}

pub struct ContextProjectionReport {
    pub estimated_tokens: usize,
    pub replacements: Vec<ContextReplacement>,
    pub dropped_messages: usize,
}
```

Important: do not leave this as arbitrary JSON inside core logic. Parse metadata at the Rust boundary into concrete structs.

## Required Behavior

### 1. Token Estimate

Add deterministic token estimation in Rust.

Rules:

- text chars estimate as `(chars + 3) / 4`
- assistant tool call names and serialized arguments count toward estimate
- tool result text counts toward estimate
- system prompt counts toward estimate in the final projection estimate

### 2. Strategy Selection

Projection should use typed metadata when present.

Fallback by tool name when metadata is missing:

- `read` -> `Head`
- `bash` -> `Tail`
- `edit` -> `KeepFull`
- `write` -> `KeepFull`
- `grep` -> `Head`
- `find` -> `Head`
- `ls` -> `Head`
- unknown -> `Head`

### 3. Tool Result Budget

Only project tool-result text. Do not mutate the canonical transcript.

Behavior:

- small tool results stay inline
- oversized tool results are replaced with deterministic preview text
- artifact IDs must be stable, e.g. `tool-result-{tool_call_id}`
- preserve `tool_call_id`, `tool_name`, `is_error`, timestamp, and typed details
- projection reports what was replaced so the host can store the full content

Preview marker should be explicit and provider-neutral:

```text
<context-artifact id="tool-result-..." tool="bash">
Tool result was too large and was replaced with a preview.
Full content should be available from host artifact: tool-result-...
Strategy: tail
Preview:
...
</context-artifact>
```

### 4. Replacement State

Projection decisions must be stable across turns.

Add a projection state object in Rust or a typed boundary payload equivalent to:

```rust
pub struct ContextProjectionState {
    pub replacements: BTreeMap<String, ContextReplacement>,
}
```

Use `tool_call_id` as the stable key.

Behavior:

- if a tool result was already replaced, reuse the same replacement metadata and preview strategy
- if a tool result was previously kept inline, do not randomly flip it later under the same budget
- repeated projection with same input/state must be byte-identical

### 5. Recent Window Trimming

After tool-result budgeting, trim old history if estimated tokens exceed `max_context_tokens`.

Rules:

- drop whole old turns from the front
- never leave a `tool_result` whose assistant `tool_call` is missing
- keep newest user message if possible
- preserve message ordering
- return dropped count in `ContextProjectionReport`

First implementation may approximate a turn as:

```text
user -> assistant -> following tool_result messages
```

### 6. WASM Boundary

Expose projection through `pi-host-web` with JSON envelopes.

Suggested export:

```text
projectContext(inputJson) -> { ok: true, data: { context, state, report } }
```

Input must parse into typed Rust structs. Errors should use concrete `thiserror` variants and stable codes.

### 7. JS Wiring

Update `web/src/providers/realLlm.ts` so `RealLlm` uses the Rust projection path before provider conversion.

If JS context modules are added, they must be thin wrappers over WASM. Do not reintroduce JS-owned projection policy.

## Tests

Add or update Rust tests for:

- token estimate counts user text
- token estimate counts assistant tool call arguments
- token estimate counts tool result text
- `read` uses head preview
- `bash` uses tail preview
- `edit` defaults to keep-full
- metadata strategy overrides tool-name fallback
- replacement IDs are deterministic
- repeated projection with same state is byte-identical
- canonical input transcript is not mutated
- trimming drops old messages when over budget
- trimming does not leave orphan tool results
- parse errors return useful typed errors

Add or update JS tests for:

- WASM projection export succeeds
- WASM projection export returns error envelope for invalid input
- `RealLlm` can call through Rust projection without network
- existing context projection behavior remains covered at the public boundary

## Verification

Run:

```bash
cargo test --workspace
cd web && npm test
```

Expected:

- Rust tests pass.
- JS tests pass.
- `pi-core` remains runtime-neutral.
- JS provider adapter still owns Anthropic-specific formatting.

## Report Back

Report:

- changed files
- test results
- exact Rust types added
- WASM export name and envelope shape
- whether JS projection modules are now wrappers or still interim compatibility code

Do not commit.
