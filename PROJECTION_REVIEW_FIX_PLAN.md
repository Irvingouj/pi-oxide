# Projection Review + Fix Plan

## Current Review

The projection refactor is moving in the right direction: tool results now stay canonical, projection happens at the LLM boundary, `ProjectionStrategy` is aligned around `shape/min_age`, and Rust/web tests are green.

The remaining gaps are mostly integration gaps, not core algorithm gaps.

## Findings

### P1: Artifact Recovery Is Not Exposed To The Agent

`web/src/services/projectionService.ts` now stores full replaced tool output in an in-memory `artifactStore`, and `pi-core` emits markers like:

```text
Full content should be available from host artifact: tool-result-...
```

But the agent has no callable tool to read or search that artifact. `readArtifact()` and `searchArtifacts()` are exported local functions only; they are not registered in the LLM tool map.

Impact: the projected context tells the model that full content exists, but the model cannot retrieve it. This makes the marker misleading and breaks the main value of artifact projection.

Fix:

- Add host tools such as `artifact_read` and `artifact_search`.
- Register them in the same tool registry passed to `runTurn`.
- Make the marker name the exact tool the agent should call.
- Add tests proving a replaced result can be recovered by artifact id.

### P2: `toolService.ts` Uses `ToolCall` Without Importing It

`web/src/services/toolService.ts` uses `ToolCall` in the registry callback but only imports `ToolMap`.

Impact: `npm test` and `vite build` pass because they do not typecheck. A real `tsc --noEmit` would catch this.

Fix:

- Import `ToolCall` from `@pi-oxide/pi-host-web`.
- Add a TypeScript typecheck script to `web/package.json`.
- Include that script in local/CI verification.

### P2: Projection Service Re-Defines Weak Local Types

`web/src/services/projectionService.ts` hand-defines `ProjectionBudget`, `ProjectionState`, and `ProjectionResult`, then reads `report` via `Record<string, unknown>`.

Impact: this bypasses the generated `@pi-oxide/pi-host-web` types at the exact boundary where type safety matters most.

Fix:

- Import generated types from `@pi-oxide/pi-host-web`.
- Type `state`, `budget`, `result.data.report`, and replacement reads using generated interfaces.
- Remove broad `Record<string, unknown>` casts except at true unknown external boundaries.

### P3: Projection State And Artifact Store Are Global Singletons

`projectionService.ts` keeps both projection state and artifact storage in module-level variables.

Impact: this is acceptable for a single demo session, but it will leak across sessions, resets, agents, or workspaces.

Fix:

- Move projection state and artifact store behind a session-scoped object.
- Clear them on agent reset/session reset.
- Key artifacts by session id plus artifact id, or store them inside the session state.

### P3: `smartExtract` Is Dead Code

`smartExtract` has been removed from the active agent path, but the function still exists in `llmService.ts`.

Impact: it can confuse future maintainers into thinking pre-projection summarization is still part of the design.

Fix:

- Delete `smartExtract` if no longer intended.
- If it remains as a future feature, document that it must not replace canonical tool result content before projection.

## Fix Order

1. Close the artifact loop.
   - Implement `artifact_read` and `artifact_search`.
   - Register them as agent tools.
   - Update marker text to reference those tool names.
   - Add recovery tests.

2. Add real TypeScript checking.
   - Fix missing imports.
   - Add `tsconfig.json`.
   - Add `npm run typecheck`.
   - Make `npm test` or CI run typecheck.

3. Replace weak projection service types.
   - Use generated pi-host-web types.
   - Remove avoidable `unknown` casts.

4. Scope projection state.
   - Bind state/artifacts to a session object.
   - Reset state when the agent/session resets.

5. Remove or quarantine dead summarization code.
   - Delete `smartExtract`, or document it as unused and unsafe before projection.

## Required Verification

Run:

```bash
cargo test -p pi-core -p pi-host-web
cd web && npm test
cd web && npm run build
cd web && npm run typecheck
```

Add at least one test for each behavior:

- Oversized tool result is projected into a marker.
- Original full text is stored as an artifact.
- Agent-visible `artifact_read` returns the original full text.
- Agent-visible `artifact_search` can find text inside the stored artifact.
- Bad projection metadata fails visibly or falls back in a tested way.

## Acceptance Criteria

- The canonical transcript keeps full tool results until projection.
- Projection may replace LLM-visible context, but never destroys recoverability.
- Every artifact marker has a working agent-callable recovery path.
- Generated Rust/WASM TypeScript types are the source of truth at the web boundary.
- Web verification includes typecheck, not only runtime tests and Vite build.
