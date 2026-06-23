# Agent 架构：杯子与调酒师

## 核心原则

**Host 是杯子，Core 是调酒师。**

- Host 持有状态，负责持久化、序列化、I/O。
- Core 决定杯子里装什么。纯计算，不假设运行时环境。
- Core 不持有任何跨 turn 的状态。所有状态通过参数传入，通过 Transition 传出。

---

## 状态定义

两个状态，Host 持有。Turn 开始传给 Core，Turn 结束 Core 返回更新后的版本。

```
T = Vec<TrimmedMessage>                // 已投影，可直接发给 LLM
A = Map<EntryId, OriginalToolResult>   // 被投影替换掉的原始工具结果

完整对话 = T ∪ A                        // 没有"原始副本"
```

### T — TrimmedContext

发给 LLM 的消息序列。**已经投影过了，不会再投影。**

- `build_llm_context()` 是脑死操作：把 T 转成 LLM wire format + system_prompt + tools。完事。
- T 里的老消息已经是投影后的。Core 永远不重新投影老消息。
- 投影只发生一次：turn 结束时，Core 生成 T_n+1。

### A — Artifacts

被替换掉的原始工具结果，按 entry ID 索引。

- Core 在 T 里把一个 OriginalTool 替换成 ProjectedTool 时，原始内容存入 A。
- 要还原完整对话：T 里遇到 ProjectedTool 时，从 A 取回原始内容。
- 没有别的"原始副本"。T + A 就是全部真相。

---

## 消息类型：OriginalTool vs ProjectedTool

T 里的消息类型严格区分原始和投影后的工具结果。**不混用。**

```rust
pub enum TrimmedMessage {
    User(UserMessage),
    Assistant(AssistantMessage),
    OriginalTool(OriginalToolResult),     // 完整原始文本，还没被投影
    ProjectedTool(ProjectedToolResult),   // 预览 + artifact 引用，已被投影
    Compaction(CompactionSummary),        // 压缩摘要
}
```

### OriginalToolResult — 原始工具结果

```rust
pub struct OriginalToolResult {
    pub entry_id: String,
    pub tool_call_id: ToolCallId,
    pub tool_name: ToolName,
    pub content: Vec<Content>,          // 完整原始文本
    pub is_error: bool,
    pub turn: u32,                       // 创建时的 turn 编号（用于 defer 判断）
}
```

- 完整文本，直接发给 LLM。不需要查 A。
- 每个工具结果刚进来时都是 OriginalTool。
- 可能在后续 turn 被 Core 投影变成 ProjectedTool（defer）。

### ProjectedToolResult — 投影后的工具结果

```rust
pub struct ProjectedToolResult {
    pub entry_id: String,
    pub tool_call_id: ToolCallId,
    pub tool_name: ToolName,
    pub preview: String,                 // 截断的预览文本
    pub artifact_id: String,             // A 中的 key，存了完整原始文本
    pub original_char_count: usize,      // 原始长度（让 LLM 知道有多少被省略）
    pub is_error: bool,
}
```

- 只有预览文本 + 一个指向 A 的引用。
- 发给 LLM 时，preview 变成 `<context-artifact id="...">预览文本</context-artifact>` 格式。
- 完整文本在 A 里，通过 artifact_id 查找。
- **一旦变成 ProjectedTool，永远不会再变回 OriginalTool。**

### 状态转换图

```
工具结果进入 T:
  Host 执行工具 → Core 收到原始结果
                  │
                  ▼
            OriginalToolResult        ← 完整文本，直接在 T 里
            entry_id: "e-42"
            content: "fn main()..."   (5000字符)
            turn: 3
                  │
                  │
   ┌──────────────┴──────────────┐
   │                              │
   │ Turn 结束时立即投影            │ Turn 结束时不投影（defer）
   │ (结果太大，必须现在裁)         │ (结果不大，或者还太新)
   │                              │
   ▼                              ▼
  ProjectedToolResult          仍然是 OriginalToolResult
  entry_id: "e-42"             entry_id: "e-42"
  preview: "fn main()..."      content: "fn main()..."   (还是5000字符)
  artifact_id: "e-42"          turn: 3                   (记录是第几轮的)
  original_char_count: 5000
                                  │
                                  │ 几个 turn 之后...
                                  │ Core 检查：turn 3 的结果，
                                  │ 现在到 turn 6 了，够老了
                                  │
                                  ▼
                              ProjectedToolResult
                              (和左边一样)
```

