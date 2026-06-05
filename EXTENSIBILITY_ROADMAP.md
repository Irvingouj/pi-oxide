# Extensibility Roadmap

This roadmap captures the extension points `pi-oxide` should add to approach
the extensibility of `../pi` while preserving the central `pi-oxide` constraint:
the Rust core must remain synchronous, typed, runtime-free, and portable across
web, local, mobile, and future hosts.

`../pi` can expose extension hooks as TypeScript callbacks because provider
calls, tool execution, context transforms, session state, and async runtime all
live in the same environment. `pi-oxide` cannot use that shape directly. The
equivalent extension model should be:

```text
pi callback hook
-> pi-oxide typed action/event
-> serializable host response
```

The goal is not to port `../pi` to Rust. The goal is to keep `pi-oxide`'s typed
host/core boundary while making policy and orchestration extensible.

## Principles

1. Keep `pi-core` runtime-free.
   - No Tokio, browser, filesystem, HTTP, shell, or JS callback assumptions.
   - Hosts perform side effects; core emits typed actions and accepts typed
     results.

2. Prefer typed policy over hardcoded behavior.
   - Context projection, tool permissions, continuation, and model selection
     should be configurable through serializable structs.

3. Keep provider-specific formatting outside core.
   - Core owns agent state, typed messages, context projection decisions, and
     runtime-neutral policies.
   - Hosts own provider APIs, request payloads, auth, transport, retries, and
     stream parsing.

4. Extension points should compose.
   - Tool packs should be able to contribute tool definitions, projection
     policies, permission hints, artifact behavior, and execution defaults.

## Priority 1: Configurable Context Projection Policy

Current issue: projection strategy is hardcoded by tool name in `pi-core`.
Adding new tool categories or platform-specific tools currently requires core
changes.

Add a serializable projection policy:

```rust
pub struct ContextProjectionPolicy {
    pub default_tool_strategy: ProjectionStrategy,
    pub per_tool: HashMap<ToolName, ProjectionStrategy>,
}

pub enum ProjectionStrategy {
    KeepFull,
    Head { min_age_turns: u32, max_chars: usize },
    Tail { min_age_turns: u32, max_chars: usize },
    HeadTail { min_age_turns: u32, head_chars: usize, tail_chars: usize },
    DropIfOld { min_age_turns: u32 },
}
```

Expected outcome:

- Core still owns projection decisions.
- Hosts and tool packs can define policy without changing core.
- Default policy can preserve current behavior.

## Priority 2: Before Tool Call Preparation

`../pi` has `beforeToolCall`. `pi-oxide` should add an equivalent typed
preparation phase before executing tools.

Possible shape:

```rust
pub enum AgentAction {
    PrepareToolCalls { calls: Vec<ToolCall> },
    ExecuteTools { calls: Vec<PreparedToolCall> },
}

pub enum ToolCallDecision {
    Allow,
    Block { reason: String },
    RewriteArgs { arguments: ToolArguments },
    RequireApproval { reason: String },
}
```

Use cases:

- Permission checks.
- User approval for dangerous actions.
- Schema-adjacent validation.
- Platform capability checks.
- Argument normalization.
- Sandbox policy.

Expected outcome:

- Tool safety becomes extensible without putting callbacks in Rust core.
- Hosts can enforce platform-specific constraints before execution.

## Priority 3: After Tool Call Finalization

`../pi` has `afterToolCall`, which can rewrite results, mark errors, and
terminate the loop. `pi-oxide` should make tool result finalization explicit and
typed.

Possible additions:

```rust
pub struct ToolResult {
    pub content: Vec<Content>,
    pub details: Option<ToolDetails>,
    pub terminate: Option<bool>,
    pub visibility: Option<ToolResultVisibility>,
    pub projection_hint: Option<ProjectionStrategy>,
}

pub enum ToolResultVisibility {
    Full,
    PreviewOnly,
    HiddenFromModel,
}
```

Use cases:

- Hide sensitive tool output from the model.
- Attach binary/browser/file artifacts while passing previews to the model.
- Override projection behavior for a specific result.
- Convert raw host output into model-readable content.
- Stop the agent after terminal tool results.

Expected outcome:

- Artifact and projection behavior becomes tool-pack extensible.
- Core remains responsible for applying typed result policy.

## Priority 4: Prepare Next Turn

`../pi` can update model, context, and thinking after each turn through
`prepareNextTurn`. `pi-oxide` should support a typed equivalent.

Possible shape:

```rust
pub struct NextTurnUpdate {
    pub model: Option<Model>,
    pub thinking_level: Option<ThinkingLevel>,
    pub system_prompt: Option<String>,
    pub tools: Option<Vec<ToolDefinition>>,
    pub context_budget: Option<ContextProjectionBudget>,
    pub projection_policy: Option<ContextProjectionPolicy>,
}
```

