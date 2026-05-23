# pi-oxide Roadmap

## Mission

Build a Rust-based coding-agent runtime whose core is type-safe, runtime-neutral, and host-driven.

The product direction is still a browser-capable coding agent, but the next proving ground is a normal-computer local host. A functional coding agent must first be able to read files, edit files, write files, run commands, and manage context on a real machine. Once that loop is trusted locally, the browser host becomes another runtime implementation of the same protocol.

```text
user prompt
-> Rust core emits AgentAction
-> JS host prepares context and calls provider/tools
-> JS host feeds typed results back into Rust
-> Rust core updates state and emits events/actions
-> repeat until Finished or WaitForInput
```

## Source Principles

This plan follows Anthropic's "Building effective agents":

- Start simple.
- Prefer composable building blocks over framework complexity.
- Keep the loop transparent and observable.
- Design the agent-computer interface carefully.
- Add autonomy only when the simpler workflow is proven insufficient.

For this repository, that means:

- Rust owns the synchronous agent state machine, typed wire/domain contracts, and coding-agent invariants.
- JS hosts own runtime behavior: provider calls, filesystem/browser storage, shell/browser capabilities, permissions, context projection, and UI.
- The Rust core must not assume Tokio, DOM, Node, browser APIs, filesystem, shell, or network.

See also: [AGENT_RUNTIME_MEMO.md](./AGENT_RUNTIME_MEMO.md).

## Current Baseline

Already present:

- `pi-core`: synchronous Rust agent state machine.
- `pi-bindings`: C ABI with JSON envelope responses.
- `pi-host-web`: WASM wrapper with agent lifecycle exports.
- `web/src/agentHost.ts`: JS host loop for fake providers/tools.
- `web/src/providers/anthropic.ts`: first real provider adapter.
- `web/src/tools/*`: in-memory pi-compatible tool surface.
- Typed wrappers for important Rust identifiers and JSON domains.
- Basic tracing in core/bindings.

Completed proof points:

- Rust tests pass.
- JS fake loop can handle streaming, tool calls, tool errors, follow-ups, and steering.
- Anthropic conversion handles grouped tool results.
- In-memory tools support `read`, `write`, `edit`, `bash`, `grep`, `find`, `ls`.
- Real LLM smoke script exists, but only runs when `ANTHROPIC_API_KEY` is set.

Current gap:

- The project does not yet have a functional local-machine coding agent.
- The project does not yet have real filesystem-backed tools.
- The project does not yet have real constrained bash.
- The project does not yet have context projection/tool-result budgeting.
- The project does not yet have browser UI/storage.

## Non-Goals

- No desktop-first app shell.
- No TUI as a product goal.
- No provider registry before one provider path works end-to-end.
- No multi-agent orchestration before the single-agent loop is trusted.
- No hidden runtime behavior inside `pi-core`.
- No automatic long-running autonomy before local tool safety and context management are explicit.

## Architecture Target

### Rust Core

Rust crates should provide:

- `pi-core`: runtime-neutral state machine.
- `pi-bindings`: stable native ABI.
- `pi-host-web`: WASM-facing wrapper around core operations.
- Future Rust coding-agent domain crate only if shared typed contracts outgrow `pi-core`.

Rust must not:

- Fetch models from network.
- Read/write files directly as part of `pi-core`.
- Execute shell commands.
- Assume browser, Node, Tokio, OS-specific APIs, or UI.
- Accept unparsed stringly data past the Rust boundary.

### JS Host Runtime

JS hosts should provide:

- Event loop that executes `AgentAction`.
- Provider adapter.
- Tool executor registry.
- Permission/safety policy.
- Context projection before provider calls.
- Artifact/session storage.
- Trace/log sink.

There will be two JS host targets:

- local-machine host first: Node filesystem + shell + local artifacts.
- browser host later: File System Access API/OPFS/IndexedDB/remote runner as available.

## Milestone 0: Planning and Guardrails

Goal: make project boundaries explicit.

Status: complete.

Verification:

- `AGENTS.md` aliases `CLAUDE.md`.
- `CLAUDE.md` documents type safety, runtime-neutral core, web/JS direction, useful errors, tracing, abstraction, and simplicity.
- `ROADMAP.md` exists at project root.

## Milestone 1: Web WASM Binding Contract

Goal: expose the current Rust core through browser-friendly WASM APIs with typed JSON envelopes.

Status: complete.

Scope:

- Agent lifecycle exports: create, prompt, feed chunk, LLM done, tool done, steer, follow-up, state, reset, destroy.
- Stable envelope shape:
  - `{ ok: true, data: ... }`
  - `{ ok: false, error: { code, message } }`
