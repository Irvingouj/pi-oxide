# pi-host-tui Feature Gap

Gap analysis vs `../pi` (TypeScript reference) and `../claude-code` (industry reference).

## Must-Have (blocks productivity)

1. **More model providers** — ~~Anthropic-only is a hard ceiling.~~ OpenAI, Google, and local models at minimum. `pi` has 10+ providers with OAuth; claude-code supports all major providers. **DONE: OpenAI + Anthropic wire formats implemented (Anthropic, OpenAI, DeepSeek, DeepSeek Anthropic, OpenAI-compat, Anthropic-compat). Remaining: Google, local models.**
2. **grep/find/glob tools** — Cannot search codebases without them. Both `pi` (7 tools) and claude-code (~40 tools) include these. **DONE: grep + glob implemented with gitignore awareness. pi-oxide now has 6 tools (bash/read/write/edit/grep/glob).**
3. **Session tree (fork/branch/resume)** — Cannot explore alternatives or recover from bad paths. `pi` has JSONL-based session trees with branching and forking; claude-code has full session lifecycle management.
4. **Virtual scrolling** — Long sessions become unusable without it. ratatui does full-screen redraws. claude-code uses viewport + overscan rendering; `pi` uses differential rendering (better but still not virtual). **DONE: Two-pass virtual scroll with pure scroll helpers (derive_scroll_intent, apply_scroll, compute_scroll, visible_range), E2E tests via TestBackend, partial-entry overlap handling, and tracing.**

## Should-Have (serious UX deficit)

5. **Configurable keybindings** — Power users need custom shortcuts. Both `pi` and claude-code support this; claude-code adds hot-reload from `~/.claude/keybindings.json`.
6. **Theming** — Dark/light mode is table stakes for terminal apps. claude-code has dark/light/auto with OSC 11 detection; `pi` has theme support.
7. **Transcript search** — Cannot find past messages in long sessions. claude-code has inline `/` search with highlighting.
8. **Diff viewer** — Code changes are unreadable as raw text. claude-code has a StructuredDiff component.
9. **Cost/token tracking** — No visibility into spend. Both `pi` and claude-code show live cost, token usage, and context window %.
10. **More slash commands** — Missing: `/compact`, `/settings`, `/export`, `/review`. `pi` has 22 slash commands; claude-code has ~85. pi-oxide has 9.

## Nice-to-Have (polish)

11. Terminal hyperlinks (OSC 8) — `pi` and claude-code both support clickable links.
12. Terminal images (Kitty/iTerm2 protocol) — `pi` has full image support; claude-code supports image paste.
13. Vim mode — claude-code has full vim mode (normal/insert, motions, operators).
14. External editor integration (`Ctrl+X`) — claude-code supports launching `$EDITOR`.
15. Animated spinner — claude-code has 60fps shimmer with rotating action verbs.
16. Configurable status bar — claude-code uses a hook system for user-defined statusline elements.
17. Permission model — claude-code has approve/allow/deny for tool execution.
18. MCP integration — claude-code supports MCP servers for external tool discovery.
19. Sub-agent spawning — claude-code supports team creation and sub-agent delegation.
20. Multi-agent teams — claude-code has multi-agent orchestration.

## pi-oxide Advantages (do not lose)

- **Typestate safety** — Compile-time enforcement of agent phases. Neither `pi` nor claude-code has this.
- **WASM host** — Runs in browser. Neither competitor does this.
- **Rust core** — Zero-cost abstractions, no GC pauses. Both competitors are TypeScript.
- **Cleaner architecture** — Core/host separation is crisper than either competitor.