This can be driven by either:

- host calling a `prepare_next_turn(update)` API after `TurnEnd`, or
- core emitting `AgentAction::PrepareNextTurn { summary }` and accepting a
  typed host response.

Use cases:

- Switch models by task phase.
- Adjust thinking level after tool failures.
- Change active tools based on host state.
- Tighten context budget near window limits.
- Support multi-agent or workflow handoff.

Expected outcome:

- Dynamic orchestration becomes possible without runtime-specific core logic.

## Priority 4a: Per-Turn System Prompt

Add a typed way to provide turn-specific instructions without recreating the
agent session or mutating the base session prompt. See
`PER_TURN_SYSTEM_PROMPT_PLAN.md`.

Possible shape:

```rust
pub enum SystemPromptUpdate {
    Keep,
    Replace { system_prompt: String },
    Append { text: String },
}
```

Expected outcome:

- Hosts can adjust instructions per turn.
- The base session prompt remains stable.
- Future `PrepareNextTurn` can reuse the same prompt update type.

## Priority 5: Continuation Policy

`../pi` has `shouldStopAfterTurn`. `pi-oxide` needs a portable way to prevent
runaway loops and support different product modes.

Possible shape:

```rust
pub struct TurnContinuationPolicy {
    pub continue_after_tools: bool,
    pub continue_after_assistant_without_tools: bool,
    pub max_turns: Option<u32>,
    pub max_tool_rounds: Option<u32>,
}

pub enum ContinuationDecision {
    Continue,
    Stop { reason: String },
    WaitForInput,
}
```

Use cases:

- Stop after current turn.
- Enforce max tool rounds.
- Chat mode versus automation mode.
- Budget guards.
- Workflow-specific stopping rules.

Expected outcome:

- Loop continuation becomes configurable and testable in Rust.

## Priority 6: Provider Registry in Host SDKs

Rust core should not own provider registries. The web/JS SDK should expose a
clear provider extension surface similar to `../pi`.

Possible TypeScript shape:

```ts
registerModelProvider({
  id: "anthropic",
  createModel(config) {
    return defineModel(...);
  },
  supports: {
    streaming: true,
    tools: true,
    vision: true,
  },
});
```

Expected outcome:

- `defineModel` remains useful for one-off custom models.
- `registerModelProvider` supports plugin ecosystems and reusable providers.
- Core continues to see only typed `Model` and provider-neutral LLM results.

## Priority 7: Tool Pack Registry

Tool packs should carry more than handlers. A reusable tool pack should be able
to contribute runtime-neutral policy.

Possible TypeScript shape:

```ts
registerToolPack({
  id: "browser",
  tools: [...],
  permissions: ...,
  projectionPolicy: ...,
  artifactPolicy: ...,
  defaultExecutionMode: "parallel",
});
```

Expected outcome:

- Browser, filesystem, shell, artifact, and app-specific tools become reusable
  packages.
- Tool packs can configure core policy through typed structs.
- Hosts remain responsible for actual execution.

## Priority 8: Typed Context Injection

`../pi` has flexible `transformContext`. `pi-oxide` should avoid arbitrary
message transforms in core, but still allow hosts to inject runtime state in a
typed way.

Possible shape:

```rust
pub enum ContextInjection {
    SystemAppend { text: String },
    UserVisibleNote { text: String },
    HiddenPolicy { text: String },
    ToolState { tool_name: ToolName, summary: String },
}
```

Use cases:

- Browser page state.
- Repository snapshot.
- Selected files.
- Host capability state.
- User preferences.
- Ephemeral UI state.

Expected outcome:

- Hosts can enrich context without smuggling unstructured messages deep into
  core.
- Core keeps deterministic, testable context construction.

## Implementation Order

1. Add `ContextProjectionPolicy` and remove hardcoded tool-name projection.
2. Add before-tool-call preparation as typed action/response.
3. Extend tool result finalization with visibility and projection hints.
4. Add next-turn update support for model, thinking, tools, budget, and policy.
5. Add continuation policy.
6. Add provider registry to the web SDK.
7. Add tool pack registry to the web SDK.
8. Add typed context injection.

## Success Criteria

- New tools can define projection behavior without modifying `pi-core`.
- Hosts can block, approve, or rewrite tool calls before execution.
- Tool results can control visibility, projection, artifact behavior, and
  termination through typed fields.
- Hosts can switch model, tools, thinking level, and context policy between
  turns.
- Web SDK extension ergonomics are close to `../pi`, while core remains
  cross-platform and runtime-free.
