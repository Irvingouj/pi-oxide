# pi-oxide: Rust Agent-Core Runtime 设计文档

> 一个无异步运行时的 Rust agent-core，通过同步状态机 + Host 驱动事件循环实现跨平台绑定。Host 平台自行提供 shell、UI、tools、存储和 provider 调用。当前产品方向是 Web coding agent，但实现顺序是先跑通本机 JS host，再迁移到浏览器 host。

> 当前路线以 [ROADMAP.md](./ROADMAP.md) 为准：不做 desktop-first app shell；Rust core 除状态机外还应拥有 runtime-neutral 的 context projection policy；JS/浏览器 host 只负责运行时 I/O 和 provider 适配。

---

## 1. 设计哲学

### 1.1 核心约束

- **pi-core 零异步**：不使用 `tokio`、`async-std`、`futures`、`async-trait`。纯同步 Rust。
- **Host 驱动事件循环**：LLM 流式响应、Shell 执行、文件 I/O 等所有异步操作由 Host 层完成，完成后通过同步回调推进 core 状态机。
- **稳定跨平台绑定**：通过 C ABI（Opaque Pointer + JSON 线协议）输出，不暴露 Rust 内部类型布局。
- **平台能力注入**：core 只定义接口（trait），不实现任何平台相关逻辑。

### 1.2 为什么 Core 必须无 Async

| 问题 | 如果 Core 含 Tokio | Core 无 Async 的解决方式 |
|---|---|---|
| WASM 目标 | Tokio 不支持 WASM32 | Host（浏览器）自带事件循环 |
| 嵌入式/RTOS | Tokio 体积过大 | Host 提供最小执行环境 |
| C FFI 绑定 | `tokio::Runtime` 跨 FFI 管理复杂 | Host 自己管理运行时，Core 纯同步 |
| 多宿主共存 | Desktop (tokio) + Web (wasm) 需条件编译 | 统一同步 API，Host 各自桥接 |
| 测试确定性 | Async 测试需 runtime，时序难控 | 同步状态机，单线程可完全确定性地测试 |

---

## 2. 分层架构

```
pi-oxide/
├── pi-core/              # 纯同步 Agent 状态机（无 async，无 I/O）
│   ├── src/
│   │   ├── agent.rs      # Agent 状态机：Host 驱动，同步响应
│   │   ├── loop.rs       # Turn 编排逻辑（同步）
│   │   ├── events.rs     # AgentEvent / AgentAction enum
│   │   ├── tool.rs       # Tool trait（同步定义）
│   │   ├── message.rs    # Message / Content 类型系统
│   │   ├── session.rs    # Session 树结构（内存表示）
│   │   ├── context.rs    # AgentContext / LlmContext
│   │   ├── context_projection.rs # runtime-neutral context projection
│   │   ├── context_strategy.rs   # head/tail/keep/drop 策略
│   │   ├── context_metadata.rs   # typed tool-result metadata
│   │   └── llm.rs        # LlmProvider trait（同步定义）
│   └── Cargo.toml        # 零异步依赖
│
├── pi-bindings/          # C ABI 稳定绑定层
│   ├── src/
│   │   ├── c_api.rs      # `extern "C"` 函数
│   │   ├── types.rs      # Opaque Pointer 管理
│   │   └── bridge.rs     # JSON 序列化桥接
│   └── Cargo.toml        # 依赖 pi-core + serde_json
│
├── web/                  # 当前 JS host 工作区：本机 smoke + 后续浏览器 UI
│   ├── src/
│   │   ├── local/        # Node filesystem/bash tools
│   │   ├── providers/    # Anthropic provider adapter
│   │   ├── context/      # JS wrapper around Rust projection / compatibility
│   │   └── tools/        # tool schemas and in-memory tools
│
├── pi-host-web/          # WASM binding（Web/JS host）
│   ├── src/
│   │   ├── lib.rs        # wasm-bindgen 导出
│   │   ├── runner.rs     # JS Promise 驱动的事件循环
│   │   ├── env.rs        # Web FileSystem Access API 适配
│   │   ├── tools.rs        # 沙箱 tool 实现
│   │   └── llm.rs        # fetch() 流式适配
│   └── Cargo.toml        # wasm-bindgen, js-sys, web-sys
│
├── pi-host-mobile/       # Mobile Host（iOS/Android）
│   └── ...               # 通过 C FFI 调用 pi-bindings，各自用原生语言写 UI
│
└── pi-llm/               # LLM Provider 协议定义（纯类型，无网络实现）
    └── src/
        ├── model.rs        # Model / Provider 定义
        ├── stream.rs       # LlmEvent / LlmChunk 类型
        └── schema.rs       # JSON Schema 辅助
```

---

## 3. Core 层设计：同步状态机

### 3.1 核心交互模式

