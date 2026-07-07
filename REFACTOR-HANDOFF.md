# Handoff: AgentHost Runtime Ownership Refactor

## Context

TUI 在用户发送第一条消息后 crash：`panic: agent runtime not set`。

## Bug 已修复

**文件:** `pi-host-tui/src/tui/llm_stream.rs`

`stream_llm` 的 `Ok` 分支里，`take_runtime()` 把 runtime 从 host 取出后，chunk loop 结束时没有 `set_runtime()` 就跳到了 post-loop 的 `transition()`，后者又调 `take_runtime()` → panic。

修复：在 chunk loop 结束后加了 `set_runtime(streaming.into_runtime())`。

## 所有权审查结果

全项目 runtime 操作点：

| 文件 | 方法 | 操作 | 状态 |
|------|------|------|------|
| `llm_stream.rs` | `stream_llm` | `take_runtime()` → loop → `set_runtime()` → `transition()` | ✅ 已修复 |
| `app.rs` | `submit_prompt` | 仅 `transition()` | ✅ |
| `app.rs` | `handle_actions` | 仅 `transition()` | ✅ |
| `app.rs` | `handle_summarize` | 仅 `transition()` | ✅ |
| `tool_runner.rs` | `execute_tools` | 仅 `runtime_mut()` | ✅ |
| `tool_runner.rs` | `on_tool_result` | 仅 `transition()` | ✅ |
| `tool_runner.rs` | `poll_running_tasks` | `runtime_mut()` + `transition()` | ✅ |
| `tool_runner.rs` | `auto_continue` | 仅 `transition()` | ✅ |
| `commands.rs` | `/session load` | `take_runtime()` → `reset()` → `restore()` 立刻回填 | ✅ |
| `agent_host.rs` | `reset()` | `take_runtime()` → `set_runtime()` 成对 | ✅ |

## 根本问题

`transition()` 内部 `take()` + `expect()` 是运行时检查，编译期保不住所有权：

```rust
pub fn transition(&mut self, f: impl FnOnce(AgentRuntime, ...) -> ...) {
    let runtime = self.runtime.take().expect("agent runtime not set"); // ← 运行时 panic
    let parts = f(runtime, ...);
    self.runtime = Some(parts.runtime);
    ...
}
```

`stream_llm` 的 `take_runtime()` → `set_runtime()` → `transition()` 模式就是这种设计缺陷的直接产物：必须先放回去才能被 `transition()` 取。

## Refactor 方案

**目标:** 消除 `Option<AgentRuntime>` 的动态检查，让编译器保证 runtime 始终存在。

### 方案 A: `transition_with` — 接受已取出的 runtime（最小改动）

```rust
impl AgentHost {
    // 新增：接受外部传入的 runtime，不再内部 take()
    pub fn transition_with(
        &mut self,
        runtime: AgentRuntime,
        f: impl FnOnce(AgentRuntime, Vec<TrimmedMessage>, Artifacts, u32) -> TransitionParts,
    ) -> TransitionOutput {
        let transcript = std::mem::take(&mut self.transcript);
        let artifacts = std::mem::take(&mut self.artifacts);
        let turn_number = self.turn_number;

        let parts = f(runtime, transcript, artifacts, turn_number);

        self.runtime = Some(parts.runtime);
        self.transcript = parts.transcript;
        self.artifacts = parts.artifacts;
        self.turn_number = parts.turn_number;

        (parts.events, parts.actions)
    }
}
```

`stream_llm` 变成：

```rust
let runtime = self.agent_host.as_mut().expect("agent").take_runtime();
let AgentRuntime::Streaming(mut streaming) = runtime else { ... };

// chunk loop ...

// 直接用 streaming 做 transition，不需要 set_runtime + 再 take
let (_events, actions) = self.agent_host.as_mut().expect("agent")
    .transition_with(streaming.into_runtime(), |runtime, transcript, artifacts, turn| {
        ...
    });
```

**优点:** 改动最小，只影响 `stream_llm` 一个调用点。
**缺点:** `Option` 还在，只是多了一条安全路径。

### 方案 B: 消除 `Option` — runtime 始终存在（推荐）

把 `AgentHost.runtime` 从 `Option<AgentRuntime>` 改成直接持有 `AgentRuntime`。

```rust
pub struct AgentHost {
    runtime: AgentRuntime,  // 不再是 Option
    pub transcript: Vec<TrimmedMessage>,
    pub artifacts: Artifacts,
    pub turn_number: u32,
}
```

`transition()` 不再 `take()`，改为在 closure 内部完成所有权转移：

```rust
pub fn transition(
    &mut self,
    f: impl FnOnce(AgentRuntime, Vec<TrimmedMessage>, Artifacts, u32) -> TransitionParts,
) -> TransitionOutput {
    let runtime = std::mem::replace(&mut self.runtime, /* 需要一个默认值? */);
    ...
}
```

或者用 `std::ptr::replace` 配合一个临时占位值。但 `AgentRuntime` 是 enum，没有 `Default`。

更好的方案：**借用变体 + 所有者分离**。让不需要移动所有权的操作使用 `&mut AgentRuntime`，只有真正需要消费的地方才 move。

### 方案 C: 分离 streaming 状态（最彻底）

`stream_llm` 需要独占 `StreamingAgent` 来 feed chunk。问题的根源是 `transition()` 和 chunk loop 都需要 runtime 的所有权。

```rust
pub struct AgentHost {
    runtime: AgentRuntime,
    // ...
}

impl AgentHost {
    // 不需要 move 的过渡
    pub fn transition(
        &mut self,
        f: impl FnOnce(&AgentRuntime) -> ... // 改为借用
    ) -> ... { ... }

    // 需要 move 的过渡
    pub fn consume_transition(
        &mut self,
        f: impl FnOnce(AgentRuntime, ...) -> TransitionParts,
    ) -> ... {
        let runtime = std::mem::replace(&mut self.runtime, UNINITIALIZED);
        let parts = f(runtime, ...);
        self.runtime = parts.runtime; // 编译期保证回填
        ...
    }
}
```

## 下一步

1. **评估方案 B vs C** — 方案 B 改动中等但能消除 `Option`；方案 C 最彻底但需要重新设计 API 边界。
2. **如果选方案 B:**
   - `AgentHost.runtime` 从 `Option<AgentRuntime>` → `AgentRuntime`
   - `transition()` 用 `std::mem::replace` + 编译期保证回填
   - 所有 `.expect("agent runtime not set")` 可以消除
   - 需要处理测试里 `App` 构造时不传 runtime 的场景
3. **如果选方案 A:**
   - 只加 `transition_with()` 方法
   - `stream_llm` 改用 `transition_with()`
   - 消除 `set_runtime()` + `take_runtime()` 的来回搬运

## 关键文件

- `pi-host-tui/src/agent_host.rs` — `AgentHost` 定义和 `transition()`
- `pi-host-tui/src/tui/llm_stream.rs` — `stream_llm()`，runtime 取出的唯一调用点
- `pi-host-tui/src/app.rs` — `submit_prompt`, `handle_actions`, `handle_summarize`
- `pi-host-tui/src/tui/tool_runner.rs` — `execute_tools`, `on_tool_result`, `poll_running_tasks`
- `pi-host-tui/src/commands.rs` — `/session load` 的 `take_runtime()` + `restore()`

## 设计原则提醒

AGENTS.md 要求：
1. **类型安全保护程序核心** — `Option` + `expect()` 违背了这个原则
2. **Rust 拥有可移植的 agent 决策** — 状态转换应该编译期安全
3. **简洁优雅优于复杂** — 不要为了灵活性引入不必要的抽象
