# Runtime Ownership Refactor: Eliminate Option&lt;AgentRuntime&gt;

## Goal

消除 `AgentHost.runtime: Option<AgentRuntime>` 的动态检查，让编译器保证 runtime 始终存在。

## 设计

### 核心变更

1. **`AgentRuntime::Uninitialized` 占位变体** — 用于 `std::mem::replace` 临时占位
2. **`AgentHost.runtime: AgentRuntime`** — 不再是 `Option`
3. **`AgentHost::stream_and_transition`** — 封装 streaming 完整生命周期，消除 take/set/take
4. **`AgentHost::with_runtime_mut`** — 借用式可变访问，替代 `runtime_mut()` 的外部模式匹配

### API 设计

```rust
pub enum AgentRuntime {
    Idle(IdleAgent),
    Streaming(StreamingAgent),
    Compacting(CompactingAgent),
    PreToolCall(PreToolCallAgent),
    ExecutingTools(ExecutingToolsAgent),
    ReadyToContinue(ReadyAgent),
    Finished(FinishedAgent),
    Aborted(AbortedAgent),
    Uninitialized,  // 新增：mem::replace 占位
}

pub struct AgentHost {
    runtime: AgentRuntime,  // 不再是 Option
    pub transcript: Vec<TrimmedMessage>,
    pub artifacts: Artifacts,
    pub turn_number: u32,
}

impl AgentHost {
    // 借用式可变访问
    pub fn with_runtime_mut(&mut self, f: impl FnOnce(&mut AgentRuntime));

    // 消费式转换（已有，改用 mem::replace）
    pub fn transition(&mut self, f: impl FnOnce(AgentRuntime, ...) -> TransitionParts)
        -> TransitionOutput;

    // Streaming 完整生命周期
    pub fn stream_and_transition(
        &mut self,
        terminal: &mut DefaultTerminal,
        budget: &ContextProjectionBudget,
        feed: impl FnOnce(&mut App, &mut StreamingAgent, LlmStream) -> StreamOutcome,
    ) -> TransitionOutput;
}

pub enum StreamOutcome {
    Finished(LlmResult),
    Cancelled,
}
```

### 调用点迁移

| 文件 | 原调用 | 新调用 |
|------|--------|--------|
| `llm_stream.rs` | `take_runtime()` → loop → `set_runtime()` → `transition()` | `stream_and_transition()` |
| `tool_runner.rs` | `runtime_mut()` + match | `with_runtime_mut()` |
| `commands.rs` | `take_runtime()` → `reset()` → `restore()` | `reset()` + 直接赋值 |
| `app.rs` | `agent()` / `agent_mut()` | 直接访问 `.runtime` / `.runtime_mut()` |

## 执行步骤

- [x] 新增 `AgentRuntime::Uninitialized`
- [x] `AgentHost` 去掉 `Option`，所有方法改用 `mem::replace`
- [x] 添加 `stream_and_transition` + `with_runtime_mut`
- [x] 迁移 `stream_llm`
- [x] 迁移 `tool_runner.rs`
- [x] 迁移 `commands.rs`
- [x] 迁移 `app.rs` 其余调用点
- [x] 清理测试 fixture
- [x] 编译 + 测试

## 结果

- `cargo build --workspace` ✅ 零警告
- `cargo test --workspace` ✅ 230/231（1 个 pre-existing failure：`tab_cycles_suggestions`）
- `Option<AgentRuntime>` 已完全消除
- `take_runtime()` / `set_runtime()` 已标记 deprecated 且无外部调用
- 所有 `.expect("agent")` 已消除