`Agent` 是一个**被动式同步状态机**。它不主动发起任何 I/O，而是响应 Host 的调用，返回 `Vec<AgentEvent>` 和 `Vec<AgentAction>`。

```
Host 事件循环（async）                          Core（同步）
┌─────────────────────┐                       ┌──────────────┐
│ 1. 用户输入 prompt  │ ──agent.start_turn()──>│ 状态机推进   │
│                     │ <──[AgentAction]───────│              │
│ 2. 看到 StreamLlm   │                       │              │
│    发起 HTTP 请求   │                       │              │
│ 3. SSE chunk 到达   │ ──agent.feed_chunk()──>│ 更新 message │
│                     │ <──[AgentEvent]────────│              │
│ 4. 响应完成         │ ──agent.on_llm_done()──>│ 检查 tool calls│
│                     │ <──[ExecuteTool]───────│              │
│ 5. 异步执行 tool    │                       │              │
│ 6. tool 完成        │ ──agent.tool_done()────>│ 状态机推进   │
│                     │ <──[AgentEvent]────────│              │
│ 7. 进入下一轮或结束 │                       │              │
└─────────────────────┘                       └──────────────┘
```

### 3.2 关键类型

#### AgentAction —— Core 请求 Host 执行的操作

```rust
pub enum AgentAction {
    /// Core 需要 Host 向 LLM 发起流式请求
    StreamLlm {
        context: LlmContext,
        session_id: Option<String>,
    },

    /// Core 需要 Host 执行一组 tool calls
    ExecuteTools {
        calls: Vec<ToolCall>,
    },

    /// Core 进入等待状态，需要 steering / follow-up 消息才能继续
    WaitForInput {
        mode: WaitMode,
    },

    /// 当前 run 完全结束
    Finished {
        messages: Vec<AgentMessage>,
    },
}

pub enum WaitMode {
    Steering,   // 可以接受 steer() 注入消息
    FollowUp,   // 可以接受 followUp() 注入消息
    Any,        // 接受任何消息
}
```

#### AgentEvent —— Core 通知 Host 的状态变化

```rust
pub enum AgentEvent {
    // Agent 生命周期
    AgentStart,
    AgentEnd { messages: Vec<AgentMessage> },

    // Turn 生命周期
    TurnStart,
    TurnEnd { message: AssistantMessage, tool_results: Vec<ToolResultMessage> },

    // Message 生命周期（流式）
    MessageStart { message: AgentMessage },
    MessageUpdate { message: AgentMessage, delta: ContentDelta },
    MessageEnd { message: AgentMessage },

    // Tool 生命周期
    ToolExecutionStart { tool_call_id: String, tool_name: String, args: Value },
    ToolExecutionUpdate { tool_call_id: String, partial_result: ToolResult },
    ToolExecutionEnd { tool_call_id: String, result: ToolResult, is_error: bool },

    // Queue 状态
    QueueUpdate { steer: Vec<AgentMessage>, follow_up: Vec<AgentMessage> },

    // Compaction / Session
    SavePoint { had_pending_writes: bool },
    Settled,
}
```

### 3.3 Agent 状态机 API

```rust
pub struct Agent {
    state: AgentState,
    steering_queue: Vec<AgentMessage>,
    follow_up_queue: Vec<AgentMessage>,
    phase: Phase,
    pending_tool_calls: HashMap<String, ToolCall>,
}

enum Phase {
    Idle,
    Streaming,         // 等待 LLM 流式数据
    ExecutingTools,    // 等待 tool 执行完成
    WaitForInput,      // 等待用户/steering/follow-up
}

impl Agent {
    /// 创建 Agent。所有依赖通过构造时注入。
    pub fn new(options: AgentOptions) -> Self;

    /// Host 调用：开始处理用户 prompt。返回 (events, actions)。
    pub fn start_turn(&mut self, prompt: AgentMessage) -> (Vec<AgentEvent>, Vec<AgentAction>);

    /// Host 调用：继续当前会话（无新 prompt）。
    pub fn continue_turn(&mut self) -> (Vec<AgentEvent>, Vec<AgentAction>);

    /// Host 调用：LLM 流式 chunk 到达。
    pub fn feed_llm_chunk(&mut self, chunk: LlmChunk) -> Vec<AgentEvent>;

    /// Host 调用：LLM 流结束（正常完成或报错）。
    pub fn on_llm_done(&mut self, result: LlmResult) -> (Vec<AgentEvent>, Vec<AgentAction>);

    /// Host 调用：单个 tool 执行完成。
    pub fn on_tool_done(
        &mut self,
        tool_call_id: String,
        result: Result<ToolResult, ToolError>,
    ) -> (Vec<AgentEvent>, Vec<AgentAction>);

    /// Host 调用：注入 steering 消息（ mid-run 干预）。
    pub fn steer(&mut self, message: AgentMessage) -> Vec<AgentEvent>;

    /// Host 调用：注入 follow-up 消息（ run 结束后继续）。
    pub fn follow_up(&mut self, message: AgentMessage);

    /// Host 调用：取消当前运行。
    pub fn abort(&mut self) -> Vec<AgentEvent>;

    /// 只读访问当前状态。
    pub fn state(&self) -> &AgentState;

    /// 重置状态（清空消息、队列、运行时状态）。
    pub fn reset(&mut self);
}
```

