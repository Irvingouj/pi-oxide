# Context Projection Engine Design

## Goal

Per-tool-result context projection with:
- **Deferred execution** — new results stay full until old enough
- **One-time projection** — each result is projected once, then cached
- **Scriptable strategies** — Rhai scripts decide when/how to project
- **LLM-friendly prompts** — scripts can say "let LLM handle this"

## Core Types

### ProjectionStrategy

Meta-enum choosing between fixed and dynamic strategies.

```rust
pub enum ProjectionStrategy {
    Fixed {
        strategy: FixedStrategy,
        old_after: u32, // 0 = immediate, 5 = project after 5 turns
    },
    Dynamic {
        script: String,
    },
}
```

### FixedStrategy

```rust
pub enum FixedStrategy {
    KeepFull,
    Head { max_chars: usize },
    Tail { max_chars: usize },
    HeadTail { head_chars: usize, tail_chars: usize },
    DropIfOld,
    Microcompacted,
}
```

### ProjectionOutcome

```rust
pub enum ProjectionOutcome {
    Projected { text: String },
    Deferred { reevaluate_after: u32 },
    Prompted { text: String },
}
```

## State

```rust
pub struct ContextProjectionState {
    pub replacements: BTreeMap<String, ContextReplacement>,
    pub deferred: BTreeMap<String, DeferredState>,
    pub last_api_usage: Option<ApiUsageSnapshot>,
    pub turns_since_compaction: u32,
}

pub struct ContextReplacement {
    pub tool_call_id: String,
    pub tool_name: String,
    pub artifact_id: String,
    pub original_chars: usize,
    pub preview_chars: usize,
    pub strategy: ProjectionStrategy,
    pub outcome: ProjectionOutcome,
}

pub struct DeferredState {
    pub tool_call_id: String,
    pub reevaluate_at_turn: u32,
}
```

## Execution Flow

### 1. Check cached replacements

If `tool_call_id` in `state.replacements`:
- `Projected { text }` → build preview with cached text
- `Prompted { text }` → build preview with cached text
- `Deferred` → never cached

### 2. Check deferred state

If `tool_call_id` in `state.deferred`:
- If `current_turn < reevaluate_at_turn` → keep original text, skip
- If `current_turn >= reevaluate_at_turn` → remove from `deferred`, continue

### 3. Execute strategy

#### Fixed strategy

```rust
let age = total_turns - turn_index;
if age < old_after {
    Deferred { reevaluate_after: old_after - age }
} else {
    apply_fixed_strategy(text, strategy) // returns Projected
}
```

#### Dynamic script

Rhai script receives:
- `text`, `tool_name`, `tool_call_id`
- `turn_index`, `total_turns`
- `total_tokens`, `max_context_tokens`
- `max_tool_result_chars`, `turns_since_compaction`
- `was_replaced_before`

Built-in functions:
- `head(text, n)` → first N chars
- `tail(text, n)` → last N chars
- `lines(text)` → line array
- `join(lines, sep)` → joined string
- `contains(text, pattern)` → bool
- `length(text)` → char count

Script returns a Map:

```rhai
// Project — replace with projection text
#{
    action: "project",
    text: head(text, 2000)
}

// Defer — keep original, re-evaluate after N turns
#{
    action: "defer",
    reevaluate_after: 3
}

// Prompt — replace with LLM-facing prompt
#{
    action: "prompt",
    text: "This is a 50K log file. Ask me to filter if needed."
}
```

### 4. Cache and return

- `Projected` / `Prompted` → cache to `state.replacements`, build preview
- `Deferred` → cache to `state.deferred`, keep original text

## Semantics

| Action | Meaning | Cache | Text |
|--------|---------|-------|------|
| `project` | Script decided, here is the text | `replacements` | projection text |
| `defer` | Not yet, ask again later | `deferred` | original text |
| `prompt` | Replace with LLM prompt | `replacements` | prompt text |

`keep_full` is `project` returning original text.

## Examples

### SDK user: fixed strategy with delay

