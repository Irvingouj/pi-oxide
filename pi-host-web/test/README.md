# pi-host-web test suite

Canonical tests for the **Agent SDK** and **bindings layer**. Framework-agnostic — no React.

## What is tested here

- `agent.test.ts` — `Agent` class lifecycle
- `bindings/` — turn loop and WASM boundary (`engine.test.ts`)
- `model.test.ts` — provider factories
- `orchestration.test.ts` — engine orchestration
- `tools.test.ts`, `browserTools.test.ts` — tool registry and packs
- `stores.test.ts`, `snapshot.test.ts` — persistence
- `event-mapper.test.ts` — raw WASM event → SDK event mapping
- `events.test.ts` — SDK event emitter

## UI tests

React hook tests live in [`web/test/react/`](../web/test/react/).

Run SDK tests: `npm test` from this directory.