### 3.4 Turn 内循环逻辑（伪代码）

```rust
fn on_llm_done(&mut self, result: LlmResult) -> (Vec<AgentEvent>, Vec<AgentAction>) {
    let mut events = vec![];
    let mut actions = vec![];

    // 1. 完成 assistant message
    let assistant_msg = result.finalize_message();
    events.push(AgentEvent::MessageEnd { message: assistant_msg.clone() });
    self.state.messages.push(assistant_msg.clone());

    // 2. 检查 tool calls
    let tool_calls: Vec<_> = assistant_msg.content.iter()
        .filter_map(|c| match c { Content::ToolCall(tc) => Some(tc), _ => None })
        .cloned()
        .collect();

    if tool_calls.is_empty() {
        // 无 tool calls，turn 结束
        events.push(AgentEvent::TurnEnd { message: assistant_msg, tool_results: vec![] });

        // 检查 steering / follow-up 队列
        if let Some(pending) = self.drain_steering() {
            // 有 steering，继续 inner loop
            return self.start_inner_turn(pending, events);
        } else if let Some(follow) = self.drain_follow_up() {
            // 有 follow-up，进入 outer loop
            return self.start_outer_turn(follow, events);
        } else {
            // 完全结束
            events.push(AgentEvent::AgentEnd { messages: self.state.messages.clone() });
            actions.push(AgentAction::Finished { messages: self.state.messages.clone() });
            self.phase = Phase::Idle;
            return (events, actions);
        }
    }

    // 3. 有 tool calls，请求 Host 执行
    self.phase = Phase::ExecutingTools;
    for tc in &tool_calls {
        self.pending_tool_calls.insert(tc.id.clone(), tc.clone());
        events.push(AgentEvent::ToolExecutionStart {
            tool_call_id: tc.id.clone(),
            tool_name: tc.name.clone(),
            args: tc.arguments.clone(),
        });
    }
    actions.push(AgentAction::ExecuteTools { calls: tool_calls });

    (events, actions)
}
```

### 3.5 Tool 接口（同步定义）

Core 不执行 tool，只定义 tool 的元数据 schema。Host 执行完成后通过 `agent.on_tool_done()` 通知 core。

```rust
pub struct ToolDefinition {
    pub name: String,
    pub label: String,
    pub description: String,
    pub parameters: Value, // JSON Schema
    pub execution_mode: ExecutionMode,
    pub context_strategy: Option<ContextStrategy>,
}

pub enum ExecutionMode {
    Parallel,   // 可与其他 tool 并发执行
    Sequential, // 必须串行执行
}

/// Tool 注册表由 Host 在构造 Agent 时注入
pub struct AgentOptions {
    pub tools: Vec<ToolDefinition>,
    pub system_prompt: String,
    pub model: Model,
    pub convert_to_llm: Option<Box<dyn ConvertToLlm>>,
    pub transform_context: Option<Box<dyn TransformContext>>,
    pub thinking_level: ThinkingLevel,
    pub steering_mode: QueueMode,
    pub follow_up_mode: QueueMode,
    pub tool_execution_mode: ToolExecutionMode,
    pub session_id: Option<String>,
    pub before_tool_call: Option<Box<dyn BeforeToolCallHook>>,
    pub after_tool_call: Option<Box<dyn AfterToolCallHook>>,
}
```

### 3.6 Context Projection（同步、纯 Rust）

Context projection 是 core/domain 级策略，不是 JS host 的私有逻辑。原因是 tool result 虽然最终以文本形式喂给模型，但不同文本有不同语义：

- `read` 更适合保留 head。
- `bash` 更适合保留 tail。
- `edit`/diff 应优先 keep-full。
- `grep`/`find`/`ls` 应保留有界列表预览。

Core 不存 artifact 文件，也不做 provider-specific formatting。Core 只做 deterministic projection，并返回 report 给 host。

