# pi-oxide Roadmap

## Mission

Build a Rust-based coding-agent runtime whose core is type-safe, runtime-neutral, and host-driven.

The first product path is a web-based, JavaScript-driven coding agent:

- Rust owns the synchronous agent state machine, typed wire/domain contracts, and coding-agent invariants.
- JavaScript owns the web host event loop, model/provider calls, browser storage, UI, and tool execution.
- The browser host drives Rust by consuming `AgentAction` values and feeding back typed results.

The goal is not to build a desktop app first. Desktop may remain compilable as a scaffold, but it is not a near-term milestone.

## Source Principles

This plan follows the agent-building principles from Anthropic's "Building effective agents":

- Start simple.
- Prefer composable building blocks over framework complexity.
- Keep the loop transparent and observable.
- Design the agent-computer interface carefully.
- Add autonomy only when the simpler workflow is proven insufficient.

For this repository, that means the first real system is an augmented LLM plus typed tools in a feedback loop:

```text
user prompt
-> Rust core emits AgentAction
-> JS host executes LLM/tool action
-> JS host feeds typed result back into Rust
-> Rust core updates state and emits events/actions
-> repeat until Finished or WaitForInput
```

## Current Baseline

Already present:

- `pi-core`: synchronous Rust agent state machine, no async runtime.
- `pi-bindings`: C ABI with JSON envelope responses.
- `pi-host-web`: WASM host scaffold.
- `pi-llm`: protocol/type surface, now sharing model types with `pi-core`.
- Typed wrappers for important identifiers and JSON domains.
- Basic tracing in core and bindings.
- Workspace includes host crates, so web/desktop/mobile scaffolds compile in normal tests.

Known limitation:

- The project does not yet have a real JS-driven web host loop.
- The project does not yet execute a complete browser agent run against fake or real providers.
- Tool interfaces exist only as definitions/results, not as a coding-agent tool set.

## Non-Goals

- No desktop-first runner.
- No TUI.
- No provider registry before one provider path works.
- No multi-agent orchestration before a single agent loop is verified.
- No broad framework abstraction before the action protocol is proven.
- No runtime-specific behavior inside `pi-core`.

## Architecture Target

### Rust Core

Rust crates should provide:

- `pi-core`: runtime-neutral state machine.
- `pi-bindings`: stable native ABI.
- `pi-host-web`: WASM-facing wrapper around core operations.
- Future Rust coding-agent domain crate if needed, for typed tool definitions and coding-agent-specific policies.

Rust must not:

- Fetch models from network.
- Read/write browser files directly.
- Assume Tokio, DOM, Node, Web Worker, or any browser API in `pi-core`.
- Accept unparsed stringly data past the binding layer.

### JavaScript Web Host

The JS host should provide:

- Event loop that executes `AgentAction`.
- Provider adapter for one initial model API.
- Tool executor registry.
- Browser storage/session adapter.
- UI event bridge.
- Tracing/log sink.

JS may call:

- WASM exports from `pi-host-web`.
- Browser APIs such as File System Access API, IndexedDB, OPFS, fetch, Web Workers, and UI frameworks.

JS must preserve:

- Typed JSON envelope contracts at the Rust boundary.
- Event order.
- Action/result correlation by typed IDs.
- Useful error details.

## Milestone 0: Planning and Guardrails

Goal: make the project direction explicit enough for cheaper coding agents to execute without drifting.

Scope:

- Keep `CLAUDE.md` aligned with web-first direction.
- Keep this `ROADMAP.md` as the root planning document.
- Every future large task should point to a milestone in this file.

Verification:

- `AGENTS.md` aliases `CLAUDE.md`.
- `CLAUDE.md` says web/JS host is first-level support.
- `ROADMAP.md` exists at project root.

Status: complete.

## Milestone 1: Web WASM Binding Contract

Goal: expose the current Rust core through browser-friendly WASM APIs with typed JSON envelopes.

Scope:

- Replace `WebRunner::hello()` with a minimal agent wrapper.
- Export functions equivalent to:
  - create agent from `AgentOptions`
  - prompt
  - feed LLM chunk
  - LLM done
  - tool done
  - steer
  - follow up
  - state
  - reset
