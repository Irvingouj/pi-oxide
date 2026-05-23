# Milestone 5: Minimal Context Projection

Implement this milestone next, after Milestone 4 local tools.

## Read First

- `CLAUDE.md`
- `ROADMAP.md`
- `AGENT_RUNTIME_MEMO.md`
- `web/src/providers/realLlm.ts`
- `web/src/providers/anthropic.ts`
- `web/src/providers/types.ts`
- `web/src/local/*`
- `web/src/tools/*`

## Goal

Prepare bounded provider context before LLM calls, without mutating Rust's canonical transcript.

The Rust core should keep the true typed message history. The JS host should create a provider-bound projection each turn:

```text
Rust LlmContext
-> prepareContext()
-> provider-ready LlmRequest
-> Anthropic adapter
```

This milestone is about context shape and tool-result budgeting. It is not about real local coding smoke yet.

## Constraints

- Do not change `pi-core`.
- Do not add UI.
- Do not add browser APIs.
- Do not implement summarizer compaction.
- Do not run real network smoke unless explicitly asked.
- Keep runtime-specific storage in JS/TS host code.
- Keep provider adapter changes minimal.
- Preserve existing tests.
- Do not commit.

## Suggested Files

- `web/src/context/tokenEstimate.ts`
- `web/src/context/artifactStore.ts`
- `web/src/context/toolResultBudget.ts`
- `web/src/context/prepareContext.ts`
- `web/test/contextProjection.test.ts`

Small edits may be needed in:

- `web/src/providers/realLlm.ts`
- `web/src/providers/types.ts`

## Required Concepts

### Canonical Transcript

The input from Rust is canonical:

```ts
{
  system_prompt: string
  messages: AgentMessageShape[]
  tools: ToolDefinition[]
}
```

Do not mutate the input messages in place.

### Provider Projection

`prepareContext()` returns a new provider-bound request with projected messages:

```ts
export interface PreparedContext {
  request: LlmRequest
  estimatedTokens: number
  replacements: ContextReplacement[]
}
```

The `request` should remain compatible with `callAnthropic()`.

### Artifact Store

Add an artifact store interface:

```ts
export interface ArtifactStore {
  put(record: ArtifactRecord): string
  get(id: string): ArtifactRecord | undefined
}
```

First implementation:

```ts
MemoryArtifactStore
```

Artifacts should store full tool-result text when the provider projection replaces that text with a preview.

Do not add filesystem/IndexedDB persistence in this milestone.

## Required Behavior

### 1. Token Estimate

Add `estimateTokensForMessages(messages)`.

Rules:

- Estimate text content as `Math.ceil(chars / 4)`.
- Include assistant tool call names and serialized arguments.
- Include tool result text.
- Include assistant usage if useful, but keep first version simple and deterministic.

Tests should cover:

- user text
- assistant text
- assistant tool call arguments
- tool result text

### 2. Tool Result Budget

Add a deterministic tool-result budgeting function.

Input:

```ts
AgentMessageShape[]
```

Output:

```ts
{
  messages: AgentMessageShape[]
  replacements: ContextReplacement[]
}
```

Behavior:

- Only inspect `role: "tool_result"` messages.
- If total text size is under `maxToolResultChars`, keep it unchanged.
- If too large:
  - store full text in artifact store
  - replace content with deterministic preview text
  - include artifact id/reference in preview text
  - preserve `tool_call_id`, `tool_name`, `is_error`, `timestamp`, and `details`

Preview policy:

- For `tool_name === "bash"`, keep tail preview.
- For `tool_name === "read"`, keep head preview.
- For other tools, use head preview.
- Include a clear marker like:

```text
<context-artifact id="..." tool="bash">
Tool result was too large and was replaced with a preview.
Full content is stored in artifact: ...
Preview:
...
</context-artifact>
```

Determinism:

- Re-running projection on the same messages with the same replacement state must produce byte-identical projected messages.
- Do not generate random IDs inside projection unless they are stable from `tool_call_id`.

Recommended artifact id:

```text
tool-result-{tool_call_id}
```

### 3. Replacement State

Add a small state object so projection decisions survive turns:

```ts
export interface ContextProjectionState {
  replacements: Map<string, ContextReplacement>
}
```

Use `tool_call_id` as the stable key.

Behavior:

- If a tool result was already replaced, reuse the same replacement text.
- If a tool result was previously kept inline, keep it inline in future prepares.
- Do not flip decisions across turns.

### 4. Recent Window Trimming

Add simple trimming after tool-result budgeting.

Behavior:

- If estimated tokens are below `maxContextTokens`, keep all projected messages.
- If above budget, drop oldest messages until under budget.
- Do not split a tool call from its corresponding tool result.
- Keep the newest user message if possible.
- Preserve ordering.

Simplify if needed:

- First version may drop whole turns from the front.
- A turn can be approximated as user -> assistant -> following tool_result messages.
- Never leave a `tool_result` whose assistant `tool_call` is no longer present.

### 5. Provider Normalization Boundary

Keep `convertMessages()` in `anthropic.ts` responsible for Anthropic-specific formatting.

`prepareContext()` should be provider-neutral:

- no Anthropic `tool_use`
- no Anthropic `tool_result`
- no provider-specific cache controls

But it may ensure obvious transcript hygiene:

- no empty text-only tool result after replacement
- no in-place mutation

### 6. Wire Into RealLlm

Update `web/src/providers/realLlm.ts` so `RealLlm` can optionally use context projection before calling the provider.

Suggested shape:

```ts
const llm = new RealLlm({
  apiKey,
  baseUrl,
  model,
  contextProjection?: {
    state,
    artifacts,
    budget,
  }
})
```

If no context projection config is provided, behavior should remain unchanged.

## Tests

Add `web/test/contextProjection.test.ts`.

Cover:

- token estimate counts user text
- token estimate counts assistant tool call arguments
- small `read` tool result remains unchanged
- large `read` tool result becomes head preview
- large `bash` tool result becomes tail preview
- full replaced result is stored in `MemoryArtifactStore`
- repeated prepare with same state gives byte-identical projected messages
- canonical input messages are not mutated
- trimming drops old messages when over budget
- trimming does not leave orphan `tool_result`
- `RealLlm` without projection keeps current behavior
- `RealLlm` with projection sends projected messages to provider path if this can be tested without network

Do not overfit tests to private implementation details. Test user-visible projection behavior.

## Verification

Run:

```bash
cargo test --workspace
cd web && npm test
```

Expected:

- Rust tests pass.
- JS tests pass.
- No changes to `pi-core`.
- Existing local tool tests remain green.

## Report Back

Report:

- changed files
- test results
- whether `RealLlm` projection wiring was implemented
- any TODOs or intentionally deferred behavior

Do not commit.