```rust
pub enum ContextStrategy {
    KeepFull,
    Head { max_chars: usize },
    Tail { max_chars: usize },
    HeadTail { head_chars: usize, tail_chars: usize },
    DropIfOld,
}

pub enum ContentKind {
    FileRead,
    CommandOutput,
    Diff,
    SearchResults,
    DirectoryListing,
    GenericText,
}

pub struct ToolResultContext {
    pub content_kind: ContentKind,
    pub strategy: ContextStrategy,
    pub original_chars: usize,
    pub truncated_by_tool: bool,
    pub path: Option<String>,
    pub exit_code: Option<i32>,
}

pub struct ContextProjectionBudget {
    pub max_tool_result_chars: usize,
    pub max_context_tokens: usize,
    pub default_preview_chars: usize,
}

pub struct ContextProjectionReport {
    pub estimated_tokens: usize,
    pub replacements: Vec<ContextReplacement>,
    pub dropped_messages: usize,
}
```

Projection invariants:

- canonical transcript 不被修改。
- 相同输入 + 相同 state + 相同 budget 得到 byte-identical projection。
- 不留下 orphan `tool_result`。
- artifact id 稳定，例如 `tool-result-{tool_call_id}`。
- host 根据 report 存储完整 artifact。

---

## 4. Host 层设计：异步事件循环

### 4.1 Local JS Host（Node）

当前优先 host 是本机 JS host，不是 desktop app shell。它通过 WASM binding 驱动 Rust core，并用 Node 提供真实文件和 shell 能力。

```text
AgentAction::StreamLlm
-> call WASM projectContext
-> call Anthropic provider adapter
-> feed chunks/result into Rust

AgentAction::ExecuteTools
-> execute local read/write/edit/bash in cwd
-> attach typed metadata to tool result
-> feed tool result into Rust
```

Local JS host owns:

- filesystem and shell execution
- provider HTTP calls
- permission policy
- artifact/session persistence
- trace sinks
- stdout/stderr streaming
- background process job table
- signal delivery and process cleanup

Rust owns:

- state transitions
- typed tool definitions/results
- context projection policy
- projection reports

Current limitation: the first local tool implementation is smoke-test grade. A real local coding agent needs async host-side tool execution, streaming command output, explicit background process lifecycle, and abort/signal handling. See [LOCAL_TOOL_RUNTIME_SPEC.md](./LOCAL_TOOL_RUNTIME_SPEC.md).

### 4.2 Browser Host（浏览器事件循环）

Web Host 没有 tokio，使用浏览器原生的 `Promise` 和 `fetch()`。

```rust
// pi-host-web/src/runner.rs
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use pi_core::{Agent, AgentAction, AgentEvent};

#[wasm_bindgen]
pub struct WebRunner {
    agent: Agent,
    on_event: js_sys::Function, // JS 回调
}

#[wasm_bindgen]
impl WebRunner {
    pub fn prompt(&mut self, text: String) {
        let prompt = AgentMessage::user(text);
        let (events, actions) = self.agent.start_turn(prompt);
        self.emit_js(events);

        for action in actions {
            match action {
                AgentAction::StreamLlm { context, .. } => {
                    self.spawn_fetch(context);
                }
                AgentAction::ExecuteTools { calls } => {
                    self.spawn_tool_execution(calls);
                }
                _ => {}
            }
        }
    }

    fn spawn_fetch(&self, context: LlmContext) {
        let mut agent = self.agent; // 需要内部可变性，用 RefCell 或特殊设计
        spawn_local(async move {
            let response = web_sys::window().unwrap()
                .fetch_with_str_and_init(&url, &opts)
                .dyn_into::<web_sys::Response>().unwrap();

            let reader = response.body().unwrap()
                .get_reader().dyn_into::<web_sys::ReadableStreamDefaultReader>().unwrap();

            loop {
                let chunk = js_sys::Promise::from(reader.read());
                let result = wasm_bindgen_futures::JsFuture::from(chunk).await.unwrap();
                let done = js_sys::Reflect::get(&result, &"done".into()).unwrap().as_bool().unwrap();
                if done { break; }

                let value = js_sys::Reflect::get(&result, &"value".into()).unwrap();
                let bytes = js_sys::Uint8Array::from(value).to_vec();
                let llm_chunk = parse_sse(&bytes);

                let events = agent.feed_llm_chunk(llm_chunk);
                // 通过 JS 回调 emit...
            }
        });
    }
}
```

---

## 5. 稳定 C ABI 绑定（pi-bindings）

### 5.1 绑定策略：Opaque Pointer + JSON

不暴露任何 Rust struct layout。所有参数和返回值通过 JSON 字符串传递。C 侧只持有不透明指针。

