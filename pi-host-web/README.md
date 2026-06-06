# @pi-oxide/pi-host-web

WASM host for [pi-core](../pi-core) with a three-layer JavaScript API.

## Architecture

```
Layer 1 — Raw WASM          @pi-oxide/pi-host-web/raw
Layer 2 — Bindings          @pi-oxide/pi-host-web/bindings
Layer 3 — High-level SDK    @pi-oxide/pi-host-web
```

| Layer | Import | Use when |
|-------|--------|----------|
| **Raw WASM** | `@pi-oxide/pi-host-web/raw` | You own the event loop and want direct access to `createHostAgent`, `startTurn`, `hostLlmDone`, etc. |
| **Bindings** | `@pi-oxide/pi-host-web/bindings` | You want `HostAgent`, directive turn loop, and tool-preparation hooks without the `Agent` class |
| **SDK** | `@pi-oxide/pi-host-web` | You want `Agent.run()`, providers, tools, and stores — framework-agnostic |

Every WASM function returns a typed result envelope (`{ ok, data?, error? }`) — never throws.

## Quick start (SDK)

```typescript
import { Agent, defineModel, memoryStore, ensureInit } from "@pi-oxide/pi-host-web";

await ensureInit();

const agent = new Agent({
  sessionId: "demo",
  instructions: "You are a helpful assistant.",
  model: defineModel({
    id: "mock",
    generate: async () => ({
      content: [{ type: "text", text: "Hello!" }],
      stopReason: "end",
    }),
  }),
  store: memoryStore(),
});

const result = await agent.run("Hello");
console.log(result.text);
agent.dispose();
```

## Bindings layer (advanced)

```typescript
import { ensureInit, createHostAgentInstance, runTurnWithHostAgent } from "@pi-oxide/pi-host-web/bindings";

await ensureInit();

const hostAgent = await createHostAgentInstance({
  sessionId: "demo",
  instructions: "You are helpful.",
  model: { id: "custom", generate: async () => ({ content: [], stopReason: "end" }) },
});

await runTurnWithHostAgent(hostAgent, {
  role: "user",
  content: [{ type: "text", text: "hi" }],
  timestamp: Date.now(),
}, {
  llm: { call: async (ctx) => ({ chunks: async function* () {}, result: Promise.resolve({ Ok: { /* ... */ } }) }) },
  tools: {},
});

hostAgent.destroy();
```

## Raw WASM

```typescript
import init, * as wasm from "@pi-oxide/pi-host-web/raw";

await init();

const { data } = wasm.createHostAgent({ system_prompt: "...", model: { /* ... */ } });
const step = wasm.startTurn(data.handle, { prompt: { role: "user", content: [...], timestamp: 0 }, tools: [] });
```

## Tests

- **SDK and bindings:** [`test/`](test/) — run with `npm test` in this directory
- **React UI:** [`../web/test/react/`](../web/test/react/) — run with `npm test` in `web/`

## Package exports

```json
{
  ".": "./dist/index.js",
  "./bindings": "./dist/sdk/bindings/index.js",
  "./raw": "./pi_host_web.js"
}
```

## License

MIT