- Return the same envelope shape as bindings:
  - `{ ok: true, data: ... }`
  - `{ ok: false, error: { code, message } }`
- Keep JS-facing payloads serializable and stable.
- Add WASM-side errors using concrete Rust error types with `thiserror`.

Non-goals:

- No real provider calls.
- No real tool execution.
- No UI.

Verification:

- Rust unit tests cover successful prompt and parse failure.
- WASM package builds for `wasm32-unknown-unknown`.
- A tiny JS smoke script can create an agent and receive `StreamLlm`.

Suggested files:

- `pi-host-web/src/lib.rs`
- `pi-host-web/Cargo.toml`
- `pi-core/tests/agent_smoke.rs`

Status: complete.

Build note: WASM build requires rustup-managed `rustc` (not Homebrew). Use:
```
PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH" cargo build -p pi-host-web --target wasm32-unknown-unknown --release
wasm-bindgen --target nodejs --out-dir web/pkg target/wasm32-unknown-unknown/release/pi_host_web.wasm
cp web/pkg/pi_host_web.js web/pkg/pi_host_web.cjs  # CJS compat for Node ESM
```

## Milestone 2: JS Host Loop with Fake LLM and Fake Tools

Goal: prove the full browser host contract without external APIs.

Scope:

- Add a JS/TS host package or example under the repository.
- Implement `runAgentLoop(agent, host)`:
  - call Rust prompt
  - process returned actions
  - for `StreamLlm`, use fake streaming chunks
  - for `ExecuteTools`, call fake tool handlers
  - feed results back into Rust
  - stop on `Finished` or `WaitForInput`
- Preserve and display event order.
- Add deterministic tests for:
  - no-tool response
  - one tool call
  - multiple tool calls
  - tool error
  - follow-up
  - steering

Non-goals:

- No real network.
- No browser UI polish.
- No provider-specific formatting.

Verification:

- JS tests run from a clean checkout.
- The fake loop completes at least one tool-using run.
- Logs show every `AgentAction` and every resulting Rust event.

Suggested files:

- `web/package.json`
- `web/src/agentHost.ts`
- `web/src/fakeLlm.ts`
- `web/src/fakeTools.ts`
- `web/test/agentHost.test.ts`

## Milestone 3: Coding-Agent Tool Contract

Goal: define the first typed coding-agent tool set for web execution.

Scope:

- Define tool schemas and result shapes for the minimal coding loop:
  - `read_file`
  - `list_files`
  - `search_files`
  - `write_file`
  - `run_command` as a constrained host capability, if the web environment supports it
- Tool definitions must be model-friendly:
  - clear names
  - clear descriptions
  - examples where useful
  - explicit boundaries
  - no hard-to-write formats such as manual patch hunk counts as the first edit tool
- Arguments and results must be typed before crossing into Rust-facing contracts.
- Prefer absolute or workspace-root-relative paths, never implicit current-directory semantics.

Non-goals:

- No edit diff tool yet unless `write_file` proves insufficient.
- No unrestricted shell from browser.
- No hidden filesystem assumptions in Rust core.

Verification:

- Tool schema tests ensure required fields and path semantics.
- Fake host can run a loop where the model reads, writes, and gets feedback.
- Bad tool arguments produce useful typed errors.

Suggested files:

- Rust side if shared definitions are needed:
  - `pi-core/src/tool.rs`
  - new `pi-coding-agent` crate only if the contract becomes large enough
- JS side:
  - `web/src/tools/*.ts`
  - `web/test/tools/*.test.ts`

## Milestone 4: Browser Storage and Session Tree

Goal: persist agent state and coding sessions in the web host.

Scope:

- Store sessions in IndexedDB or OPFS through JS.
- Keep Rust core session structures runtime-neutral.
- Support:
  - create session
  - append event/message entries
  - load session
  - resume from current leaf
  - inspect branch
- Keep storage failures visible as host errors, not silent UI failures.

Non-goals:

- No cloud sync.
- No collaboration.
- No compaction until basic sessions work.

Verification:

- Reloading the page can restore a fake agent session.
- Session data can round-trip through Rust state and JS storage.
- Corrupt session data reports a useful error.