```rust
// pi-bindings/src/c_api.rs
use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::{Arc, Mutex};
use pi_core::{Agent, AgentOptions};

/// 不透明 Agent 句柄
pub struct PiAgent(Mutex<Agent>);

/// 事件回调类型
pub type PiEventCallback = extern "C" fn(event_json: *const c_char, user_data: *mut c_void);

/// 创建 Agent
/// `options_json`: JSON 字符串，序列化的 AgentOptions
/// `callback`: C 侧事件接收函数
/// `user_data`: C 侧上下文指针
/// 返回: 不透明句柄，或 NULL 如果配置错误
#[no_mangle]
pub extern "C" fn pi_agent_create(
    options_json: *const c_char,
    callback: PiEventCallback,
    user_data: *mut c_void,
) -> *mut PiAgent {
    let options_str = unsafe {
        CStr::from_ptr(options_json).to_str().expect("invalid UTF-8")
    };
    let options: AgentOptions = serde_json::from_str(options_str)
        .expect("invalid options JSON");

    let agent = Agent::new(options);
    Box::into_raw(Box::new(PiAgent(Mutex::new(agent))))
}

/// 发送 prompt
/// `agent`: pi_agent_create 返回的句柄
/// `prompt_json`: JSON 字符串，AgentMessage[] 或 { text, images? }
/// 返回: JSON 字符串 `[{ "type": "StreamLlm", ... }, ...]` 表示需要 Host 执行的动作
///       调用者负责 `free()` 返回的字符串（见 pi_free_string）
#[no_mangle]
pub extern "C" fn pi_agent_prompt(
    agent: *mut PiAgent,
    prompt_json: *const c_char,
) -> *mut c_char {
    let agent = unsafe { &*agent };
    let prompt_str = unsafe { CStr::from_ptr(prompt_json).to_str().unwrap() };
    let prompt: AgentMessage = serde_json::from_str(prompt_str).unwrap();

    let mut guard = agent.0.lock().unwrap();
    let (_events, actions) = guard.start_turn(prompt);

    let actions_json = serde_json::to_string(&actions).unwrap();
    CString::new(actions_json).unwrap().into_raw()
}

/// 注入 LLM 流式 chunk
/// `chunk_json`: JSON 字符串，LlmChunk
/// 返回: JSON 字符串，AgentEvent[]
#[no_mangle]
pub extern "C" fn pi_agent_feed_llm_chunk(
    agent: *mut PiAgent,
    chunk_json: *const c_char,
) -> *mut c_char {
    let agent = unsafe { &*agent };
    let chunk_str = unsafe { CStr::from_ptr(chunk_json).to_str().unwrap() };
    let chunk: LlmChunk = serde_json::from_str(chunk_str).unwrap();

    let mut guard = agent.0.lock().unwrap();
    let events = guard.feed_llm_chunk(chunk);

    let events_json = serde_json::to_string(&events).unwrap();
    CString::new(events_json).unwrap().into_raw()
}

/// LLM 流结束
/// `result_json`: JSON 字符串，{ "ok": true, "message": AssistantMessage }
///                或 { "ok": false, "error": "...", "aborted": false }
/// 返回: JSON 字符串，{ "events": [...], "actions": [...] }
#[no_mangle]
pub extern "C" fn pi_agent_on_llm_done(
    agent: *mut PiAgent,
    result_json: *const c_char,
) -> *mut c_char {
    let agent = unsafe { &*agent };
    let result_str = unsafe { CStr::from_ptr(result_json).to_str().unwrap() };
    let result: LlmResult = serde_json::from_str(result_str).unwrap();

    let mut guard = agent.0.lock().unwrap();
    let (events, actions) = guard.on_llm_done(result);

    let out = serde_json::json!({ "events": events, "actions": actions });
    CString::new(out.to_string()).unwrap().into_raw()
}

/// Tool 执行完成
/// `tool_call_id`: tool call ID 字符串
/// `result_json`: JSON 字符串，ToolResult 或 { "error": "..." }
/// 返回: JSON 字符串，{ "events": [...], "actions": [...] }
#[no_mangle]
pub extern "C" fn pi_agent_on_tool_done(
    agent: *mut PiAgent,
    tool_call_id: *const c_char,
    result_json: *const c_char,
) -> *mut c_char {
    let agent = unsafe { &*agent };
    let id = unsafe { CStr::from_ptr(tool_call_id).to_str().unwrap() };
    let result_str = unsafe { CStr::from_ptr(result_json).to_str().unwrap() };

    let result = if result_str.starts_with("{\"error\":") {
        let err: ToolError = serde_json::from_str(result_str).unwrap();
        Err(err)
    } else {
        let ok: ToolResult = serde_json::from_str(result_str).unwrap();
        Ok(ok)
    };

    let mut guard = agent.0.lock().unwrap();
    let (events, actions) = guard.on_tool_done(id.to_string(), result);

    let out = serde_json::json!({ "events": events, "actions": actions });
    CString::new(out.to_string()).unwrap().into_raw()
}

/// Steering / FollowUp
#[no_mangle]
pub extern "C" fn pi_agent_steer(
    agent: *mut PiAgent,
    message_json: *const c_char,
) -> *mut c_char {
    let agent = unsafe { &*agent };
    let msg: AgentMessage = serde_json::from_str(
        unsafe { CStr::from_ptr(message_json).to_str().unwrap() }
    ).unwrap();

    let mut guard = agent.0.lock().unwrap();
    let events = guard.steer(msg);
    CString::new(serde_json::to_string(&events).unwrap()).unwrap().into_raw()
}

/// 获取当前状态快照（JSON）
#[no_mangle]
pub extern "C" fn pi_agent_state(agent: *const PiAgent) -> *mut c_char {
    let agent = unsafe { &*agent };
    let guard = agent.0.lock().unwrap();
    let state = guard.state();
    CString::new(serde_json::to_string(state).unwrap()).unwrap().into_raw()
}

/// 重置
#[no_mangle]
pub extern "C" fn pi_agent_reset(agent: *mut PiAgent) {
    let agent = unsafe { &*agent };
    let mut guard = agent.0.lock().unwrap();
    guard.reset();
}

/// 释放 Agent
#[no_mangle]
pub extern "C" fn pi_agent_destroy(agent: *mut PiAgent) {
    if !agent.is_null() {
        unsafe { drop(Box::from_raw(agent)) };
    }
}

/// 释放 Rust 分配的字符串（C 侧调用完必须释放）
#[no_mangle]
pub extern "C" fn pi_free_string(s: *mut c_char) {
    if !s.is_null() {
        unsafe { drop(CString::from_raw(s)) };
    }
}
```