**关键：OriginalTool 只能单向变成 ProjectedTool。不可逆。**

---

## 跨 Turn 投影（Defer）

投影不是只针对新结果。老的 OriginalTool 也可能在后续 turn 被投影。

### 为什么需要 defer？

- 工具结果刚进来时，LLM 需要看完整内容来继续推理。
- 过了几个 turn 后，这个结果变"老"了，LLM 不需要看完整内容。
- 此时才裁剪，保留上下文空间给新内容。

### Defer 规则

Core 在每个 turn 结束时做投影扫描：

```
对 T 中每个 OriginalTool:
  ┌─────────────────────────────────────────┐
  │ 年龄 = current_turn - tool.turn          │
  │ 大小 = tool.content 字符数               │
  │ 策略 = projection_strategy(tool.name)    │
  │                                         │
  │ 策略 = KeepFull → 永远不投影              │
  │ 策略 = Head(age_threshold, max_chars):   │
  │   如果 年龄 >= age_threshold              │
  │     且 大小 > max_chars                   │
  │   → 投影：OriginalTool → ProjectedTool   │
  │   否则 → 保持 OriginalTool               │
  │                                         │
  │ 策略 = Defer(N):                         │
  │   等到 年龄 >= N 再评估                   │
  └─────────────────────────────────────────┘
```

### Defer 示例（ASCII 时间线）

```
Turn 3: read("foo.rs") → 5000字符
  策略: Head(age=2, max=2000)
  年龄 = 0 (当前 turn)，不够老
  → 保持 OriginalTool，不投影
  T = [..., OriginalTool("fn main()..." 5000字符, turn=3)]

Turn 4: 用户问别的事
  投影扫描：turn=3 的结果，年龄=1，不够老(age<2)
  → 保持 OriginalTool
  T = [..., OriginalTool("fn main()..." 5000字符, turn=3), ...]

Turn 5: 用户继续问
  投影扫描：turn=3 的结果，年龄=2，够老了！且 5000>2000
  → 投影！
  OriginalTool → ProjectedTool
  T = [..., ProjectedTool(preview="fn main()..."(前200字符),
                           artifact_id="e-42",
                           turn=3),
       ...]
  A = { "e-42": OriginalToolResult("fn main()..." 5000字符) }
```

### 策略按工具名分

```
┌────────────┬──────────────────────────────────────────────┐
│ 工具名     │ 策略                                         │
├────────────┼──────────────────────────────────────────────┤
│ read       │ Head(age=2, max=2000) — 2轮后裁到2000字符    │
│ grep       │ Head(age=1, max=3000) — 1轮后裁到3000字符    │
│ edit       │ KeepFull — 永远不裁，LLM 需要看完整编辑       │
│ write      │ KeepFull — 永远不裁                           │
│ bash       │ Head(age=1, max=5000) — 1轮后裁              │
│ 默认       │ Head(age=2, max=2000)                         │
└────────────┴──────────────────────────────────────────────┘
```

---

## Turn 协议（ASCII 图解）

### 总览

```
  Host (杯子)                              Core (调酒师)
  ┌──────────────┐                        ┌──────────────┐
  │  T_n, A_n    │                        │              │
  │  sys_prompt  │                        │  纯计算，     │
  │  tools       │                        │  无状态       │
  └──────┬───────┘                        └──────┬───────┘
         │                                       │
         │  ① start_turn(prompt, tools)           │
         │ ───────────────────────────────────►  │
         │                                       │
         │                     ② 内部循环：       │
         │                        T → context     │
         │                        StreamLlm      │
         │                        ExecuteTools    │
         │                        (可能多轮)      │
         │                                       │
         │  ③ Transition { T_n+1, A_n+1, markers }
         │  ◄─────────────────────────────────── │
         │                                       │
  ┌──────┴───────┐                        ┌──────┴───────┐
  │  T_n+1, A_n+1│  ④ 持久化              │              │
  │  持久化到存储  │                        │              │
  └──────────────┘                        └──────────────┘
```

---

## Step-by-Step：普通 Turn（有工具调用 + 投影）

### ① Turn 开始