Suggested files:

- `web/src/storage/sessionStore.ts`
- `web/test/storage/sessionStore.test.ts`
- `pi-core/src/session.rs`

## Milestone 5: One Real Provider Path

Goal: connect one real model provider through the JS host.

Preferred initial path:

- Anthropic Messages API from JS host, or an OpenAI-compatible endpoint if easier for local development.

Scope:

- Implement one provider adapter.
- Convert provider streaming events into Rust `LlmChunk` and `LlmResult`.
- Convert Rust `ToolDefinition` into provider tool format.
- Preserve provider errors with useful code/message/details.
- Keep API key handling outside Rust core.

Non-goals:

- No provider registry.
- No all-provider compatibility layer.
- No model auto-routing.

Verification:

- One real prompt streams text through Rust events.
- One real prompt can request a fake tool and continue after tool result.
- Provider error and abort paths produce typed Rust-visible failures.

Suggested files:

- `web/src/providers/anthropic.ts`
- `web/src/providers/types.ts`
- `web/test/providers/*.test.ts`

## Milestone 6: Minimal Web Coding Agent

Goal: run a real web coding-agent loop on a browser-accessible workspace.

Scope:

- Build a minimal UI that shows:
  - transcript
  - current actions
  - tool calls and results
  - errors
  - session state
- Use browser storage or a browser workspace capability for files.
- Implement enough tools for the agent to inspect and modify files.
- Keep all agent state transitions visible.

Non-goals:

- No marketing landing page.
- No desktop parity.
- No multi-agent mode.

Verification:

- User can load a small workspace.
- User can ask the agent to make a small code change.
- Agent reads files, writes a change, and reports completion.
- The run can be replayed from persisted session state.

Suggested files:

- `web/src/ui/*`
- `web/src/agentHost.ts`
- `web/src/tools/*`

## Milestone 7: Evaluation Harness

Goal: make the coding agent objectively measurable.

Scope:

- Add a small suite of local coding tasks.
- Each task includes:
  - initial files
  - user prompt
  - expected file changes or black-box checks
  - optional test command if supported by the host
- Run tasks against fake provider first, then real provider.
- Store transcripts and tool traces for inspection.

Non-goals:

- No SWE-bench scale yet.
- No leaderboard.
- No auto-optimization loop.

Verification:

- CI can run fake-provider evals.
- Real-provider evals can run manually with an API key.
- Failures include enough trace data to debug tool/prompt/model issues.

Suggested files:

- `evals/tasks/*`
- `evals/run.ts`
- `evals/README.md`

## Milestone 8: Compaction and Long-Running Control

Goal: support longer coding sessions without losing transparency or control.

Scope:

- Add session compaction as an explicit workflow, not hidden magic.
- Add stop conditions:
  - max turns
  - max tool calls
  - max token estimate
  - user stop
- Add checkpoints where the agent can ask for human input.

Non-goals:

- No autonomous long-running background mode until the basic web agent is trusted.
- No opaque memory system.

Verification:

- A long fake session compacts deterministically.
- Compaction entries are visible in the session tree.
- Stop conditions reliably produce `WaitForInput` or `Finished`.

Suggested files:

- `pi-core/src/session.rs`
- `web/src/session/compaction.ts`
- `web/test/session/*.test.ts`

## Execution Rules for Cheaper Agents

When implementing a milestone:

1. Do not skip milestones unless explicitly told.
2. Start with tests that express user-visible behavior.
3. Keep Rust core runtime-neutral.
4. Keep JS host runtime-specific behavior out of Rust core.
5. Use typed structs at every Rust boundary.
6. Return useful errors with concrete codes and messages.
7. Add tracing at action boundaries and recoverable failures.
8. Keep changes scoped to the milestone.
9. Do not build desktop features unless the milestone says so.

## Definition of Done for the Next Step

The next step is Milestone 1.

Milestone 1 is done when:

- `pi-host-web` exposes a usable WASM wrapper around the Rust agent.
- JS can create an agent, send a prompt, and receive a typed `StreamLlm` action.
- Invalid JS input returns a typed error envelope.
- The implementation builds for the web target.
- Existing `cargo test --workspace` stays green.
