# pi-core

Pure synchronous agent state machine. No async runtime, no I/O, no network.

The host drives progress by calling into core via synchronous transitions and executing the `AgentAction` values core returns.

## Module map

| Module | Purpose |
|--------|---------|
| `types`, `message`, `events`, `tool`, `llm` | Typed domain model |
| `context_projection`, `session` | Portable context trimming and compaction policy |
| `agent/` | State machine logic (`streaming`, `tools`, `turn`, `queues`) |
| `agent_runtime.rs` | Typestate wrapper (`IdleAgent` → `StreamingAgent` → …) |

Start with [`src/lib.rs`](src/lib.rs) for the public API surface, then [`src/agent_runtime.rs`](src/agent_runtime.rs) for the phase-specific transition API.

## Tests

```bash
cargo test -p pi-core
```

Integration tests are split by concern under [`tests/`](tests/):

- `lifecycle.rs` — idle, start/finish, reset, serialization
- `tools_execution.rs`, `tool_preparation.rs` — tool batches and preparation
- `steering_abort.rs`, `compaction.rs`, `context_integration.rs`, `streaming.rs`
- `serde_roundtrip.rs` — message/event JSON roundtrips
- `common/mod.rs` — shared test helpers

Unit tests also live in `context_projection.rs`, `session.rs`, and `agent/mod.rs`.