```
Host                          Core
│                             │
│  AgentRuntime 状态: Idle    │
│                             │
│  ┌─ Host 持有 ────────────────┐
│  │ T_n = [                     │
│  │   User("hello"),            │
│  │   Assistant("hi"),          │
│  │   OriginalTool(             │  ← 上轮的 grep 结果，还没被投影
│  │     content:"匹配:50处..."  │     (6000字符, turn=2)
│  │     turn: 2                 │
│  │   ),                        │
│  │ ]                           │
│  │ A_n = {}                    │
│  │ sys = "You are..."          │
│  │ tools = [read, edit, grep]  │
│  └─────────────────────────────┘
│                             │
│  用户输入: "read foo.rs"     │
│                             │
│── IdleAgent.start_turn() ──►│
│  参数:                      │
│    msg:  User("read foo.rs")│
│    tools: [read, edit, grep]│
│                             │
│                   Core 内部: │
│                   ┌────────────────────────────────────────┐
│                   │ 1. T_n 脑死转成 LLM context            │
│                   │    - User → user message                │
│                   │    - Assistant → assistant message      │
│                   │    - OriginalTool → 完整 tool_result    │
│                   │    - ProjectedTool → 带 artifact 标记的 │
│                   │                   tool_result           │
│                   │ 2. 不需要 compact? → StreamLlm          │
│                   └────────────────────────────────────────┘
│                             │
│◄── StartTurnTransition ─────│
│    ::Streaming(Transition { │
│      actions: [             │
│        StreamLlm { context }│  ← T_n 直接拼的，脑死
│      ],                     │
│    })                       │
```

### ②-④ LLM → 工具调用 → 工具结果

（和之前一样，LLM 返回 tool_call，Host 执行，结果回 Core）

```
Host                          Core (StreamingAgent → WaitingTools → Ready)
│                             │
│  LLM: "让我读取 foo.rs"      │
│  → tool_call(read, foo.rs)  │
│                             │
│  finish_llm(tool_call) ────►│ → WaitingTools
│                             │
│  Host 执行 read("foo.rs")   │
│  结果: 5000字符              │
│                             │
│  on_tool_done(              │
│    "tc-1",                  │
│    Ok(ToolResult {          │
│      content: "fn main().." │  ← 原始 5000 字符
│    })                       │
│  ) ────────────────────────►│ → Ready
│                             │
│  continue_turn() ──────────►│ → Streaming
│                             │
│  LLM: "文件内容如下..."      │
│  → EndTurn (无更多工具调用)  │
│                             │
│  finish_llm(end_turn) ────►│
```

### ⑤ Turn 结束 — 投影扫描

```
                   Core 内部 (finish_llm, EndTurn):
                   ┌────────────────────────────────────────────────────┐
                   │                                                    │
                   │ ★ 投影扫描：处理 T 中所有 OriginalTool              │
                   │                                                    │
                   │ 当前 turn = 3                                      │
                   │                                                    │
                   │ ┌─ OriginalTool #1: grep 结果 ──────────────────┐  │
                   │ │ content: "匹配:50处..." (6000字符)             │  │
                   │ │ turn: 2                                        │  │
                   │ │ 策略: Head(age=1, max=3000)                    │  │
                   │ │ 年龄: 3 - 2 = 1，够老了(age>=1)                 │  │
                   │ │ 大小: 6000 > 3000，够大                         │  │
                   │ │ → 投影！OriginalTool → ProjectedTool           │  │
                   │ │ → 原始 6000 字符 → A["entry-grep-28"]          │  │
                   │ └────────────────────────────────────────────────┘  │
                   │                                                    │
                   │ ┌─ 新的 OriginalTool #2: read 结果 ─────────────┐  │
                   │ │ content: "fn main()..." (5000字符)             │  │
                   │ │ turn: 3 (当前 turn)                             │  │
                   │ │ 策略: Head(age=2, max=2000)                    │  │
                   │ │ 年龄: 3 - 3 = 0，不够老(age<2)                  │  │
                   │ │ → defer！保持 OriginalTool                     │  │
                   │ └────────────────────────────────────────────────┘  │
                   │                                                    │
                   │ ┌─ 检查总 token ────────────────────────────────┐  │
                   │ │ grep 被裁了，省了约 3000 字符                   │  │
                   │ │ 没超预算，不需要 microcompact/compact           │  │
                   │ └────────────────────────────────────────────────┘  │
                   │                                                    │
                   │ 生成 T_3, A_3                                      │
                   └────────────────────────────────────────────────────┘
│                             │
│◄── Finished(Transition {    │
│      markers: [             │
│        NewArtifacts {       │
│          entry_ids: [       │
│            "entry-grep-28"  │  ← grep 的原始结果
│          ]                  │
│        }                    │
│      ],                     │
│    })                       │
│                             │
│  ┌─ Host 更新状态 ──────────────────────────────┐
│  │ T_3 = [                                      │
│  │   User("hello"),                             │
│  │   Assistant("hi"),                           │
│  │   ProjectedTool(               ← 被 defer 投影了 │
│  │     preview: "匹配:50处...前200字符",          │
│  │     artifact_id: "entry-grep-28",            │
│  │     original_char_count: 6000,               │
│  │   ),                                         │
│  │   User("read foo.rs"),                       │
│  │   Assistant(tool_call: read),                │
│  │   OriginalTool(                ← 还没被投影    │
│  │     content: "fn main()..." (5000字符),      │
│  │     turn: 3,                                 │
│  │   ),                                         │
│  │   Assistant("文件内容如下..."),                │
│  │ ]                                            │
│  │ A_3 = {                                      │
│  │   "entry-grep-28": OriginalToolResult(       │
│  │     content: "匹配:50处...完整6000字符"        │
│  │   )                                          │
│  │ }                                            │
│  └──────────────────────────────────────────────┘
│                             │
│  Host 持久化 T_3, A_3        │
```

