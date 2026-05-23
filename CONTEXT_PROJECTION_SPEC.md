# Context Projection Technical Spec

## Purpose

Context projection turns the canonical agent transcript into a bounded provider-neutral transcript for one model call.

The canonical transcript remains true history. Projection is a read-time view.

```text
canonical messages + tools + system prompt + projection state
-> Rust projection engine
-> projected messages + updated projection state + projection report
-> JS provider adapter
```

## Ownership

Rust owns:

- typed message/context contracts
- typed tool-result metadata
- token estimation policy
- tool-result preview strategy
- recent-window trimming
- deterministic replacement state
- projection reports

Host owns:

- filesystem, shell, browser APIs, HTTP, UI
- provider-specific request conversion
- full artifact storage and retrieval
- permission prompts and runtime policy
- session persistence

## Invariants

- Projection must not mutate canonical messages.
- Projection must be deterministic for the same input, budget, and state.
- Tool-call/tool-result pairs must not be split.
- All metadata crossing into Rust must parse into concrete structs.
- Provider-specific terms such as Anthropic `tool_use`, `tool_result`, and cache controls must stay outside Rust projection.
- Core must not read or write artifact files.

## Tool Metadata

Tool results should carry enough typed metadata for Rust to choose a projection strategy without guessing from raw text.

Minimum metadata:

```rust
pub enum ContentKind {
    FileRead,
    CommandOutput,
    Diff,
    SearchResults,
    DirectoryListing,
    GenericText,
}

pub enum ContextStrategy {
    KeepFull,
    Head { max_chars: usize },
    Tail { max_chars: usize },
    HeadTail { head_chars: usize, tail_chars: usize },
    DropIfOld,
}

pub struct ToolResultContext {
    pub content_kind: ContentKind,
    pub strategy: ContextStrategy,
    pub original_chars: usize,
    pub truncated_by_tool: bool,
    pub path: Option<String>,
    pub exit_code: Option<i32>,
}
```

Fallback strategy when metadata is missing:

| Tool | Strategy |
|---|---|
| `read` | head |
| `bash` | tail |
| `edit` | keep full |
| `write` | keep full |
| `grep` | head |
| `find` | head |
| `ls` | head |
| unknown | head |

## Projection State

Projection state exists so the model sees stable context across turns.

```rust
pub struct ContextProjectionState {
    pub replacements: BTreeMap<String, ContextReplacement>,
}
```

Key by `tool_call_id`.

State rules:

- once replaced, reuse the same artifact ID and strategy
- once kept inline under the same budget, keep inline
- no random IDs
- no wall-clock timestamps in projected text

## Replacement Report

Projection returns a report for observability and host artifact storage.

```rust
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

Host can use the report to store the original full output under the emitted artifact ID.

## Token Estimation

First version uses deterministic rough estimation:

- count text chars in user, assistant, and tool-result messages
- count assistant tool call name and serialized arguments
- count system prompt text
- estimate tokens as `(chars + 3) / 4`

Real provider usage may be added later, but this first version should be simple and stable.

## Preview Text

Preview markers are provider-neutral:

```text
<context-artifact id="tool-result-..." tool="bash">
Tool result was too large and was replaced with a preview.
Full content should be available from host artifact: tool-result-...
Strategy: tail
Preview:
...
</context-artifact>
```

The marker is intentionally plain text because providers understand it without special APIs.

## Trimming

When projected context still exceeds `max_context_tokens`, drop whole old turns from the front.

Rules:

- preserve ordering
- keep the newest user message if possible
- never keep a `tool_result` without its matching assistant tool call
- report how many messages were dropped

First implementation may use a simple turn approximation:

```text
user -> assistant -> following tool_result messages
```

## WASM API

Suggested exported function:

```text
projectContext(inputJson) -> envelope
```

Envelope:

```json
{ "ok": true, "data": { "context": {}, "state": {}, "report": {} } }
```

Errors:

```json
{ "ok": false, "error": { "code": "invalid_json", "message": "..." } }
```

Use `thiserror` for Rust error variants and keep error codes stable.

## JS Integration

`RealLlm` should call the WASM projection before `callAnthropic()`.

JS may still provide:

- in-memory or filesystem artifact store
- compatibility wrapper around the WASM projection
- provider-specific normalization in `anthropic.ts`

JS should not remain the owner of projection policy after Milestone 5.
