# Per-Turn Instructions Plan

This plan adds dynamic turn-scoped instructions without turning the session
system prompt into host-owned boilerplate.

The design is intentionally narrower than a generic prompt-update system:

- the agent keeps stable base/session instructions
- each user turn may provide a full replacement effective instruction string
- callers that want append/compose behavior read the current base instructions
  and build the replacement string themselves

This keeps Rust core deterministic and typed while giving SDK users the dynamic
per-turn control they need.

## Problem

`AgentOptions.system_prompt` currently initializes a session-level prompt that is
used for every LLM request. That is stable, but too rigid for workflows that need
temporary mode changes:

- review mode for one request
- planning mode for one request
- execution mode for one request
- browser/project-state-aware instructions
- temporary safety or permission framing

We do not want users to recreate the agent session just to change instructions
for one turn.

## Design Choice

Use **replacement-only turn instructions**.

Do not add core-level append semantics.

Append sounds convenient, but it makes core responsible for prompt composition,
separator policy, ordering, and later prompt-template behavior. The more
idiomatic API is to expose the current base instructions and let the caller
compose the full effective prompt when needed.

Example SDK append built by the caller:

```ts
const base = agent.getInstructions();

await agent.run("Review this patch", {
  instructions: `${base}

For this turn, act as a strict code reviewer.`,
});
```

Core receives only the final effective instruction string for that turn.

## Core Model

There are two instruction values:

```text
base instructions
optional active turn instructions
```

Effective instructions are:

```text
active turn instructions if present
otherwise base instructions
```

The active turn instructions are ephemeral and scoped to the current user turn.
They must remain active across tool loops in the same turn.

For example:

```text
user run starts with turn instructions
-> LLM call uses turn instructions
-> assistant asks for tools
-> host executes tools
-> continue_turn LLM call still uses the same turn instructions
-> turn settles
-> active turn instructions are cleared
```

This is the key behavioral requirement. Per-turn does not mean “first LLM call
only”; it means the whole agent turn, including post-tool continuation.

## Rust Types

Add a small domain type instead of passing raw strings deeper into core:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instructions(String);

impl Instructions {
    pub fn new(value: impl Into<String>) -> Result<Self, InstructionError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(InstructionError::Empty);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
```

Add turn options:

```rust
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnOptions {
    pub instructions: Option<Instructions>,
}
```

`None` means inherit the base/session instructions.

Add active turn state to `Agent`:

```rust
pub(crate) active_turn_instructions: Option<Instructions>,
```

`AgentOptions.system_prompt` can stay as the external constructor field for
compatibility, but internally it should be parsed into `Instructions` when data
crosses into Rust core. A later cleanup can rename the public field to
`instructions`.

## Core Semantics

At `start_turn`:

- accept `TurnOptions`
- set `active_turn_instructions = options.instructions`
- push the user message as today
- build `LlmContext` from effective instructions

At `continue_turn`:

- do not accept new instructions
- use the current effective instructions
- this preserves the same turn-level instructions across tool loops

When a turn reaches a terminal boundary:

- `Finished`
- `Aborted`
- settled waiting for new user input after the assistant responds without
  further tool work

clear `active_turn_instructions`.

This clearing should happen in core, not in SDK, so all hosts get the same
behavior.

## Effective Instruction Helper

Add one helper on `Agent`:

```rust
fn effective_instructions(&self) -> &Instructions {
    self.active_turn_instructions
        .as_ref()
        .unwrap_or(&self.state.system_prompt)
}
```

Then context construction uses that value:

```rust
build_llm_context_from_trimmed(
    t,
    self.effective_instructions().as_str(),
    &self.turn_tools,
)
```

## Public Read API

Expose the stable/base instructions so callers can compose a replacement.

Rust core:

```rust
impl AgentRuntime {
    pub fn instructions(&self) -> &Instructions;
}
```

WASM host:

```ts
getHostAgentInstructions(handle: number): StringResult
```

SDK:

```ts
agent.getInstructions(): string
```

This returns the base/session instructions, not the ephemeral active turn
replacement. That is the value users need for append-like composition.

## WASM API Shape

Extend `StartTurnInput`:

```ts
type StartTurnInput = {
  prompt: AgentMessage;
  tools: ToolDefinition[];
  instructions?: string;
};
```

Semantics:

- omitted: inherit base instructions
- string: use this full instruction string for the whole current turn

Validate at the Rust DTO boundary:

- reject empty or whitespace-only strings
- preserve actionable error messages

Do not add `{ type: "replace" }` unless we later need more modes. Optional
string already means replacement.

## SDK API Shape

Add replacement-only run option:

```ts
export interface AgentRunOptions {
  signal?: AbortSignal;
  metadata?: Record<string, unknown>;
  instructions?: string;
}
```

Usage:

```ts
await agent.run("Review this file", {
  instructions: "You are a strict code reviewer. Focus on bugs and regressions.",
});
```

Append-like usage:

```ts
const base = agent.getInstructions();

await agent.run("Review this file", {
  instructions: `${base}

For this turn, focus only on security issues.`,
});
```

No separate `turnInstructions`, `systemPrompt`, or update enum in the SDK for
the first implementation.

## Persistence

Base instructions remain persisted as part of host/agent state.

Active turn instructions are ephemeral.

Do not persist active turn instructions for completed turns.

If we later need exact resume of an in-flight LLM request, persist the pending
`LlmContext` or active action state, not a mutation to base instructions.

## Tests

Core tests:

- default turn uses base instructions
- turn replacement uses replacement instructions
- replacement does not mutate base instructions
- post-tool `continue_turn` uses the same active replacement instructions
- next user turn without replacement returns to base instructions
- empty replacement instructions are rejected at the typed boundary

WASM tests:

- `StartTurnInput` without `instructions` preserves current behavior
- `StartTurnInput.instructions` reaches emitted `StreamLlm.context.system_prompt`
- `hostContinueTurn` after tools keeps the same replacement instructions
- `getHostAgentInstructions` returns base instructions
- empty or whitespace-only instructions return a typed error

SDK tests:

- `agent.run(..., { instructions })` reaches `ModelRequest.instructions`
- `agent.getInstructions()` returns config/restored base instructions
- caller-composed append works by reading `getInstructions()`
- consecutive runs can use different replacements without recreating the agent
- a run without replacement after a replacement uses the base instructions

## Non-Goals

Do not implement these in the first pass:

- append mode in core
- prompt template registry
- persistent mutation of base instructions
- provider-specific prompt formatting
- automatic prompt selection by model
- arbitrary context transform callbacks
- full `PrepareNextTurn` model/tool/budget update

## Implementation Order

1. Add `Instructions` and `TurnOptions` to `pi-core`.
2. Parse `AgentOptions.system_prompt` into `Instructions` at the Rust boundary.
3. Add `active_turn_instructions` to `Agent`.
4. Thread `TurnOptions` through typestate `start_turn`.
5. Update context construction to use `effective_instructions()`.
6. Clear active turn instructions at core turn terminal boundaries.
7. Add `AgentRuntime::instructions()`.
8. Extend WASM `StartTurnInput` with optional `instructions`.
9. Add WASM `getHostAgentInstructions`.
10. Add SDK `AgentRunOptions.instructions`.
11. Add SDK `agent.getInstructions()`.
12. Add core, WASM, and SDK tests.

## Success Criteria

- Callers can set dynamic per-turn instructions without recreating the agent.
- Core owns the active turn scope, including post-tool continuation.
- Base/session instructions remain stable and readable.
- Append behavior is possible through caller-side composition.
- The public API stays small: optional replacement string, no prompt-update enum.