### ⑥ 下一轮：defer 的 read 结果被投影

```
Turn 4 开始:
  投影扫描:
    OriginalTool(read, turn=3, 5000字符)
    策略: Head(age=2, max=2000)
    年龄: 4 - 3 = 1，还是不够老(age<2)
    → 继续保持 OriginalTool

Turn 5 开始:
  投影扫描:
    OriginalTool(read, turn=3, 5000字符)
    策略: Head(age=2, max=2000)
    年龄: 5 - 3 = 2，够老了！且 5000>2000
    → 投影！
    → ProjectedTool(preview=前200字符, artifact_id="entry-read-42")
    → A["entry-read-42"] = 完整5000字符
```

---

## Step-by-Step：Compaction Turn

```
Host                          Core
│                             │
│  ┌─ Host 持有 ──────────────────┐
│  │ T_n = [很长的消息，           │  ← 估计 token 超过 threshold
│  │   ProjectedTool(...),        │
│  │   ProjectedTool(...),        │
│  │   OriginalTool(...),         │
│  │   ...10轮对话...]            │
│  │ A_n = {很多 artifacts}       │
│  └──────────────────────────────┘
│                             │
│── IdleAgent.start_turn() ──►│
│                             │
│                   Core 内部: │
│                   ┌──────────────────────────────────────┐
│                   │ 1. T_n → context                      │
│                   │ 2. 估算 token                         │
│                   │ 3. token > threshold × max_tokens?   │
│                   │    是 → 需要 compact                   │
│                   │ 4. 从 T_n 选出要压缩的老消息           │
│                   │ 5. 构建 Summarize action               │
│                   └──────────────────────────────────────┘
│                             │
│◄── Compacting(Transition {  │
│      actions: [             │
│        Summarize {          │
│          context: LlmContext│  ← compaction_prompt + 老消息
│          plan: CompactionPlan│
│        }                    │
│      ],                     │
│    })                       │
│                             │
│  Host 调用 LLM              │
│  summary = "用户讨论了..."   │
│                             │
│── accept_summary(summary) ─►│
│                             │
│                   Core 内部: │
│                   ┌──────────────────────────────────────────────┐
│                   │ 1. 把老消息从 T 中移除                        │
│                   │    老的 ProjectedTool → 已在 A 里，不用管     │
│                   │    老的 OriginalTool → 原始文本 → A           │
│                   │ 2. 在 T 前面插入一条 Compaction 摘要          │
│                   │    Compaction("用户讨论了...")                │
│                   │ 3. 清理 A 中不再被 T 引用的 artifact_id      │
│                   │ 4. 对剩余 T 做 投影扫描（老 OriginalTool     │
│                   │    可能因为移除了前面的消息而变"更显眼"）     │
│                   │ 5. 重新组装 context → StreamLlm              │
│                   └──────────────────────────────────────────────┘
│                             │
│◄── Streaming(Transition {   │
│      markers: [             │
│        NewArtifacts {       │
│          entry_ids: [...]   │  ← 被压缩的老 OriginalTool IDs
│        },                   │
│        CompactionApplied,   │
│      ],                     │
│    })                       │
```