### 5.2 C 侧调用示例

```c
// host.c
#include "pi_oxide.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

void on_event(const char* event_json, void* user_data) {
    printf("EVENT: %s\n", event_json);
}

int main() {
    // 1. 创建 Agent
    const char* options = "{"
        "\"system_prompt\":\"You are a helpful assistant.\",""
        "\"tools\":[{\"name\":\"bash\",\"label\":\"Bash\",\"description\":\"Execute shell commands.\",""
        "  \"parameters\":{\"type\":\"object\",\"properties\":{\"command\":{\"type\":\"string\"}}},\""
        "  \"execution_mode\":\"parallel\"}],\""
        "\"model\":{\"id\":\"gpt-4o\",\"provider\":\"openai\"}\""
        "}";

    void* agent = pi_agent_create(options, on_event, NULL);
    if (!agent) {
        fprintf(stderr, "Failed to create agent\n");
        return 1;
    }

    // 2. 发送 prompt
    const char* prompt = "{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"Hello\"}]}";
    char* actions = pi_agent_prompt(agent, prompt);
    printf("ACTIONS: %s\n", actions);
    pi_free_string(actions);

    // 3. 假设 LLM 返回了一个 tool call，Host 执行后通知 core
    const char* tool_result = "{\"content\":[{\"type\":\"text\",\"text\":\"result\"}]}";
    char* response = pi_agent_on_tool_done(agent, "call_123", tool_result);
    printf("RESPONSE: %s\n", response);
    pi_free_string(response);

    // 4. 清理
    pi_agent_destroy(agent);
    return 0;
}
```

### 5.3 为什么不用 uniffi / cxx

| 方案 | 适用场景 | 本项目的取舍 |
|---|---|---|
| **C ABI + JSON** | 任何语言都能绑定，ABI 最稳定 | **选用**。牺牲微量序列化性能，换取最大兼容性 |
| `uniffi` | Mozilla 生态，自动生成 Swift/Kotlin/Python | 不选用。C 支持弱，依赖 UDL 文件 |
| `cxx` | C++ 互操作 | 不选用。仅限 C++，无法覆盖 Web/Mobile 全场景 |
| `wasm-bindgen` | Web 专用 | **Web Host 内部可用**，但不是 core 绑定层 |

---

## 6. Session / Compaction / Branch 树

### 6.1 设计原则

- **Core 只定义内存数据结构**，不实现持久化
- **Host 提供 storage trait 实现**：决定是写 JSONL 文件、IndexedDB、OPFS、SQLite、原生文件存储还是网络后端
- **Core 不做 I/O**：不导入 filesystem、IndexedDB、SQLite、网络、Node、browser 或 mobile storage API
- 序列化格式与原项目 JSONL 兼容，方便数据互通

### 6.2 内存模型

