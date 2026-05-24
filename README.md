# pi-oxide

A Rust agent framework where the core state machine is runtime-free and hosts own all side effects.

## Crates

| Crate | Description | Publishes to |
|-------|-------------|-------------|
| `pi-core` | Pure synchronous agent state machine. No async, no I/O. | [crates.io](https://crates.io/crates/pi-core) |
| `pi-llm` | LLM provider protocol definitions. Pure types, no network. | [crates.io](https://crates.io/crates/pi-llm) |
| `pi-host-web` | WASM host — browser fetch, events, storage | npm |
| `pi-host-desktop` | Terminal host — ratatui TUI, reqwest, local tools | binary |
| `pi-bindings` | Stable C ABI (JSON wire protocol) | source build |
| `pi-host-mobile` | Mobile host scaffold (iOS/Android via pi-bindings) | source build |

## Quick Start

```rust
use pi_core::{Agent, AgentOptions, AgentAction, AgentMessage, Model};

let model = Model {
    id: pi_core::ModelId::new("claude-sonnet-4-20250514"),
    name: pi_core::ModelName::new("claude-sonnet-4-20250514"),
    api: pi_core::ApiName::new("anthropic"),
    provider: pi_core::ProviderName::new("anthropic"),
    base_url: None,
    reasoning: false,
    context_window: 200_000,
    max_tokens: 8_192,
    capabilities: Default::default(),
    cost: Default::default(),
};

let agent = Agent::new(AgentOptions {
    system_prompt: "You are a helpful assistant.".into(),
    model,
    tools: vec![],
    thinking_level: pi_core::ThinkingLevel::Off,
    steering_mode: pi_core::QueueMode::OneAtATime,
    follow_up_mode: pi_core::QueueMode::OneAtATime,
    tool_execution_mode: pi_core::ToolExecutionMode::Parallel,
    session_id: None,
    messages: vec![],
});

let (_events, actions) = agent.start_turn(AgentMessage::user("Hello"));
// actions: [StreamLlm { context }] — host calls LLM, then agent.on_llm_done(result)
```

## Architecture

```
Host (async, platform-specific)
  owns: HTTP, filesystem, UI, browser APIs
  drives: event loop, calls into core
      |
      | typed messages
      v
pi-core (sync, runtime-free)
  owns: agent state machine, context projection, tool routing
  emits: AgentAction (StreamLlm, ExecuteTools, Finished, ...)
```

The host calls `agent.start_turn()`, gets actions, performs I/O, then feeds results back via `agent.on_llm_done()` and `agent.on_tool_done()`. Core never touches network, filesystem, or async runtime.

## Context Projection

pi-core includes a portable context projection engine that manages context window budgets:

- Token estimation with API usage calibration
- Artifact budgeting (replaces large tool results with previews)
- Microcompaction (summarizes old tool results)
- Soft/hard threshold compaction signals
- Cache breakpoint hints for prompt caching

## License

MIT