---

## Typestate 状态机

```
                    start_turn()
         ┌──────────────────────────────┐
         │                              │
         │  不需要 compact              │  需要 compact
         ▼                              ▼
     StreamingAgent              CompactingAgent
         │                              │
         │ finish_llm()                 │ accept_summary()
         │                              │
         ├─ 有 tool_calls ──► WaitingToolsAgent
         │                         │
         │                    on_tool_done()
         │                         │
         │                    ├─ 还有 pending → WaitingTools
         │                    └─ 全部 done → Ready
         │                              │
         │                    continue_turn()
         │                         │
         ├─ EndTurn ──► ★ 投影扫描 ──► FinishedAgent
         │
         ├─ 有 steering/follow-up → Streaming
         │
         └─ 默认 → Ready

    ★ 投影扫描在 EndTurn 时执行：
       - 新 OriginalTool → 立即投影 or defer
       - 老 OriginalTool → 检查是否够老，够老就投影
       - 检查总 token → 可能触发 microcompact

    Finished.into_idle() ──► Idle
    Aborted.into_idle() ──► Idle
    *.abort() ──► Aborted
```

---

## 结构体定义

### TrimmedMessage — T 中的消息类型

```rust
pub enum TrimmedMessage {
    User(UserMessage),
    Assistant(AssistantMessage),
    /// 完整原始工具结果。还没被投影。
    /// 可能在后续 turn 被投影变成 ProjectedTool。
    OriginalTool(OriginalToolResult),
    /// 投影后的工具结果。只有预览 + artifact 引用。
    /// 一旦变成 ProjectedTool，永远不变回 OriginalTool。
    ProjectedTool(ProjectedToolResult),
    /// 压缩摘要。替代被压缩掉的老消息。
    Compaction(CompactionSummary),
}

pub struct OriginalToolResult {
    pub entry_id: String,
    pub tool_call_id: ToolCallId,
    pub tool_name: ToolName,
    pub content: Vec<Content>,
    pub is_error: bool,
    pub turn: u32,
}

pub struct ProjectedToolResult {
    pub entry_id: String,
    pub tool_call_id: ToolCallId,
    pub tool_name: ToolName,
    pub preview: String,
    pub artifact_id: String,
    pub original_char_count: usize,
    pub is_error: bool,
}

pub struct CompactionSummary {
    pub summary: String,
    pub compacted_entry_ids: Vec<String>,
    pub tokens_before: usize,
}
```

### A — Artifacts

```rust
/// key = entry_id, value = 被替换掉的原始工具结果
pub type Artifacts = BTreeMap<String, OriginalToolResult>;
```

### ChangeMarker

```rust
pub enum ChangeMarker {
    /// 这些 entry 的原始工具结果已存入 A
    NewArtifacts { entry_ids: Vec<String> },
    /// Compaction 发生，T 结构大改，建议全量持久化
    CompactionApplied,
}
```

### AgentAction

```rust
pub enum AgentAction {
    StreamLlm { context: LlmContext },
    Summarize { context: LlmContext, plan: CompactionPlan },
    ExecuteTools { calls: Vec<ToolCall> },
    CancelTools { tool_call_ids: Vec<ToolCallId>, reason: CancelReason },
    WaitForInput { mode: WaitMode },
    Finished,
}
```

### Transition

```rust
pub struct Transition<T> {
    pub events: Vec<AgentEvent>,
    pub actions: Vec<AgentAction>,
    pub state: T,
    pub markers: Vec<ChangeMarker>,
}
```

### HostState

```rust
pub struct HostState {
    pub T: Vec<TrimmedMessage>,
    pub A: Artifacts,
    pub system_prompt: String,
    pub compaction_prompt: String,
    pub budget: ContextProjectionBudget,
}
```

### PersistData

```rust
pub struct PersistData {
    pub T: Vec<TrimmedMessage>,
    pub A: Vec<(String, OriginalToolResult)>,
    pub system_prompt: String,
    pub compaction_prompt: String,
    pub budget: ContextProjectionBudget,
}
```

---

## 函数签名

### IdleAgent::start_turn