```rust
// pi-core/src/session.rs

/// SessionEntry 构成一棵树，parent_id 指向父节点
pub struct SessionEntry {
    pub id: String,
    pub parent_id: Option<String>,
    pub kind: EntryKind,
    pub timestamp: u64,
}

pub enum EntryKind {
    Message(AgentMessage),
    Compaction {
        summary: String,
        first_kept_entry_id: String,
        tokens_before: u32,
        details: Value,
    },
    BranchSummary {
        summary: String,
        details: Value,
    },
    ModelChange {
        provider: String,
        model_id: String,
    },
    ThinkingLevelChange(ThinkingLevel),
    Custom {
        custom_type: String,
        data: Value,
    },
}

/// Session 状态
pub struct SessionState {
    pub entries: Vec<SessionEntry>,
    pub leaf_id: String,        // 当前分支末端
    pub name: String,           // 会话名称
}

/// Core 提供的纯函数操作（无副作用）
impl SessionState {
    /// 获取从 root 到 leaf 的完整分支
    pub fn get_branch(&self) -> Vec<&SessionEntry>;

    /// 移动到某节点，创建 branch summary（内存操作，不持久化）
    pub fn move_to(&mut self, target_id: &str, summary: Option<BranchSummary>) -> Option<String>;

    /// 构建给 LLM 的上下文（过滤、截断）
    pub fn build_context(&self) -> Vec<AgentMessage>;
}
```

### 6.3 Host Storage 接口

```rust
// pi-core/src/session.rs

/// Host 实现此 trait 以提供持久化
pub trait SessionStorage: Send + Sync {
    fn append_entry(&mut self, entry: SessionEntry) -> Result<String, SessionError>;
    fn get_entry(&self, id: &str) -> Result<Option<SessionEntry>, SessionError>;
    fn get_branch(&self, leaf_id: &str) -> Result<Vec<SessionEntry>, SessionError>;
    fn move_to(&mut self, target_id: &str, summary: Option<BranchSummary>) -> Result<Option<String>, SessionError>;
    fn set_leaf_id(&mut self, id: &str) -> Result<(), SessionError>;
    fn get_leaf_id(&self) -> Result<String, SessionError>;
    fn append_compaction(&mut self, summary: String, first_kept: String, tokens: u32, details: Value)
        -> Result<String, SessionError>;
}

/// Host 实现此 trait 以提供大 artifact 存取。
pub trait ArtifactStorage: Send + Sync {
    fn put_artifact(&mut self, artifact: ArtifactRecord) -> Result<ArtifactRef, SessionError>;
    fn get_artifact(&self, artifact_id: &str) -> Result<Option<ArtifactRecord>, SessionError>;
}
```

Implementations are runtime-specific:

- Local JS host: JSONL session files + artifact files.
- Browser host: IndexedDB or OPFS.
- iOS host: SQLite or native file storage.
- Android host: SQLite or app storage.
- Remote/cloud host: service-backed storage.

**注意**：storage trait 是同步 core-facing contract。如果 Host 的持久化是异步的（如 IndexedDB、网络），Host runner 应在异步任务中完成 I/O，再通过同步回调更新 core 的 `SessionState` 缓存。Core 永远不选择也不执行具体 I/O。

### 6.4 Compaction 流程

1. Core 的 `prepare_compaction()` 纯函数分析消息树，决定哪些旧消息可被总结
2. Host 的 runner 拿到 `CompactionPreparation`，用异步 HTTP 请求 LLM 生成 summary
3. Host 拿到 summary 后，调用 `SessionStorage::append_compaction()` 持久化
4. Core 的 `SessionState` 更新，后续 `build_context()` 自动使用 compaction 后的精简历史

---

## 7. 实现路线图

### Phase 1: 核心类型系统（第 1-2 周）

目标：`pi-core` 编译通过，可单元测试。

- 定义 `Message`、`Content`、`AgentEvent`、`AgentAction` enum
- 定义 `ToolDefinition`、`Model`、`LlmContext`
- 实现 `AgentState` 结构
- 写同步单元测试（Mock LLM、Mock Tool）
- **零 I/O，零网络，零 async**

### Phase 2: Agent 状态机（第 2-3 周）

- 实现 `Agent::new()`、`Agent::start_turn()`、`Agent::on_llm_done()`、`Agent::on_tool_done()`
- 实现 turn 内外层循环（steering / follow-up 队列）
- 实现 `MessageQueue`（`all` vs `one-at-a-time` drain 模式）
- 实现 `abort()` 和 `reset()`
- 单元测试覆盖：无 tool turn、单 tool turn、多 tool parallel、steering mid-run、error recovery

### Phase 3: C FFI 绑定层（第 3-4 周）

- `pi-bindings` crate：所有 `extern "C"` 函数
- JSON 序列化/反序列化测试
- C 侧测试程序编译运行
- `cbindgen` 生成头文件
- 内存安全测试（valgrind / address sanitizer）

### Phase 4: Local JS Host（当前路线）

- WASM binding 驱动 Rust core
- Anthropic provider adapter
- Node `fs` 实现 read / write / edit / ls / find / grep tools
- Node child process 实现 bash tool
- Rust context projection API 生成 bounded provider context
- 端到端测试：prompt -> LLM -> tool -> Rust state -> finished

