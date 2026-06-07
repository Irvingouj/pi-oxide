# Changelog

All notable changes to this project will be documented in this file.

## [0.4.0] - 2026-06-07

### Fixed

- Rebuilt and synchronized the checked-in WASM bindings so the web SDK uses the current `PreToolCall` and `ExecutingTools` state machine instead of the stale `WaitingTools` implementation.
- Consecutive tool-call rounds no longer stop after the second preparation request.

### Added

- End-to-end SDK regression coverage for two consecutive tool-call rounds followed by a final LLM response.

### Changed

- Workspace version bumped to `0.4.0`.
- `@pi-oxide/pi-host-web` SDK version bumped to `0.9.0`.
- `pi-oxide-web` version bumped to `0.6.0`.

## [0.3.0] - 2026-06-06

### Added

- **Typestate split: `PreToolCall` and `ExecutingTools`** — `WaitingTools` is now two explicit phases. `PreToolCallAgent` handles tool preparation/approval; `ExecutingToolsAgent` handles execution and completion. This removes the `preparations_applied` boolean that caused a silent multi-turn hang.
- `PreToolCall` and `ExecutingTools` variants added to `Phase` enum and `AgentRuntime`.
- `PreToolCallAgent::prepare_tool_calls()` transitions to `ExecutingTools` (or `ReadyToContinue` if all tools are blocked).
- `ExecutingToolsAgent::cancel_tool()` increments `turn_number` when all pending calls empty, matching `PreToolCallAgent::cancel_tool()` parity.
- `multi_turn_tool_execution_prepares_tools_on_second_batch` test verifies the multi-turn hang is fixed.

### Changed

- `AgentRuntime` variants: `WaitingTools` replaced with `PreToolCall` and `ExecutingTools`.
- `StreamingAgent::finish_llm()` now returns `FinishLlmTransition::PreToolCall` when tool calls are present.
- `ToolTransition` gains `PreToolCall` and `ExecutingTools` variants.
- `cancel_tool` and `submit_user_message` are available on both `PreToolCallAgent` and `ExecutingToolsAgent`.
- All hosts (WASM, TUI) dispatch to the new typestate variants.
- `pi-host-tui/src/app.rs` now calls `prepare_tool_calls` before emitting `ExecuteTools` directives.
- Workspace version bumped to `0.3.0`.
- `@pi-oxide/pi-host-web` SDK version bumped to `0.8.0`.
- `pi-oxide-web` version bumped to `0.5.0`.

### Fixed

- Multi-turn hang: second batch of tool calls no longer inherits the old batch's "prepared" bit.
- `cargo clippy --workspace -- -D warnings` is clean.
- `cargo fmt --all -- --check` is clean.
- All 115 tests pass across `pi-core`, `pi-host-tui`, and `pi-host-web`.
- Biome check is clean on source files (warnings remain in test files only).

## [0.2.0] - 2026-05-29

### Added

- **Rhai script engine** for dynamic context projection strategies. Scripts can call built-ins (`head`, `tail`, `contains`, `lines`, `filter`, `smart_budget`) and are sandboxed with a max-operations limit.
- **Unified `ToolProjectionState` enum** (`Inline | Deferred | Replaced`) replaces the previous two separate `replacements` and `deferred` maps, making invalid states unrepresentable.
- **Artifact store** in `projectionService` — a `Map<string, string>` capped at 1000 entries with FIFO eviction. Stores original tool result text for artifact retrieval and search.
- `artifact_read` and `artifact_search` browser tools, allowing agents to retrieve and search previously projected tool results.
- `ArtifactEntry` added to `SessionState` so artifact store contents persist across sessions.
- `snapshotArtifacts` / `loadArtifacts` methods on `ProjectionService` for round-trip artifact persistence.
- `ContextProjectionReport` now includes `cache_breakpoints` for cache-aware prompt engineering.
- `ProjectionStrategy::Dynamic { script }` variant for per-tool-result Rhai-driven compaction decisions.
- `#[serde(default)]` on all new projection fields for backward-compatible session deserialization.

### Changed

- **Projection engine redesign**: pure decision (`decide_projection`) + atomic application (`apply_projection`) pipeline. No mutation during decision-making.
- `ProjectionShape` renamed from `ProjectionStrategy` (old flat struct) to explicit shape enum: `KeepFull`, `Head`, `Tail`, `HeadTail`, `Microcompacted`.
- `min_age` replaces `old_after` for fixed-strategy deferral — clearer semantics (minimum turn count before projection).
- `DropIfOld` shape removed — trimming logic is now handled by `TrimBoundary` enum (`None`, `DropTurns`, `KeepLastTurn`).
- `ProjectionOutcome` is now a simple struct `{ text: string }` — the unused `Prompted` and `Deferred` variants were removed.
- `ScriptResult` no longer has a `Prompt` variant.
- `fallback_strategy` returns `ProjectionStrategy::Fixed { shape: KeepFull, min_age: 0 }` instead of `KeepFull` directly.
- `OldContextReplacement` and `OldContextStrategy` DTOs preserved for backward-compatible session migration.
- `SessionState` now includes `projection_state` and `artifacts` fields.
- SDK (`@pi-oxide/pi-host-web`) exports `projectContext`, `ToolProjectionState`, `ArtifactEntry`, and all new projection types.

### Fixed

- `cargo clippy --workspace -- -D warnings` is now clean.
- `biome check` is clean on `src/` and `test/`.
- `npm run typecheck` and `npm run build` pass without errors.
- All 66 Rust core lib tests, 29 WASM tests, and 24 smoke tests pass.
- All 31 web tests pass.
- Backward-compat: old session JSON with `replacements` and `deferred` maps transparently migrates to the new `tools` map on load.
- `trim_to_budget` no longer leaves orphan tool results when dropping turns.
- `findOriginalText` in `projectionService` no longer silently skips messages without a `tool_call_id`.
- `escape_xml` properly escapes all XML special characters in artifact markers.
- `evict_oldest_if_over_limit` now correctly skips `Inline` entries.
- `max_operations` limit in Rhai engine prevents infinite loops.
- Microcompact summary text persists correctly across re-evaluations.

### Removed

- `generate_types.rs` binary — type generation is now handled by `wasm-bindgen` + `tsify`.
- `Prompted` outcome variant and `ScriptResult::Prompt` — no downstream behavior existed.
- `DropIfOld` from `ProjectionShape` — trimming is now handled by the turn-based trim boundary.
- `DeferredState` struct — subsumed by `ToolProjectionState::Deferred`.

## [0.1.1] - 2026-05-28

### Added

- Per-turn tool definitions (`turn_tools` field in `AgentState`).
- Browser agent UI with Stop & Steer support.
- AbortSignal wired into LLM fetch.
- `BROWSER_TOOLS` schemas passed to `Agent.create()`.
- `configStore` for `max_tool_result_chars` configuration.
- Multi-turn replay test script.

### Fixed

- Guard against malformed IndexedDB session state.
- Remove empty assistant divs from DOM.
- SDK correctness issues for tool execution.

## [0.1.0] - 2026-05-20

### Added

- Initial release of `pi-core` agent state machine.
- `pi-host-web` WASM bindings for browser host.
- `pi-oxide-web` React + Zustand frontend.
- Context projection with fixed strategies (`head`, `tail`, `head_tail`, `keep_full`).
- Browser tools (`get_page`, `eval_js`, `query_selector`, `click`, `type`, `console`).
- Anthropic provider integration.
- Session storage with IndexedDB.
