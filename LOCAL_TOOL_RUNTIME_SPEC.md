# Local Tool Runtime Spec

This document records runtime requirements for local coding-agent tools.

The current local tools are enough for smoke tests, but they are not the final runtime model for a serious coding agent.

## Current Limitations

The current local host tool execution is intentionally simple:

- `bash` returns buffered stdout/stderr after the process exits.
- The model cannot see partial command output while a command is running.
- Background processes are not tracked as first-class jobs.
- Timeout sends termination through the current Node execution path, but there is no explicit graceful cancellation protocol.
- Tool execution is effectively synchronous from the host loop's point of view; each `onToolDone` is fed only after the tool completes.

These limitations are acceptable for Milestone 6 smoke testing. They are not acceptable for long-running real agent work.

## Required Future Runtime Capabilities

### Streaming Output

`bash` must eventually stream stdout and stderr as tool execution updates.

Target behavior:

- host emits partial stdout/stderr events while the process runs
- Rust receives or forwards `ToolExecutionUpdate` events
- UI can display live command output
- model can eventually decide whether to continue, wait, or abort based on partial output

First implementation may stream only to trace/UI. Feeding partial output back into the model can come later.

### Background Process Lifecycle

Background processes must become explicit host-managed jobs.

Target behavior:

- starting a server returns a job/process ID
- jobs are listed in session state or host runtime state
- jobs can be stopped by ID
- cleanup happens on agent shutdown, test failure, and process exit
- no orphaned long-running processes after smoke/eval runs

Do not rely on shell `$!` across separate tool calls. Each tool call may run in a different shell context.

### Signal and Abort Support

Tool execution needs an explicit cancellation path.

Target behavior:

- user stop can abort in-flight LLM and tools
- timeout first attempts graceful termination
- host escalates to force kill after a grace period
- process-tree termination is supported for shell commands that spawn children
- cancellation results are typed and observable

### Async Tool Execution

The host loop should not block while a tool runs.

Target behavior:

- tool calls return pending job handles internally
- host can process steering/user stop while tools are running
- parallel tool calls can execute concurrently when allowed by tool metadata
- sequential file mutations remain serialized per path
- `onToolDone` is called when each tool result is ready

Rust core can remain synchronous. The host owns async execution and feeds completed events back through the synchronous API.

## Boundary

Rust core owns:

- typed action/event protocol
- tool call/result correlation
- tool execution state transitions
- cancellation/result types if runtime-neutral

Host owns:

- process spawning
- stdout/stderr streaming
- process groups
- signal delivery
- job table
- runtime-specific cleanup

## Non-Goals For Current Milestone

- Do not implement a process supervisor yet.
- Do not add a UI.
- Do not add background job persistence yet.
- Do not put Node, shell, filesystem, or signal handling in `pi-core`.

This spec exists so the current smoke-test implementation does not get mistaken for the final local tool runtime.