### Phase 5: Session & Compaction

- `SessionStorage` trait + JSONL 实现
- host artifact store
- manual compaction first
- LLM summary 请求后置
- 会话持久化测试

### Phase 6: Browser Host

- `wasm-bindgen` 导出 `WebRunner`
- `fetch()` 实现 LLM 请求
- 浏览器 FileSystem Access API 适配 `ExecutionEnv`
- 沙箱 tool 实现（无真实 shell）
- UI 对接 Rust actions/events 和 JS host tools

---

## 8. 与原 TypeScript 项目的核心差异

| 维度 | TypeScript `pi` | Rust `pi-oxide` |
|---|---|---|
| **运行时模型** | `Agent` 主动 async，内部 await LLM / tools | `Agent` 纯同步状态机，Host 驱动事件循环 |
| **Async Runtime** | Node.js / Bun 内置 | Core 无 async runtime，Host 自选（tokio / JS event loop / 裸机） |
| **部署形态** | npm 包，依赖 Node.js | Rust crate / `.so` / `.wasm` / 静态库，无运行时依赖 |
| **跨语言绑定** | 仅限 JS 生态 | C ABI 全覆盖：C/C++/Python/Swift/Kotlin/Go |
| **Web 支持** | 无（Node.js 独占） | 原生 WASM 目标，浏览器直接运行 |
| **嵌入式支持** | 不可能 | `no_std` 潜在可能（需替换 serde/alloc） |
| **测试确定性** | 需要 mock fetch / mock fs，时序难控 | 纯同步状态机，输入确定则输出 100% 确定 |
| **Tool 扩展** | JS 动态 import | Rust trait object + Host 注册，或 FFI 动态注册 |
| **内存模型** | GC 管理，无生命周期问题 | 显式所有权，FFI 边界用 Opaque Pointer + JSON 隔离 |
| **错误处理** | `try/catch` + 手动 Result | `Result<T, E>` 类型系统强制，FFI 边界序列化 error |
| **性能** | 解释执行，V8 优化不确定 | 零成本抽象，无 GC 停顿，LLM 等待期间 CPU 占用趋近于零 |

---

## 9. 关键设计决策记录

### 9.1 为什么 Core 不用 `async-trait` 而是纯同步

**否决的方案**：在 core 里用 `async-trait` 定义 `Tool::execute()` 和 `LlmProvider::stream()`。

**否决原因**：
- `async-trait` 依赖 `futures` + `pin-project`，增加 core 体积
- WASM32 目标下 `async-trait` 的 `Send` bound 有问题
- C FFI 无法直接暴露 async trait，必须再包一层同步桥接，增加复杂度
- 测试需要 `tokio::test`，core 失去"任意环境可运行"的优势

**采用的方案**：Host 异步执行 -> 完成后通过同步回调推进 core。

### 9.2 为什么 C FFI 用 JSON 而不是复杂 struct

**否决的方案**：用 `#[repr(C)]` struct 直接暴露给 FFI。

**否决原因**：
- Rust struct layout 不保证跨版本稳定
- C 侧需要管理 `String` / `Vec` 的内存布局，极易出错
- 新增字段会破坏 ABI

**采用的方案**：所有参数和返回值用 JSON 字符串，C 侧只需处理 `char*`。

### 9.3 为什么 Session 持久化在 Host 而非 Core

**否决的方案**：Core 内建 `tokio::fs` 或 `wasm-bindgen` 的持久化。

**否决原因**：
- 违反"core 零 I/O"原则
- 不同 Host 的持久化语义不同：Local JS host 可用 JSONL 文件，Browser host 可用 IndexedDB/OPFS，Mobile host 可用 SQLite

**采用的方案**：Core 定义 `SessionStorage` trait（同步），Host 实现并注入。

---

## 10. 结语

`pi-oxide` 的核心设计可概括为一句话：

> **Core 是一个无 I/O 的同步状态机，Host 是异步世界与 Core 之间的桥接层。**

这种架构强迫所有平台相关能力（shell、文件系统、网络、UI）从构造时注入，而不是编译时依赖。结果是：

1. **Core 可用任意方式测试**：Mock 输入 -> 同步推进 -> 断言输出
2. **Host 可独立演进**：Local JS Host 用 Node，Browser Host 用浏览器 API，Mobile Host 用原生线程，互不干扰
3. **绑定层极简稳定**：C 侧只需要 8-10 个函数，JSON 线协议天然向后兼容
4. **分发极度灵活**：单二进制 CLI、WASM 嵌入网页、iOS/Android SDK、嵌入式固件——同一套 core 代码

这份设计文档应作为项目启动时的架构契约。任何新增 feature（如多模态输入、MCP 协议支持、新的 LLM provider）都应先回答：它属于 core 的同步逻辑，还是 host 的异步实现？
