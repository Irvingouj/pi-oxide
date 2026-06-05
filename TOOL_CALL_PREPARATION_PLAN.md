# Tool Call Preparation Plan

This plan defines the first extension point to add after the current per-tool
result configuration: a typed preparation phase before tool execution.

The goal is to add the equivalent of `../pi`'s `beforeToolCall`, but shaped for
`pi-oxide`'s cross-platform architecture. Rust core must not hold runtime
callbacks. Instead, core emits typed actions and hosts return typed decisions.

## Problem

When an LLM emits tool calls, `pi-core` currently moves directly toward tool
execution. That leaves no explicit cross-platform extension point for:

- permission checks
- sandbox policy
- platform capability checks
- user approval
- argument normalization
- host-specific argument enrichment
- blocking unsafe or unsupported calls before side effects happen

Tool result configuration happens after execution. This plan adds the missing
pre-execution seam.

## Core Model

Tool call preparation has two independent decisions:

1. Transform decision
   - Does the host need to rewrite or normalize the call before execution?
   - This must not decide whether the call is allowed.

2. Permission decision
   - Is the final call allowed to execute?
   - This must not rewrite arguments.

Keeping these separate prevents policy from becoming a catch-all callback.

Preferred first-pass flow:

```text
raw ToolCall from model
-> host transform decision
-> host permission decision
-> execute allowed calls
-> synthesize error results for blocked calls
```

Permission should generally evaluate the final transformed call, because that is
the actual side effect the host will execute.

## Proposed Rust Types

Add preparation types to `pi-core`.

```rust
pub struct ToolCallPreparation {
    pub tool_call_id: ToolCallId,
    pub transform: ToolCallTransform,
    pub permission: ToolCallPermission,
}

pub enum ToolCallTransform {
    None,
    RewriteArgs { arguments: ToolArguments },
}

pub enum ToolCallPermission {
    Allow,
    Block { reason: String },
}
```

`RequireApproval` should not be in the first implementation unless the UI flow
is implemented at the same time. It can be added later:

```rust
pub enum ToolCallPermission {
    Allow,
    Block { reason: String },
    RequireApproval { reason: String },
}
```

## Proposed Agent Actions

Add a preparation action before execution:

```rust
pub enum AgentAction {
    PrepareToolCalls {
        calls: Vec<ToolCall>,
    },
    ExecuteTools {
        calls: Vec<ToolCall>,
    },
}
```

Existing `ExecuteTools` remains the side-effect action. `PrepareToolCalls` is a
host policy action.

## State Machine Behavior

When an assistant message contains tool calls:

1. Core stores pending raw tool calls.
2. Core emits `ToolExecutionStart` events only after preparation allows a call.
3. Core emits `AgentAction::PrepareToolCalls { calls }`.
4. Host returns `Vec<ToolCallPreparation>`.
5. Core applies transforms to matching calls.
6. Core converts blocked calls into error `ToolResultMessage`s.
7. Core emits `ExecuteTools` for allowed calls.
8. If all calls are blocked, core finalizes the tool batch and continues as if
   those tool results had completed.

Important invariant:

```text
Every tool call produced by the assistant must eventually produce exactly one
tool result message, whether executed, blocked, failed, or cancelled.
```

## Host API Shape

WASM host should expose an API similar to:

```rust
hostPrepareToolCalls(handle, preparations) -> HostStepOutput
```

The host SDK should expose user-facing hooks as ordinary JS functions, but only
outside core:

```ts
type ToolCallTransformHook = (call: ToolCall) =>
  | { type: "none" }
  | { type: "rewrite_args"; arguments: unknown }
  | Promise<{ type: "none" } | { type: "rewrite_args"; arguments: unknown }>;

type ToolCallPermissionHook = (call: ToolCall) =>
  | { type: "allow" }
  | { type: "block"; reason: string }
  | Promise<{ type: "allow" } | { type: "block"; reason: string }>;
```

The TS SDK combines hook outputs into `ToolCallPreparation` and passes typed
data to WASM.

## Ordering

First implementation should use:

```text
transform -> permission -> execute
```

Rationale:

- Permission sees the actual final arguments.
- The flow is simple enough for the first protocol change.
- Hosts can still implement conservative transform hooks that avoid hiding
  dangerous user/model intent.

Future extension may add raw-intent preflight:

```text
raw permission -> transform -> final permission -> execute
```

Do not implement this until there is a concrete need.

## Blocked Tool Result

Blocked calls should produce a normal error tool result:

```text
Tool call blocked by host policy: {reason}
```

This lets the model recover or explain the limitation using the same tool-result
flow it already understands.

The result should have:

- `is_error = true`
- original `tool_call_id`
- original `tool_name`
- text content with the block reason
- no side effect

## Tests

Add Rust core tests for:

- allowed call proceeds to `ExecuteTools`
- rewritten args are used for execution
- blocked call creates error tool result
- mixed batch preserves one result per call
- all blocked calls finalize the batch without host tool execution
- unknown or duplicate preparation responses are rejected or ignored with a
  useful error/event

Add WASM/host tests for:

- `PrepareToolCalls` appears before `ExecuteTools`
- `hostPrepareToolCalls` accepts allow decisions
- `hostPrepareToolCalls` applies rewrite decisions
- `hostPrepareToolCalls` blocks calls and emits mapped SDK events

Add SDK tests for:

- transform hook runs before permission hook
- permission hook sees transformed arguments
- blocked calls do not invoke tool handlers
- blocked calls appear in run result as failed tool runs

## Non-Goals

Do not implement these in the first pass:

- user approval UI
- async deferred approval
- raw-intent and final-intent double permission
- provider-specific tool validation
- tool result visibility or projection changes
- new tool pack registry

## Implementation Order

1. Add core preparation types.
2. Add `AgentAction::PrepareToolCalls`.
3. Update assistant tool-call handling to emit preparation instead of immediate
   execution.
4. Add core API to accept `Vec<ToolCallPreparation>`.
5. Apply transforms and synthesize blocked results.
6. Emit `ExecuteTools` only for allowed calls.
7. Add wasm DTOs and `hostPrepareToolCalls`.
8. Update TS engine to handle `prepare_tool_calls`.
9. Add SDK transform and permission hooks.
10. Add focused tests at core, wasm host, and SDK layers.

## Success Criteria

- Hosts can rewrite tool arguments before execution.
- Hosts can block unsafe tool calls before any side effect.
- Transform and permission are separate typed decisions.
- Core remains synchronous and runtime-free.
- Existing tool execution behavior remains unchanged for hosts that allow all
  calls with no transform.
