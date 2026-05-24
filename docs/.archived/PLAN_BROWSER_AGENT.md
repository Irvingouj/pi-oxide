# Plan: Browser Agent — End to End

## End Goal

打开一个 HTML 页面，在输入框里打字跟 agent 对话，agent 能看到当前页面、操作 DOM、执行 JS、读 console，并把结果告诉你。

具体来说：

1. 打开 `web/public/index.html`（或 dev server）
2. 页面顶部是一个可交互的 demo 区域（比如一个计数器、一个表单、一些按钮）
3. 页面底部是 agent 聊天界面：输入框 + 对话流
4. 输入 "页面上有什么？"，agent 调用 `browser_get_page` / `browser_query_selector`，返回页面内容
5. 输入 "点一下计数器按钮"，agent 调用 `browser_click`，页面上的计数器真的变了
6. 输入 "在输入框里输入 Alice 然后点提交"，agent 连续调用 `browser_type` + `browser_click`
7. 输入 "运行 1+1"，agent 调用 `browser_eval_js`，返回 2
8. 所有对话、tool call、tool result 在 UI 里实时可见
9. 使用真实 Anthropic API（浏览器直接调用，`anthropic-dangerous-direct-browser-access` header）
10. 使用现有 Rust WASM agent lifecycle + context projection

## 不做的事

- 不做 IndexedDB 持久化（第一版刷新就丢，可以接受）
- 不做用户认证/API key 管理（写死环境变量或页面输入）
- 不做生产部署（本地 dev server 够用）
- 不做移动端适配
- 不做截图/视觉能力
- 不做沙箱隔离（eval_js 直接跑在页面上下文）

## 实现步骤

### Step 1: LiveBrowserRuntime

文件：`web/src/browser/liveBrowserRuntime.ts`

用 `window` / `document` 实现 `BrowserRuntime` 接口。

```ts
class LiveBrowserRuntime implements BrowserRuntime {
  getPage() → { url: location.href, title: document.title, readyState: document.readyState, ... }
  evalJs(source) → eval(source) 或 new Function(source)()
  querySelector(sel) → 读 DOM，返回 BrowserElementSnapshot
  querySelectorAll(sel) → 同上，返回数组
  click(sel) → element.click()
  type(sel, text) → element.value = text + dispatch input event
  getConsole() → 返回 captured console entries
}
```

需要 console capture：在 runtime 初始化时 patch console.log/warn/error/info，收集 entries。

验证：手动在浏览器 console 里 import 后调用，确认能读页面。

### Step 2: 浏览器 Anthropic Provider

文件：`web/src/providers/anthropicBrowser.ts`

现有 `callAnthropic` 是给 Node 用的。浏览器版本的区别：

- 加 header `anthropic-dangerous-direct-browser-access: true`
- 用浏览器原生 `fetch`
- API key 从页面输入或 URL param 获取（不从 env）
- 处理 SSE streaming（ReadableStream reader）

```ts
async function callAnthropicBrowser(request, { apiKey, model }) {
  const response = await fetch("https://api.anthropic.com/v1/messages", {
    method: "POST",
    headers: {
      "content-type": "application/json",
      "x-api-key": apiKey,
      "anthropic-version": "2023-06-01",
      "anthropic-dangerous-direct-browser-access": "true",
    },
    body: JSON.stringify(...),
  });
  // parse SSE stream
}
```

验证：在浏览器 console 里调用，确认能拿到 LLM response。

### Step 3: 浏览器 WASM 加载

文件：`web/public/wasm.js` 或 inline in `index.html`

需要：
- 加载 WASM module（已有的 pi-host-web build output）
- 初始化 rawBinding
- 暴露给后续 JS 使用

现有构建已经产出 WASM 文件，需要确认浏览器加载方式。

验证：浏览器 console 里能调用 `createAgent` 等 WASM 函数。

### Step 4: 最小 UI — HTML 页面

文件：`web/public/index.html`

单文件，包含：

**Demo 区域（agent 操作的目标）：**
- 一个计数器（按钮 + 显示）
- 一个表单（name input + email input + submit）
- 一些静态内容（标题、段落、列表）
- Console 输出区域

**Agent 聊天区域：**
- API key 输入框（顶部，一次输入）
- 对话流容器（滚动，显示 user/assistant/tool messages）
- 输入框 + 发送按钮

**样式：**
- 简洁，深色主题
- tool call 和 tool result 用不同样式区分
- agent 正在思考时显示 loading 状态

不需要 React/Vue，纯 HTML + CSS + JS。

验证：打开页面，能看到 demo 区域和聊天区域。

### Step 5: 串联 — Agent Loop 在浏览器里跑通

文件：`web/public/agent.js` 或 inline

把前面所有东西连起来：

```text
用户输入 prompt
→ JS 调用 WASM createAgent + prompt
→ WASM 返回 stream_llm action
→ JS 调用 Anthropic API（浏览器 fetch + SSE）
→ 拿到 response，feed 进 WASM
→ WASM 返回 execute_tools action（browser_xxx）
→ JS 通过 LiveBrowserRuntime 执行工具
→ 结果 feed 进 WASM
→ 循环直到 finished
→ UI 更新显示每一步
```

验证（这是最终验收）：
1. 打开页面，输入 API key
2. 输入 "页面上有什么？" → agent 返回页面描述
3. 输入 "点一下计数器" → 计数器 +1，agent 确认
4. 输入 "在 name 输入框里输入 Bob" → 输入框出现 Bob
5. 输入 "计算 2**10" → agent 返回 1024
6. 输入 "看看 console 有什么" → agent 返回 console 内容

## 验收标准

能完成上面 6 个验证场景 = done。

## 文件清单

| 文件 | 类型 | 说明 |
|------|------|------|
| `web/src/browser/liveBrowserRuntime.ts` | 新增 | 真实 DOM runtime |
| `web/src/browser/consoleCapture.ts` | 新增 | Console 拦截收集 |
| `web/src/providers/anthropicBrowser.ts` | 新增 | 浏览器 Anthropic fetch |
| `web/public/index.html` | 新增 | 完整页面（HTML+CSS+JS） |
| `web/public/agent.js` | 新增 | Agent loop 连接逻辑 |

不需要修改 Rust 代码。不需要新增 npm 依赖。

## 依赖关系

```
Step 1 (LiveBrowserRuntime)  ──┐
Step 2 (Anthropic Browser)   ──┼── Step 5 (串联)
Step 3 (WASM 加载)           ──┤
Step 4 (UI HTML)             ──┘
```

Step 1-4 可以并行。Step 5 必须等前四个完成。

## 工作量估计

- Step 1: ~100 行 TS
- Step 2: ~150 行 TS（主要在 SSE parsing）
- Step 3: ~30 行 JS
- Step 4: ~200 行 HTML+CSS
- Step 5: ~150 行 JS

总计 ~630 行新代码。