- Concrete WASM-side errors with `thiserror`.

Verification:

- Rust unit tests cover success and parse failures.
- WASM target builds.

## Milestone 2: JS Host Loop with Fake LLM and Fake Tools

Goal: prove the host-driven Rust loop without external APIs.

Status: complete.

Scope:

- JS loop drives `StreamLlm`, `ExecuteTools`, `Finished`, and `WaitForInput`.
- Fake LLM emits streaming chunks and final `LlmResult`.
- Fake tools return deterministic success/error payloads.
- Trace records host actions, Rust events, and agent actions.

Verification:

- JS tests cover no-tool response, tool calls, parallel tool calls, tool error, follow-up, steering, LLM error, and trace order.

## Milestone 3: Coding-Agent Tool Contract

Goal: define and test the first coding-agent tool surface.

Status: complete.

Scope:

- In-memory pi-compatible tools:
  - `read`
  - `write`
  - `edit`
  - `ls`
  - `grep`
  - `find`
  - constrained fake `bash`
- Tool groups:
  - `PI_CODING_TOOLS = read, bash, edit, write`
  - `PI_READ_ONLY_TOOLS = read, grep, find, ls`
  - `PI_ALL_TOOLS = all seven`
- Legacy Milestone 3 tools remain for compatibility.

Verification:

- Schema tests.
- Path validation tests.
- Tool behavior tests.
- Deterministic fake programming smoke test.
- `grep` supports path as directory or exact file.

## Milestone 3.5: Real Provider Adapter

Goal: connect one real provider path through the JS host.

Status: complete enough for current stage.

Scope:

- Provider-neutral request/result types.
- Anthropic adapter for messages, tools, responses, and errors.
- Consecutive `tool_result` messages are grouped into one Anthropic user message.
- Real smoke script is available.

Verification:

- Anthropic conversion tests pass.
- Network smoke is skipped when `ANTHROPIC_API_KEY` is missing.

## Milestone 3.7: Real LLM Programming Smoke Script

Goal: make the real smoke script use pi-compatible tools and a programming fixture.

Status: complete as code; real network run still requires `ANTHROPIC_API_KEY`.

Scope:

- `web/scripts/real-llm-smoke.ts` uses `PI_CODING_TOOLS`.
- Fixture contains buggy `src/index.ts`.
- Prompt asks model to read, fix, and run `npm test`.
- Script verifies `read`, `edit` or `write`, `bash`, `Finished`, and final source content.

Verification:

- Unit tests pass.
- Real smoke should be manually run with:

```text
cd web && ANTHROPIC_API_KEY=... npm run smoke:real-llm
```

## Milestone 4: Local Machine Host Tools

Goal: implement real host-side coding tools for a normal computer.

This is now the next implementation target.

Scope:

- Add local host tool implementations, initially in JS/TS.
- Implement real filesystem-backed:
  - `read`: cwd-confined path resolution, offset/limit, head truncation.
  - `write`: cwd-confined writes, parent directory creation, serialized per-path mutation.
  - `edit`: exact replacement edits, useful errors, diff details, serialized per-path mutation.
  - `bash`: real command execution in cwd, timeout, abort/cancel, stdout/stderr capture, tail truncation.
- Reuse pi-compatible tool definitions where possible.
- Keep all runtime behavior outside Rust core.

Safety boundaries:

- Default deny paths outside cwd.
- Default deny unsafe bash unless permission mode explicitly allows it.
- Make all denied operations typed tool errors.
- Trace tool start/end/error with useful metadata.

Non-goals:

- No browser support in this milestone.
- No UI.
- No summarizer compaction.
- No broad permission framework beyond local explicit policy.

Verification:

- Tests use temporary fixture directories.
- `read/write/edit` operate on real files only inside fixture cwd.
- `bash` can run deterministic fixture commands.
- Path traversal and outside-cwd attempts fail.
- Existing Rust and JS tests remain green.

Suggested files:

- `web/src/local/path.ts`
- `web/src/local/fileTools.ts`
- `web/src/local/bashTool.ts`
- `web/src/local/localToolRegistry.ts`
- `web/test/localTools.test.ts`

## Milestone 5: Minimal Context Projection

Goal: prepare bounded provider context without mutating Rust's canonical transcript.

Scope:

