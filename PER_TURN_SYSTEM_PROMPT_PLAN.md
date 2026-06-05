# Per-Turn System Prompt Plan

This plan adds a runtime-neutral extension point for changing the system prompt
per turn instead of treating it as session-wide immutable state.

The goal is to support dynamic agent behavior while preserving `pi-oxide`'s
cross-platform architecture: Rust core owns typed state transitions and context
construction; hosts provide typed updates without core holding runtime
callbacks.

## Problem

`AgentOptions` currently initializes a session-level `system_prompt`. That is
simple, but too rigid for an extensible agentic core.

Real agents often need turn-specific instructions:

- tool- or workflow-specific mode switches
- different prompt framing after compaction
- browser state or project state injected as policy
- model-specific prompt adaptation
- temporary safety or permission instructions
- task phase changes, such as planning, editing, reviewing, or summarizing

This should not require rebuilding the agent session or mutating global session
identity.

## Core Model

Separate the base session prompt from the effective prompt used for an LLM
request.

```text
base system prompt
+ optional per-turn system prompt override or append
-> effective system prompt for this LLM context
```

The base prompt remains part of agent state. Per-turn prompt state should be
ephemeral unless explicitly persisted by the host.

## Proposed Rust Types

Add a typed prompt update:

```rust
pub enum SystemPromptUpdate {
    Keep,
    Replace { system_prompt: String },
    Append { text: String },
}
```

For next-turn preparation, use:

```rust
pub struct NextTurnUpdate {
    pub system_prompt: Option<SystemPromptUpdate>,
    // future fields: model, thinking_level, tools, context_budget, projection_policy
}
```

For the current turn start path, allow a prompt update in the turn request:

```rust
pub struct StartTurnInput {
    pub prompt: AgentMessage,
    pub tools: Vec<ToolDefinition>,
    pub system_prompt: Option<SystemPromptUpdate>,
}
```

If the existing API shape should stay stable, add a parallel method first:

```rust
start_turn_with_options(...)
```

and keep `start_turn(...)` as a convenience wrapper that uses
`SystemPromptUpdate::Keep`.

## Semantics

`Keep`

- Use the base session system prompt unchanged.

`Replace`

- Use the provided prompt as the effective prompt for this turn.
- Does not mutate the base session prompt unless a later explicit persistent
  update API is added.

`Append`

- Build the effective prompt by appending text to the base prompt.
- Use a deterministic separator, for example:

```text
{base_system_prompt}

{per_turn_append}
```

## State and Persistence

First implementation should treat per-turn prompt updates as ephemeral.

Do not persist per-turn prompt update in the session snapshot unless it is
needed to resume an in-flight LLM request. For completed turns, the transcript
and resulting messages are enough.

If resumability requires exact replay of an in-flight context, persist the
effective `LlmContext` or the pending `AgentAction::StreamLlm` rather than
mutating the base prompt.

## Context Construction

Change context building from:

```rust
build_llm_context_from_trimmed(t, &self.state.system_prompt, &self.turn_tools)
```

to something that accepts an effective prompt:

```rust
build_llm_context_from_trimmed(t, effective_system_prompt, &self.turn_tools)
```

The effective prompt should be computed at the turn boundary and stored only for
the active turn if needed.

## Host API Shape

WASM start-turn input should accept an optional system prompt update:

```ts
type SystemPromptUpdate =
  | { type: "keep" }
  | { type: "replace"; system_prompt: string }
  | { type: "append"; text: string };

type StartTurnInput = {
  prompt: AgentMessage;
  tools: ToolDefinition[];
  system_prompt?: SystemPromptUpdate;
};
```

SDK-level API can expose this through run options:

```ts
agent.run("Review this file", {
  systemPrompt: {
    type: "append",
    text: "For this turn, act as a strict code reviewer.",
  },
});
```

Keep the naming distinct from session-level `instructions` in `AgentConfig`.
Possible SDK names:

- `turnInstructions`
- `systemPrompt`
- `systemPromptUpdate`

Prefer `turnInstructions` for user-facing API and map it to core
`SystemPromptUpdate`.

## Relationship to Prepare Next Turn

Per-turn system prompt can be added independently, but it should align with the
future `PrepareNextTurn` extension point.

Eventually hosts should be able to return:

```rust
NextTurnUpdate {
    system_prompt: Some(SystemPromptUpdate::Append { text }),
    ..
}
```

That supports workflows where one turn's result determines the next turn's
instructions.

## Tests

Add Rust core tests for:

- default behavior preserves session-level prompt
- `Replace` uses only the per-turn prompt for LLM context
- `Append` includes base prompt and appended prompt in deterministic order
- per-turn prompt does not mutate base session prompt
- continuation after a per-turn prompt returns to base prompt unless another
  update is supplied

Add WASM/host tests for:

- start-turn input accepts no prompt update
- start-turn input accepts replace update
- start-turn input accepts append update
- serialized DTO roundtrip preserves update type and content

Add SDK tests for:

- `turnInstructions` reaches the model request
- session-level `instructions` remain unchanged across runs
- consecutive runs with different turn instructions produce different effective
  prompts without recreating the agent

## Non-Goals

Do not implement these in the first pass:

- prompt template registry
- provider-specific prompt formatting
- automatic prompt selection by model
- persistent mutation of base session prompt
- arbitrary context transform callbacks
- full `PrepareNextTurn` model/tool/budget update

## Implementation Order

1. Add `SystemPromptUpdate` to `pi-core`.
2. Add effective prompt computation helper.
3. Thread optional prompt update through `start_turn`.
4. Update context construction to use the effective prompt.
5. Keep existing `start_turn` compatibility through a wrapper or default.
6. Add WASM DTO support.
7. Add SDK run option, preferably `turnInstructions`.
8. Add tests at core, WASM host, and SDK layers.

## Success Criteria

- Hosts can provide per-turn instructions without recreating the session.
- Base session prompt remains stable unless explicitly changed.
- The effective prompt used for each LLM request is deterministic and testable.
- Core remains synchronous, typed, and runtime-free.