```javascript
details: {
    strategy: {
        type: "fixed",
        strategy: { type: "head", max_chars: 2000 },
        old_after: 5
    },
    original_chars: 8000,
    truncated_by_tool: false
}
```

- Turns 1~4: `Deferred { reevaluate_after: 5 - age }`, full text
- Turn 5: `Projected { text: head(text, 2000) }`, cached
- Turn 6~N: direct cache reuse

### SDK user: dynamic script

```javascript
details: {
    strategy: {
        type: "dynamic",
        script: `
            if total_turns - turn_index < 3 {
                #{ action: "defer", reevaluate_after: 3 - (total_turns - turn_index) }
            } else if total_tokens > max_context_tokens * 8 / 10 {
                #{ action: "project", text: head(text, 500) }
            } else {
                #{ action: "project", text: text }
            }
        `
    },
    original_chars: 8000,
    truncated_by_tool: false
}
```

- Turn 1: `Deferred { reevaluate_after: 3 }`, full text
- Turn 2~3: deferred, skip script execution
- Turn 4: script re-runs, `Projected { text: text }`, cached
- Turn 5~N: direct cache reuse

### SDK user: prompt

```javascript
details: {
    strategy: {
        type: "dynamic",
        script: `
            if length(text) > 50000 {
                #{ action: "prompt", text: "This is a 50K log file. Ask me to filter if needed." }
            } else {
                #{ action: "project", text: text }
            }
        `
    },
    original_chars: 50000,
    truncated_by_tool: false
}
```

## Engine Limits

- `max_operations: 100_000` — caps AST operations
- `max_string_size: 100_000` — caps string length
- Engine cached per thread via `thread_local!`

## Fallback

If script errors or returns invalid Map:
- Log warning
- Fallback to `head(text, 2000)`
- Cache as `Projected`

## Comments

The overall direction is right. This design solves the main failure mode we saw in Browsergent: a fresh `run_lua` result was hard-truncated before the agent could use it. New tool results should stay full, and projection should happen only when the result is old enough or the context budget actually requires it.

Key strengths:

- Deferred execution is important because the newest tool result is usually the most relevant context for the next action.
- One-time projection plus cached replacement avoids context drift across turns.
- `Prompted` is more agent-native than blind truncation. A prompt like "this result is large; ask to filter or read the artifact" gives the model a useful next move.
- Per-tool strategies are necessary. Browser snapshots, bash logs, source files, grep output, docs, CSV/JSON extraction, and screenshots should not share one truncation rule.

Concerns:

- Rhai-first may be too much surface area for the first implementation. It adds script safety, runtime limits, debugging complexity, and versioning concerns. A small set of built-in policies should exist first, with Rhai as an escape hatch.
- `artifact_id` is present, but the design needs an explicit artifact read/search path. If the full result is replaced, the agent needs tools such as `read_artifact(id, offset, limit)` and `search_artifact(id, query)`. Otherwise artifact metadata is not recoverability.
- `keep_full` as "`project` returning original text" is semantically muddy. Prefer an explicit `ProjectionOutcome::KeepFull` so "kept full" is distinct from "projected".
- Fallback to `head(text, 2000)` risks becoming silent truncation again. Safer fallback should be `Prompted` with an artifact reference, e.g. "projection failed; full result is stored as artifact X".
- The age calculation needs precise semantics. Define whether a result produced in turn N has age 0 in turn N and age 1 in turn N+1, so `old_after` cannot drift off by one.

Recommended default policy shape:

```text
run_lua / browser snapshot:
  keep full for 2-3 turns
  if still large after that, prompt + artifact

get_doc:
  keep full if namespace-filtered
  summarize/index if full docs are huge

bash/log output:
  tail after delay

read file:
  head after delay

grep/find/list output:
  keep full if small, head if huge

structured extraction / CSV / JSON:
  keep full longer, then artifact-backed prompt
```

Bottom line: this should be a smart projection and artifact recovery layer, not a more configurable truncation layer. The full data must remain recoverable after projection.