- Add host-side context preparation before provider calls.
- Estimate tokens with chars/4 plus real assistant usage where available.
- Apply deterministic tool-result budgeting:
  - small tool results remain inline.
  - large `read` results keep head preview.
  - large `bash` results keep tail preview.
  - full content is stored in an artifact store.
  - replacement decisions remain stable across turns.
- Preserve tool-call/tool-result pairing.
- Normalize provider-bound messages enough to avoid Anthropic ordering errors.

Non-goals:

- No LLM summarizer yet.
- No automatic compaction.
- No prompt-cache engineering beyond deterministic output.
- No browser IndexedDB/OPFS store yet.

Verification:

- Small tool results pass through unchanged.
- Large tool results are replaced with deterministic previews.
- Full outputs are available through an artifact store.
- Repeated preparation of the same transcript produces byte-identical replacements.
- Trimming does not split an assistant tool call from its tool result.
- Anthropic adapter accepts prepared messages.

Suggested files:

- `web/src/context/tokenEstimate.ts`
- `web/src/context/artifactStore.ts`
- `web/src/context/toolResultBudget.ts`
- `web/src/context/prepareContext.ts`
- `web/test/contextProjection.test.ts`

## Milestone 6: Real Local Coding-Agent Smoke

Goal: prove a minimal functional coding agent on a normal computer.

Scope:

- Use the local machine host tools from Milestone 4.
- Use the context projection layer from Milestone 5.
- Use one real provider path.
- Run against a temporary local fixture repo.

Fixture:

- `package.json`
- `src/index.ts` with `add(a, b)` returning `a - b`
- deterministic test command

Required run:

- model calls `read src/index.ts`
- model calls `edit` or `write`
- model calls `bash npm test`
- run finishes

Verification:

- source file actually contains `return a + b`
- real bash command reports tests passing
- trace contains read, edit/write, bash
- context projection bounded any large outputs

## Milestone 7: Local Session and Artifact Persistence

Goal: persist local agent sessions and artifacts.

Scope:

- Append-only local session file.
- Session metadata: cwd, model, created time.
- Entries: messages, tool calls/results, artifacts, model changes, future compaction entries.
- Reload a session into `AgentOptions.messages`.
- Store large tool outputs as artifacts.

Non-goals:

- No branch tree unless needed.
- No cloud sync.
- No browser storage yet.

Verification:

- A local run can be resumed.
- Artifact references remain readable after reload.
- Corrupt session data returns useful errors.

## Milestone 8: Browser Host

Goal: move the proven agent loop into a browser-capable host.

Scope:

- Browser workspace abstraction.
- Browser artifact store via IndexedDB or OPFS.
- Browser session persistence.
- Minimal UI showing transcript, actions, tool calls/results, errors, and state.
- Use File System Access API or a virtual workspace first.
- Bash is unavailable unless backed by a safe remote/sandbox runner.

Non-goals:

- No marketing page.
- No desktop app.
- No hidden remote execution.

Verification:

- User can load or create a small workspace.
- Agent can inspect and modify files in the browser workspace.
- Session can be reloaded.
- UI reflects Rust actions/events and host tool results.

## Milestone 9: Manual Compaction and Long-Running Control

Goal: support longer sessions without opaque behavior.

Scope:

- Manual compaction command/workflow.
- Structured summary message.
- Keep recent token budget.
- Visible compaction entries in session history.
- Stop conditions:
  - max turns
  - max tool calls
  - max token estimate
  - user stop

Non-goals:

- No hidden automatic summarization until manual compaction is trusted.
- No autonomous background mode.
- No opaque memory system.

Verification:

- Long fake/local session compacts deterministically.
- Compaction summary preserves goal, constraints, files touched, decisions, and next steps.
- Stop conditions produce `WaitForInput` or `Finished`.

## Milestone 10: Evaluation Harness

Goal: make coding-agent progress measurable.

Scope:

- Small local coding tasks.
- Each task has:
  - initial files
  - prompt
  - expected file changes or black-box checks
  - optional test command
- Run fake-provider evals in CI.
- Run real-provider evals manually with API key.
- Store transcripts and traces.

Non-goals:

- No SWE-bench scale yet.
- No leaderboard.
- No auto-optimization loop.

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
9. Do not build desktop app features unless the milestone says so.

## Definition of Done for the Next Step

The next step is Milestone 4.

Milestone 4 is done when:

- real local `read/write/edit/bash` tools exist behind a host registry.
- tests prove they operate on real temporary files/commands.
- path traversal and outside-cwd access fail.
- bash has timeout and bounded output.
- no runtime-specific assumptions are added to `pi-core`.
- existing `cargo test --workspace` and `cd web && npm test` stay green.