```rust
impl IdleAgent {
    pub fn start_turn(
        self,
        msg: AgentMessage,
        tools: Vec<ToolDefinition>,
    ) -> StartTurnTransition
    // T 和 A 在 Agent 内部
    // T 脑死转成 context → StreamLlm
    // 如果 token 超阈值 → Summarize → CompactingAgent
}
```

### StreamingAgent::finish_llm

```rust
impl StreamingAgent {
    pub fn finish_llm(self, result: LlmResult) -> FinishLlmTransition
    // 如果 EndTurn:
    //   1. 新消息追加到 T
    //   2. 投影扫描（新 + 老 OriginalTool）
    //   3. 检查 token → microcompact?
    //   4. 返回 FinishedAgent + markers
}
```

### WaitingToolsAgent::on_tool_done

```rust
impl WaitingToolsAgent {
    pub fn on_tool_done(
        self,
        id: ToolCallId,
        result: Result<ToolResult, ToolError>,
    ) -> ToolTransition
    // 原始 ToolResult 存入内部缓冲区
    // 包装成 OriginalToolResult (turn=current_turn)
    // 不做投影，投影在 finish_llm 时统一做
}
```

### CompactingAgent::accept_summary

```rust
impl CompactingAgent {
    pub fn accept_summary(
        self,
        summary_text: String,
    ) -> Transition<StreamingAgent>
    // 1. 老 OriginalTool → 原始文本 → A
    // 2. 老 ProjectedTool → 已在 A，不动
    // 3. 插入 Compaction 摘要
    // 4. 清理 A 中失效的 entry
    // 5. 投影扫描剩余 T
    // 6. StreamLlm
}
```

---

## 投影扫描伪代码

```rust
fn projection_scan(
    T: &mut Vec<TrimmedMessage>,
    A: &mut Artifacts,
    current_turn: u32,
    budget: &ContextProjectionBudget,
) -> Vec<ChangeMarker> {
    let mut new_artifacts = vec![];

    for msg in T.iter_mut() {
        let TrimmedMessage::OriginalTool(tool) = msg else { continue };

        let age = current_turn - tool.turn;
        let strategy = projection_strategy(&tool.tool_name);
        let char_count = tool.content_char_count();

        match strategy {
            Strategy::KeepFull => {
                // 永远不投影
            }
            Strategy::Head { min_age, max_chars } => {
                if age >= min_age && char_count > max_chars {
                    // 投影！
                    let preview = tool.content.preview(max_chars);
                    let artifact_id = tool.entry_id.clone();

                    A.insert(artifact_id.clone(), tool.clone());
                    new_artifacts.push(artifact_id.clone());

                    *msg = TrimmedMessage::ProjectedTool(ProjectedToolResult {
                        entry_id: tool.entry_id.clone(),
                        tool_call_id: tool.tool_call_id.clone(),
                        tool_name: tool.tool_name.clone(),
                        preview,
                        artifact_id,
                        original_char_count: char_count,
                        is_error: tool.is_error,
                    });
                }
            }
        }
    }

    if new_artifacts.is_empty() {
        vec![]
    } else {
        vec![ChangeMarker::NewArtifacts { entry_ids: new_artifacts }]
    }
}
```

---

## 要删除的东西

| 类型 | 原因 |
|------|------|
| `ProjectionDecisions` | 不需要了。OriginalTool.turn 字段跟踪年龄，不需要外部状态 |
| `project()` 全量重投影 | T 已经是投影后的。投影扫描只处理 OriginalTool |
| `SessionState` 树结构 | T (`Vec<TrimmedMessage>`) 替代。不需要 tree/leaf_id |
| `ChangeMarker::NewEntries` | 从未使用过 |
| `ChangeMarker::NewReplacements` | 改名为 `NewArtifacts`，按 entry_id |
| 混用原始/投影 tool result | `OriginalTool` 和 `ProjectedTool` 是不同类型，编译器强制区分 |

---

## 职责划分

### Host（杯子）

- 持有 T 和 A
- 调 LLM API（流式）
- 执行工具
- 持久化 T、A、配置
- **不感知投影**。不知道 OriginalTool vs ProjectedTool 的区别。
- **不调用投影函数**。

### Core（调酒师）

- 决定新 tool result 的投影策略
- 决定何时 compact
- 修改 T（投影扫描、microcompact、compact）
- 填充 A（被替换的原始工具结果）
- 从 T 组装 LLM context（脑死：TrimmedMessage → wire format）
- 管理 typestate 状态机
