# CLAUDE.md

Project and behavioral guidelines for agents working in this repository.

## Project Boundaries

1. Type safety protects the program core.
   - Parse information at the first Rust boundary.
   - Do not pass unstructured strings deeper than necessary.
   - Even when the wire format is a string, wrap parsed values in concrete domain structs.
   - Prefer typed APIs over ad hoc `serde_json::Value` plumbing inside core logic.

2. `pi-core` must not assume any runtime.
   - No Tokio, browser, mobile, shell, filesystem, HTTP, or OS-specific assumptions in core.
   - Core is built around traits and synchronous state transitions.
   - Runtime-specific behavior belongs in host crates or bindings.

3. First-level platform support is web.
   - The next host is a web-based, JavaScript-driven coding agent.
   - Rust owns the typed agent core and coding-agent domain contracts.
   - JavaScript owns browser/runtime integration: model calls, storage, UI, and tool execution.
   - Desktop is not a current milestone. Do not introduce desktop-specific assumptions to justify core APIs.

4. Errors must be useful.
   - Use `thiserror` for concrete error types.
   - Preserve actionable context in errors.
   - Avoid opaque string-only failures once data has crossed into Rust.

5. Add tracing where it helps understanding and diagnosis.
   - Trace state transitions, host actions, boundary parsing, and recoverable failures.
   - Do not add noisy logs for obvious local assignments.

6. Abstraction is encouraged when it clarifies the domain.
   - Use abstractions to protect boundaries, encode invariants, and remove real duplication.
   - Avoid abstractions that only make single-use code more indirect.

7. Simplicity and elegance over spaghetti code.
   - Prefer small, coherent modules and explicit data flow.
   - Keep the core state machine readable.
   - If a change makes the control flow hard to reason about, simplify before moving on.

## 1. Think Before Coding

Don't assume. Don't hide confusion. Surface tradeoffs.

Before implementing:
- State your assumptions explicitly. If uncertain, ask.
- If multiple interpretations exist, present them - don't pick silently.
- If a simpler approach exists, say so. Push back when warranted.
- If something is unclear, stop. Name what's confusing. Ask.

## 2. Simplicity First

Minimum code that solves the problem. Nothing speculative.

- No features beyond what was asked.
- No abstractions for single-use code.
- No "flexibility" or "configurability" that wasn't requested.
- No error handling for impossible scenarios.
- If you write 200 lines and it could be 50, rewrite it.

Ask yourself: "Would a senior engineer say this is overcomplicated?" If yes, simplify.

## 3. Surgical Changes

Touch only what you must. Clean up only your own mess.

When editing existing code:
- Don't "improve" adjacent code, comments, or formatting.
- Don't refactor things that aren't broken.
- Match existing style, even if you'd do it differently.
- If you notice unrelated dead code, mention it - don't delete it.

When your changes create orphans:
- Remove imports/variables/functions that YOUR changes made unused.
- Don't remove pre-existing dead code unless asked.

The test: Every changed line should trace directly to the user's request.

## 4. Goal-Driven Execution

Define success criteria. Loop until verified.

Transform tasks into verifiable goals:
- "Add validation" -> "Write tests for invalid inputs, then make them pass"
- "Fix the bug" -> "Write a test that reproduces it, then make it pass"
- "Refactor X" -> "Ensure tests pass before and after"

For multi-step tasks, state a brief plan:

```text
1. [Step] -> verify: [check]
2. [Step] -> verify: [check]
3. [Step] -> verify: [check]
```

Strong success criteria let you loop independently. Weak criteria ("make it work") require constant clarification.

These guidelines are working if: fewer unnecessary changes in diffs, fewer rewrites due to overcomplication, and clarifying questions come before implementation rather than after mistakes.
