//! WASM host for pi-core.
//!
//! Exposes the agent state machine through typed WASM APIs.
//! Every function returns a `ResultEnvelope<T>` — never throws.

use std::cell::Cell;
use std::cell::RefCell;

use wasm_bindgen::prelude::*;

use pi_core::{estimate_tokens, estimate_tokens_for_text, AgentRuntime, Transition};
use tracing::info;
#[allow(unused_imports)]
use tracing_subscriber::layer::SubscriberExt;
#[allow(unused_imports)]
use tracing_subscriber::util::SubscriberInitExt;

fn project_context(input: pi_core::ProjectionInput) -> pi_core::ProjectionOutput {
    pi_core::project(input)
}

mod dto;
use dto::*;

thread_local! {
    static AGENT_SLOTS: RefCell<Vec<Option<AgentRuntime>>> = const { RefCell::new(Vec::new()) };
    static TRACING_INIT: Cell<bool> = const { Cell::new(false) };
    static LOG_LEVEL: Cell<tracing::Level> = const { Cell::new(tracing::Level::INFO) };
}

fn init_tracing() {
    TRACING_INIT.with(|init| {
        if !init.get() {
            #[cfg(target_arch = "wasm32")]
            {
                let _ = tracing_subscriber::registry()
                    .with(tracing_wasm::WASMLayer::default())
                    .try_init();
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let _ = tracing_subscriber::fmt()
                    .with_max_level(tracing::Level::INFO)
                    .try_init();
            }
            init.set(true);
        }
    });
}

#[wasm_bindgen(js_name = "setLogLevel")]
pub fn set_log_level(level: String) {
    let parsed = match level.as_str() {
        "trace" => tracing::Level::TRACE,
        "debug" => tracing::Level::DEBUG,
        "info" => tracing::Level::INFO,
        "warn" => tracing::Level::WARN,
        "error" => tracing::Level::ERROR,
        _ => tracing::Level::INFO,
    };
    LOG_LEVEL.with(|c| c.set(parsed));
}

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
enum HostError {
    #[error("agent not found: handle {0} is invalid")]
    BadHandle(u32),
    #[error("wrong phase: expected {expected}, got {actual}")]
    WrongPhase {
        expected: &'static str,
        actual: &'static str,
    },
}

impl HostError {
    fn code(&self) -> &'static str {
        match self {
            HostError::BadHandle(_) => "bad_handle",
            HostError::WrongPhase { .. } => "wrong_phase",
        }
    }
    fn to_dto(&self) -> ErrorDto {
        ErrorDto {
            code: self.code().to_string(),
            message: self.to_string(),
        }
    }
}

fn ok<T: serde::Serialize, R: for<'de> serde::Deserialize<'de>>(data: T) -> R {
    let mut map = serde_json::Map::new();
    map.insert("ok".to_string(), serde_json::Value::Bool(true));
    map.insert("data".to_string(), serde_json::to_value(data).unwrap());
    map.insert("error".to_string(), serde_json::Value::Null);
    serde_json::from_value(serde_json::Value::Object(map)).unwrap()
}

fn err<R: for<'de> serde::Deserialize<'de>>(e: &HostError) -> R {
    let mut map = serde_json::Map::new();
    map.insert("ok".to_string(), serde_json::Value::Bool(false));
    map.insert("data".to_string(), serde_json::Value::Null);
    map.insert(
        "error".to_string(),
        serde_json::to_value(e.to_dto()).unwrap(),
    );
    serde_json::from_value(serde_json::Value::Object(map)).unwrap()
}

fn dto_err<R: for<'de> serde::Deserialize<'de>>(e: serde_json::Error) -> R {
    let mut map = serde_json::Map::new();
    map.insert("ok".to_string(), serde_json::Value::Bool(false));
    map.insert("data".to_string(), serde_json::Value::Null);
    map.insert(
        "error".to_string(),
        serde_json::to_value(ErrorDto {
            code: "dto_conversion".to_string(),
            message: format!("DTO conversion failed: {}", e),
        })
        .unwrap(),
    );
    serde_json::from_value(serde_json::Value::Object(map)).unwrap()
}

macro_rules! try_conv {
    ($expr:expr) => {
        match $expr {
            Ok(v) => v,
            Err(e) => return dto_err(e),
        }
    };
}

// ---------------------------------------------------------------------------
// Agent handle table
// ---------------------------------------------------------------------------

fn take_runtime(handle: u32) -> Result<AgentRuntime, HostError> {
    AGENT_SLOTS.with(|slots| {
        let mut slots = slots.borrow_mut();
        let idx = handle as usize;
        if idx >= slots.len() {
            return Err(HostError::BadHandle(handle));
        }
        slots[idx].take().ok_or(HostError::BadHandle(handle))
    })
}

fn put_runtime(runtime: AgentRuntime) -> u32 {
    AGENT_SLOTS.with(|slots| {
        let mut slots = slots.borrow_mut();
        for (i, slot) in slots.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(runtime);
                return i as u32;
            }
        }
        let handle = slots.len() as u32;
        slots.push(Some(runtime));
        handle
    })
}

fn with_runtime<T>(handle: u32, op: impl FnOnce(&mut AgentRuntime) -> T) -> Result<T, HostError> {
    AGENT_SLOTS.with(|slots| {
        let mut slots = slots.borrow_mut();
        let idx = handle as usize;
        if idx >= slots.len() {
            return Err(HostError::BadHandle(handle));
        }
        match &mut slots[idx] {
            Some(runtime) => Ok(op(runtime)),
            None => Err(HostError::BadHandle(handle)),
        }
    })
}

// ---------------------------------------------------------------------------
// WASM exports — typed DTOs, never strings
// ---------------------------------------------------------------------------

fn runtime_phase_name(runtime: &AgentRuntime) -> &'static str {
    match runtime {
        AgentRuntime::Idle(_) => "Idle",
        AgentRuntime::Streaming(_) => "Streaming",
        AgentRuntime::WaitingTools(_) => "WaitingTools",
        AgentRuntime::ReadyToContinue(_) => "ReadyToContinue",
        AgentRuntime::Finished(_) => "Finished",
        AgentRuntime::Aborted(_) => "Aborted",
    }
}

#[wasm_bindgen(js_name = "createAgent")]
pub fn create_agent(options: AgentOptions) -> CreateAgentResult {
    console_error_panic_hook::set_once();
    init_tracing();
    info!("createAgent called");
    let core_options: pi_core::AgentOptions = try_conv!(options.try_into());
    let runtime = AgentRuntime::new(core_options);
    let handle = put_runtime(runtime);
    info!(handle, "agent created");
    ok(CreateAgentOutput { handle })
}

#[wasm_bindgen(js_name = "prompt")]
pub fn prompt(handle: u32, input: PromptInput) -> StepResult {
    console_error_panic_hook::set_once();
    info!(handle, "prompt called");

    let core_prompt: pi_core::AgentMessage = match input.prompt {
        PromptRequest::Message(m) => try_conv!(m.try_into()),
        PromptRequest::Text { text } => pi_core::AgentMessage::user(text),
    };
    let core_tools: Vec<pi_core::ToolDefinition> = try_conv!(input
        .tools
        .into_iter()
        .map(|t| t.try_into())
        .collect::<Result<Vec<_>, _>>());

    let runtime = match take_runtime(handle) {
        Ok(r) => r,
        Err(e) => return err(&e),
    };

    let result = match runtime {
        AgentRuntime::Idle(idle) => {
            let Transition {
                events,
                actions,
                state,
            } = idle.start_turn(core_prompt, core_tools);
            put_runtime(state.into_runtime());
            Ok((events, actions))
        }
        AgentRuntime::Finished(finished) => {
            let idle = finished.into_idle();
            let Transition {
                events,
                actions,
                state,
            } = idle.start_turn(core_prompt, core_tools);
            put_runtime(state.into_runtime());
            Ok((events, actions))
        }
        AgentRuntime::Aborted(aborted) => {
            let idle = aborted.into_idle();
            let Transition {
                events,
                actions,
                state,
            } = idle.start_turn(core_prompt, core_tools);
            put_runtime(state.into_runtime());
            Ok((events, actions))
        }
        AgentRuntime::WaitingTools(mut waiting) => {
            let (events, actions) = waiting
                .submit_user_message(core_prompt)
                .into_events_actions();
            put_runtime(waiting.into_runtime());
            Ok((events, actions))
        }
        other => {
            let actual = runtime_phase_name(&other);
            put_runtime(other);
            Err(HostError::WrongPhase {
                expected: "Idle, Finished, Aborted, or WaitingTools",
                actual,
            })
        }
    };

    match result {
        Ok((events, actions)) => ok(StepOutput {
            events: try_conv!(events
                .into_iter()
                .map(|e| e.try_into())
                .collect::<Result<Vec<_>, _>>()),
            actions: try_conv!(actions
                .into_iter()
                .map(|a| a.try_into())
                .collect::<Result<Vec<_>, _>>()),
        }),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "feedLlmChunk")]
pub fn feed_llm_chunk(handle: u32, chunk: LlmChunk) -> EventsResult {
    console_error_panic_hook::set_once();
    info!(handle, "feedLlmChunk called");

    let core_chunk: pi_core::LlmChunk = try_conv!(chunk.try_into());
    let runtime = match take_runtime(handle) {
        Ok(r) => r,
        Err(e) => return err(&e),
    };

    let result = match runtime {
        AgentRuntime::Streaming(mut streaming) => {
            let events = streaming.feed_llm_chunk(core_chunk);
            put_runtime(streaming.into_runtime());
            Ok(events)
        }
        other => {
            let actual = runtime_phase_name(&other);
            put_runtime(other);
            Err(HostError::WrongPhase {
                expected: "Streaming",
                actual,
            })
        }
    };

    match result {
        Ok(events) => ok(EventsOutput {
            events: try_conv!(events
                .into_iter()
                .map(|e| e.try_into())
                .collect::<Result<Vec<_>, _>>()),
        }),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "onLlmDone")]
pub fn on_llm_done(handle: u32, result: LlmResult) -> StepResult {
    console_error_panic_hook::set_once();
    info!(handle, "onLlmDone called");

    let core_result: pi_core::LlmResult = try_conv!(result.try_into());
    let runtime = match take_runtime(handle) {
        Ok(r) => r,
        Err(e) => return err(&e),
    };

    let result = match runtime {
        AgentRuntime::Streaming(streaming) => {
            let (events, actions, new_runtime) = streaming.finish_llm(core_result).into_parts();
            put_runtime(new_runtime);
            Ok((events, actions))
        }
        other => {
            let actual = runtime_phase_name(&other);
            put_runtime(other);
            Err(HostError::WrongPhase {
                expected: "Streaming",
                actual,
            })
        }
    };

    match result {
        Ok((events, actions)) => ok(StepOutput {
            events: try_conv!(events
                .into_iter()
                .map(|e| e.try_into())
                .collect::<Result<Vec<_>, _>>()),
            actions: try_conv!(actions
                .into_iter()
                .map(|a| a.try_into())
                .collect::<Result<Vec<_>, _>>()),
        }),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "onToolDone")]
pub fn on_tool_done(handle: u32, tool_call_id: String, payload: ToolDonePayload) -> StepResult {
    console_error_panic_hook::set_once();
    info!(
        handle,
        tool_call_id = tool_call_id.as_str(),
        "onToolDone called"
    );

    let id = pi_core::ToolCallId::new(&tool_call_id);
    let core_result: Result<pi_core::ToolResult, pi_core::ToolError> = match payload {
        ToolDonePayload::Failure { error } => Err(try_conv!(error.try_into())),
        ToolDonePayload::Success { result } => Ok(try_conv!(result.try_into())),
    };

    let runtime = match take_runtime(handle) {
        Ok(r) => r,
        Err(e) => return err(&e),
    };

    let result = match runtime {
        AgentRuntime::WaitingTools(waiting) => {
            let (events, actions, new_runtime) = waiting.on_tool_done(id, core_result).into_parts();
            put_runtime(new_runtime);
            Ok((events, actions))
        }
        other => {
            let actual = runtime_phase_name(&other);
            put_runtime(other);
            Err(HostError::WrongPhase {
                expected: "WaitingTools",
                actual,
            })
        }
    };

    match result {
        Ok((events, actions)) => ok(StepOutput {
            events: try_conv!(events
                .into_iter()
                .map(|e| e.try_into())
                .collect::<Result<Vec<_>, _>>()),
            actions: try_conv!(actions
                .into_iter()
                .map(|a| a.try_into())
                .collect::<Result<Vec<_>, _>>()),
        }),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "onToolStarted")]
pub fn on_tool_started(handle: u32, tool_call_id: String) -> EventsResult {
    console_error_panic_hook::set_once();
    info!(
        handle,
        tool_call_id = tool_call_id.as_str(),
        "onToolStarted called"
    );

    let id = pi_core::ToolCallId::new(&tool_call_id);
    let runtime = match take_runtime(handle) {
        Ok(r) => r,
        Err(e) => return err(&e),
    };

    let result = match runtime {
        AgentRuntime::WaitingTools(mut waiting) => {
            let events = waiting.on_tool_started(id);
            put_runtime(waiting.into_runtime());
            Ok(events)
        }
        other => {
            let actual = runtime_phase_name(&other);
            put_runtime(other);
            Err(HostError::WrongPhase {
                expected: "WaitingTools",
                actual,
            })
        }
    };

    match result {
        Ok(events) => ok(EventsOutput {
            events: try_conv!(events
                .into_iter()
                .map(|e| e.try_into())
                .collect::<Result<Vec<_>, _>>()),
        }),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "onToolUpdate")]
pub fn on_tool_update(handle: u32, update: ToolExecutionUpdate) -> EventsResult {
    console_error_panic_hook::set_once();

    let core_update: pi_core::ToolExecutionUpdate = try_conv!(update.try_into());
    let runtime = match take_runtime(handle) {
        Ok(r) => r,
        Err(e) => return err(&e),
    };

    let result = match runtime {
        AgentRuntime::WaitingTools(mut waiting) => {
            let events = waiting.on_tool_update(core_update);
            put_runtime(waiting.into_runtime());
            Ok(events)
        }
        other => {
            let actual = runtime_phase_name(&other);
            put_runtime(other);
            Err(HostError::WrongPhase {
                expected: "WaitingTools",
                actual,
            })
        }
    };

    match result {
        Ok(events) => ok(EventsOutput {
            events: try_conv!(events
                .into_iter()
                .map(|e| e.try_into())
                .collect::<Result<Vec<_>, _>>()),
        }),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "onToolCancelled")]
pub fn on_tool_cancelled(handle: u32, tool_call_id: String, reason: CancelReason) -> StepResult {
    console_error_panic_hook::set_once();

    let id = pi_core::ToolCallId::new(&tool_call_id);
    let core_reason: pi_core::CancelReason = try_conv!(reason.try_into());
    let runtime = match take_runtime(handle) {
        Ok(r) => r,
        Err(e) => return err(&e),
    };

    let result = match runtime {
        AgentRuntime::WaitingTools(waiting) => {
            let (events, actions, new_runtime) = waiting.cancel_tool(id, core_reason).into_parts();
            put_runtime(new_runtime);
            Ok((events, actions))
        }
        other => {
            let actual = runtime_phase_name(&other);
            put_runtime(other);
            Err(HostError::WrongPhase {
                expected: "WaitingTools",
                actual,
            })
        }
    };

    match result {
        Ok((events, actions)) => ok(StepOutput {
            events: try_conv!(events
                .into_iter()
                .map(|e| e.try_into())
                .collect::<Result<Vec<_>, _>>()),
            actions: try_conv!(actions
                .into_iter()
                .map(|a| a.try_into())
                .collect::<Result<Vec<_>, _>>()),
        }),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "abort")]
pub fn abort(handle: u32) -> StepResult {
    console_error_panic_hook::set_once();
    info!(handle, "abort called");

    let runtime = match take_runtime(handle) {
        Ok(r) => r,
        Err(e) => return err(&e),
    };

    let result = match runtime {
        AgentRuntime::Streaming(streaming) => {
            let Transition {
                events,
                actions,
                state,
            } = streaming.abort();
            put_runtime(state.into_runtime());
            Ok((events, actions))
        }
        AgentRuntime::WaitingTools(waiting) => {
            let Transition {
                events,
                actions,
                state,
            } = waiting.abort();
            put_runtime(state.into_runtime());
            Ok((events, actions))
        }
        AgentRuntime::ReadyToContinue(ready) => {
            let Transition {
                events,
                actions,
                state,
            } = ready.abort();
            put_runtime(state.into_runtime());
            Ok((events, actions))
        }
        other => {
            let actual = runtime_phase_name(&other);
            put_runtime(other);
            Err(HostError::WrongPhase {
                expected: "Streaming, WaitingTools, or ReadyToContinue",
                actual,
            })
        }
    };

    match result {
        Ok((events, actions)) => ok(StepOutput {
            events: try_conv!(events
                .into_iter()
                .map(|e| e.try_into())
                .collect::<Result<Vec<_>, _>>()),
            actions: try_conv!(actions
                .into_iter()
                .map(|a| a.try_into())
                .collect::<Result<Vec<_>, _>>()),
        }),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "continueTurn")]
pub fn continue_turn(handle: u32) -> StepResult {
    console_error_panic_hook::set_once();
    info!(handle, "continueTurn called");

    let runtime = match take_runtime(handle) {
        Ok(r) => r,
        Err(e) => return err(&e),
    };

    let result = match runtime {
        AgentRuntime::ReadyToContinue(ready) => {
            let Transition {
                events,
                actions,
                state,
            } = ready.continue_turn();
            put_runtime(state.into_runtime());
            Ok((events, actions))
        }
        other => {
            let actual = runtime_phase_name(&other);
            put_runtime(other);
            Err(HostError::WrongPhase {
                expected: "ReadyToContinue",
                actual,
            })
        }
    };

    match result {
        Ok((events, actions)) => ok(StepOutput {
            events: try_conv!(events
                .into_iter()
                .map(|e| e.try_into())
                .collect::<Result<Vec<_>, _>>()),
            actions: try_conv!(actions
                .into_iter()
                .map(|a| a.try_into())
                .collect::<Result<Vec<_>, _>>()),
        }),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "steer")]
pub fn steer(handle: u32, message: AgentMessage) -> EventsResult {
    console_error_panic_hook::set_once();

    let core_message: pi_core::AgentMessage = try_conv!(message.try_into());
    let runtime = match take_runtime(handle) {
        Ok(r) => r,
        Err(e) => return err(&e),
    };

    let result = match runtime {
        AgentRuntime::Idle(mut idle) => {
            let events = idle.steer(core_message);
            put_runtime(idle.into_runtime());
            Ok(events)
        }
        AgentRuntime::ReadyToContinue(mut ready) => {
            let events = ready.steer(core_message);
            put_runtime(ready.into_runtime());
            Ok(events)
        }
        other => {
            let actual = runtime_phase_name(&other);
            put_runtime(other);
            Err(HostError::WrongPhase {
                expected: "Idle or ReadyToContinue",
                actual,
            })
        }
    };

    match result {
        Ok(events) => ok(EventsOutput {
            events: try_conv!(events
                .into_iter()
                .map(|e| e.try_into())
                .collect::<Result<Vec<_>, _>>()),
        }),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "followUp")]
pub fn follow_up(handle: u32, message: AgentMessage) -> EmptyResult {
    console_error_panic_hook::set_once();

    let core_message: pi_core::AgentMessage = try_conv!(message.try_into());
    let runtime = match take_runtime(handle) {
        Ok(r) => r,
        Err(e) => return err(&e),
    };

    let result = match runtime {
        AgentRuntime::Idle(mut idle) => {
            idle.follow_up(core_message);
            put_runtime(idle.into_runtime());
            Ok(())
        }
        AgentRuntime::ReadyToContinue(mut ready) => {
            ready.follow_up(core_message);
            put_runtime(ready.into_runtime());
            Ok(())
        }
        AgentRuntime::WaitingTools(mut waiting) => {
            waiting.submit_user_message(core_message);
            put_runtime(waiting.into_runtime());
            Ok(())
        }
        other => {
            let actual = runtime_phase_name(&other);
            put_runtime(other);
            Err(HostError::WrongPhase {
                expected: "Idle, ReadyToContinue, or WaitingTools",
                actual,
            })
        }
    };

    match result {
        Ok(()) => ok(()),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "state")]
pub fn state(handle: u32) -> StateResult {
    console_error_panic_hook::set_once();

    let result = with_runtime(handle, |runtime| runtime.state().clone());
    match result {
        Ok(core_state) => ok(StateOutput {
            state: try_conv!(core_state.try_into()),
        }),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "reset")]
pub fn reset(handle: u32) -> EmptyResult {
    console_error_panic_hook::set_once();

    let runtime = match take_runtime(handle) {
        Ok(r) => r,
        Err(e) => return err(&e),
    };
    let runtime = runtime.reset();
    put_runtime(runtime);
    ok(())
}

#[wasm_bindgen(js_name = "destroyAgent")]
pub fn destroy_agent(handle: u32) -> EmptyResult {
    console_error_panic_hook::set_once();

    match take_runtime(handle) {
        Ok(_) => ok(()),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "projectContext")]
pub fn project_context_export(input: ProjectionInput) -> ProjectionResult {
    console_error_panic_hook::set_once();
    info!("projectContext called");

    let core_input: pi_core::ProjectionInput = try_conv!(input.try_into());
    let core_output = project_context(core_input);
    let output: ProjectionOutput = try_conv!(core_output.try_into());
    ok(output)
}

#[wasm_bindgen(js_name = "getSessionState")]
pub fn get_session_state(handle: u32) -> SessionStateResult {
    console_error_panic_hook::set_once();

    let runtime = match take_runtime(handle) {
        Ok(r) => r,
        Err(e) => return err(&e),
    };
    let core_state = runtime.session_state().clone();
    put_runtime(runtime);
    ok(SessionStateOutput {
        state: try_conv!(core_state.try_into()),
    })
}

#[wasm_bindgen(js_name = "setSessionState")]
pub fn set_session_state(handle: u32, state: SessionState) -> EmptyResult {
    console_error_panic_hook::set_once();

    let core_state: pi_core::SessionState = try_conv!(state.try_into());
    let runtime = match take_runtime(handle) {
        Ok(r) => r,
        Err(e) => return err(&e),
    };
    let mut runtime = runtime;
    runtime.set_session_state(core_state);
    put_runtime(runtime);
    ok(())
}

#[wasm_bindgen(js_name = "getSessionBranch")]
pub fn get_session_branch(handle: u32) -> SessionBranchResult {
    console_error_panic_hook::set_once();

    let runtime = match take_runtime(handle) {
        Ok(r) => r,
        Err(e) => return err(&e),
    };
    let agent = runtime.into_agent();
    let entries = agent.session_branch();
    put_runtime(AgentRuntime::from_agent(agent));
    ok(SessionBranchOutput {
        entries: try_conv!(entries
            .into_iter()
            .map(|e| e.try_into())
            .collect::<Result<Vec<_>, _>>()),
    })
}

#[wasm_bindgen(js_name = "moveTo")]
pub fn move_to(handle: u32, target_id: String, summary: Option<BranchSummary>) -> MoveToResult {
    console_error_panic_hook::set_once();

    let core_summary: Option<pi_core::BranchSummary> = summary.map(|s| try_conv!(s.try_into()));
    let runtime = match take_runtime(handle) {
        Ok(r) => r,
        Err(e) => return err(&e),
    };
    let mut agent = runtime.into_agent();
    let id = agent.move_to(&target_id, core_summary);
    put_runtime(AgentRuntime::from_agent(agent));
    ok(MoveToOutput {
        summary_entry_id: id,
    })
}

#[wasm_bindgen(js_name = "appendSessionEntry")]
pub fn append_session_entry(handle: u32, entry: SessionEntry) -> EmptyResult {
    console_error_panic_hook::set_once();

    let core_entry: pi_core::SessionEntry = try_conv!(entry.try_into());
    let runtime = match take_runtime(handle) {
        Ok(r) => r,
        Err(e) => return err(&e),
    };
    let mut agent = runtime.into_agent();
    agent.append_session_entry(core_entry);
    put_runtime(AgentRuntime::from_agent(agent));
    ok(())
}

#[wasm_bindgen(js_name = "estimateTokens")]
pub fn estimate_tokens_export(input: EstimateTokensInput) -> EstimateTokensResult {
    console_error_panic_hook::set_once();

    let core_messages: Vec<pi_core::AgentMessage> = try_conv!(input
        .messages
        .into_iter()
        .map(|m| m.try_into())
        .collect::<Result<Vec<_>, _>>());

    let tokens = estimate_tokens(&core_messages);
    ok(EstimateTokensOutput { tokens })
}

#[wasm_bindgen(js_name = "estimateTokensForText")]
pub fn estimate_tokens_for_text_export(text: String) -> EstimateTokensResult {
    console_error_panic_hook::set_once();

    let tokens = estimate_tokens_for_text(&text);
    ok(EstimateTokensOutput { tokens })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_options() -> AgentOptions {
        AgentOptions {
            system_prompt: "test agent".to_string(),
            model: Model {
                id: ModelId("test-model".to_string()),
                name: ModelName("Test".to_string()),
                api: ApiName("test".to_string()),
                provider: ProviderName("test".to_string()),
                base_url: None,
                reasoning: false,
                context_window: 4096,
                max_tokens: 1024,
                capabilities: Default::default(),
                cost: Default::default(),
            },
            thinking_level: Default::default(),
            steering_mode: Default::default(),
            follow_up_mode: Default::default(),
            tool_execution_mode: Default::default(),
            session_id: None,
            messages: vec![],
            session_state: None,
        }
    }

    #[test]
    fn create_agent_returns_ok_with_handle() {
        let resp = create_agent(dummy_options());
        assert!(resp.ok);
        assert!(resp.data.is_some());
        assert_eq!(resp.data.unwrap().handle, 0);
    }

    #[test]
    fn prompt_returns_stream_llm_action() {
        create_agent(dummy_options());
        let resp = prompt(
            0,
            PromptInput {
                prompt: PromptRequest::Text {
                    text: "hello".to_string(),
                },
                tools: vec![],
            },
        );
        assert!(resp.ok);
        let actions = resp.data.unwrap().actions;
        assert!(actions
            .iter()
            .any(|a| matches!(a, AgentAction::StreamLlm { .. })));
        destroy_agent(0);
    }

    #[test]
    fn bad_handle_returns_error() {
        let resp = prompt(
            9999,
            PromptInput {
                prompt: PromptRequest::Text {
                    text: "hi".to_string(),
                },
                tools: vec![],
            },
        );
        assert!(!resp.ok);
        assert_eq!(resp.error.as_ref().unwrap().code, "bad_handle");
    }

    #[test]
    fn on_llm_done_with_no_tools_finishes() {
        create_agent(dummy_options());
        prompt(
            0,
            PromptInput {
                prompt: PromptRequest::Text {
                    text: "hello".to_string(),
                },
                tools: vec![],
            },
        );

        let done_resp = on_llm_done(
            0,
            LlmResult::Ok(AssistantMessage {
                content: vec![Content::Text(TextContent {
                    text: "hi".to_string(),
                })],
                api: ApiName("test".to_string()),
                provider: ProviderName("test".to_string()),
                model: ModelId("test".to_string()),
                stop_reason: StopReason::EndTurn,
                error_message: None,
                timestamp: 1,
                usage: TokenUsage::default(),
            }),
        );
        assert!(done_resp.ok);
        let actions = done_resp.data.unwrap().actions;
        assert!(actions
            .iter()
            .any(|a| matches!(a, AgentAction::Finished { .. })));

        destroy_agent(0);
    }

    #[test]
    fn reset_clears_state() {
        create_agent(dummy_options());
        prompt(
            0,
            PromptInput {
                prompt: PromptRequest::Text {
                    text: "hello".to_string(),
                },
                tools: vec![],
            },
        );

        let reset_resp = reset(0);
        assert!(reset_resp.ok);

        let state_resp = state(0);
        assert!(state_resp.ok);
        assert!(state_resp.data.unwrap().state.messages.is_empty());

        destroy_agent(0);
    }

    #[test]
    fn state_returns_system_prompt() {
        create_agent(dummy_options());
        let state_resp = state(0);
        assert!(state_resp.ok);
        assert_eq!(state_resp.data.unwrap().state.system_prompt, "test agent");
        destroy_agent(0);
    }

    #[test]
    fn destroy_agent_frees_handle() {
        create_agent(dummy_options());
        let destroy_resp = destroy_agent(0);
        assert!(destroy_resp.ok);

        let state_resp = state(0);
        assert!(!state_resp.ok);
        assert_eq!(state_resp.error.as_ref().unwrap().code, "bad_handle");
    }

    #[test]
    fn empty_result_serialize() {
        let r = EmptyResult {
            ok: true,
            data: Some(()),
            error: None,
        };
        let json = serde_json::to_string(&r).unwrap();
        println!("EmptyResult JSON: {}", json);
        assert!(json.contains("\"ok\":true"));
        assert!(json.contains("\"data\":null"));
    }

    #[test]
    fn project_context_returns_ok_with_report() {
        let input = ProjectionInput {
            system_prompt: "You are helpful.".to_string(),
            messages: vec![AgentMessage::User(UserMessage {
                content: vec![Content::Text(TextContent {
                    text: "hello".to_string(),
                })],
                timestamp: 1,
            })],
            budget: ContextProjectionBudget {
                max_tool_result_chars: 50000,
                max_context_tokens: 100000,
                microcompact_after_turns: 5,
                compaction_threshold: 0.75,
            },
            state: ContextProjectionState::default(),
        };

        let resp = project_context_export(input);
        assert!(resp.ok);
        let data = resp.data.unwrap();
        assert!(!data.projected_messages.is_empty());
        assert!(data.report.estimated_tokens > 0);
    }

    #[test]
    fn get_session_state_after_creation_is_empty() {
        create_agent(dummy_options());
        let resp = get_session_state(0);
        assert!(resp.ok);
        let state = resp.data.unwrap().state;
        assert!(state.entries.is_empty());
        assert_eq!(state.leaf_id, "");
        destroy_agent(0);
    }

    #[test]
    fn set_and_get_session_state_roundtrip() {
        create_agent(dummy_options());
        let custom_state = SessionState {
            entries: vec![SessionEntry {
                id: "entry-0".to_string(),
                parent_id: None,
                kind: EntryKind::Message {
                    message: AgentMessage::User(UserMessage {
                        content: vec![Content::Text(TextContent {
                            text: "hi".to_string(),
                        })],
                        timestamp: 1,
                    }),
                },
                timestamp: 1,
            }],
            leaf_id: "entry-0".to_string(),
            name: "test-session".to_string(),
            projection_state: None,
            artifacts: Vec::new(),
        };

        let set_resp = set_session_state(0, custom_state.clone());
        assert!(set_resp.ok);

        let get_resp = get_session_state(0);
        assert!(get_resp.ok);
        let retrieved = get_resp.data.unwrap().state;
        assert_eq!(retrieved.entries.len(), 1);
        assert_eq!(retrieved.leaf_id, "entry-0");
        assert_eq!(retrieved.name, "test-session");
        destroy_agent(0);
    }

    #[test]
    fn get_session_branch_after_prompt() {
        create_agent(dummy_options());
        prompt(
            0,
            PromptInput {
                prompt: PromptRequest::Text {
                    text: "hello".to_string(),
                },
                tools: vec![],
            },
        );

        let resp = get_session_branch(0);
        assert!(resp.ok);
        let entries = resp.data.unwrap().entries;
        assert_eq!(entries.len(), 1);
        assert!(matches!(entries[0].kind, EntryKind::Message { .. }));
        destroy_agent(0);
    }

    #[test]
    fn estimate_tokens_returns_value() {
        let input = EstimateTokensInput {
            messages: vec![AgentMessage::User(UserMessage {
                content: vec![Content::Text(TextContent {
                    text: "hello world".to_string(),
                })],
                timestamp: 1,
            })],
        };
        let resp = estimate_tokens_export(input);
        assert!(resp.ok);
        assert_eq!(resp.data.unwrap().tokens, 3); // (11 + 3) / 4 = 3.5 -> 3
    }

    #[test]
    fn estimate_tokens_for_text_returns_value() {
        let resp = estimate_tokens_for_text_export("hello".to_string());
        assert!(resp.ok);
        assert_eq!(resp.data.unwrap().tokens, 2); // (5 + 3) / 4 = 2
    }

    #[test]
    fn abort_from_streaming_returns_aborted() {
        create_agent(dummy_options());
        prompt(
            0,
            PromptInput {
                prompt: PromptRequest::Text {
                    text: "hello".to_string(),
                },
                tools: vec![],
            },
        );

        let abort_resp = abort(0);
        assert!(abort_resp.ok);
        let actions = abort_resp.data.unwrap().actions;
        assert!(actions.is_empty());

        // After abort, state should not be streaming
        let state_resp = state(0);
        assert!(state_resp.ok);
        assert!(!state_resp.data.unwrap().state.is_streaming);

        destroy_agent(0);
    }

    #[test]
    fn abort_from_idle_returns_wrong_phase() {
        create_agent(dummy_options());
        let abort_resp = abort(0);
        assert!(!abort_resp.ok);
        assert_eq!(abort_resp.error.as_ref().unwrap().code, "wrong_phase");
        destroy_agent(0);
    }

    #[test]
    fn abort_from_waiting_tools_clears_pending() {
        create_agent(dummy_options());
        prompt(
            0,
            PromptInput {
                prompt: PromptRequest::Text {
                    text: "use tool".to_string(),
                },
                tools: vec![],
            },
        );

        let assistant = AssistantMessage {
            content: vec![Content::ToolCall(ToolCall {
                id: ToolCallId("tc-1".to_string()),
                name: ToolName("test_tool".to_string()),
                arguments: ToolArguments(serde_json::json!({})),
            })],
            api: ApiName("test".to_string()),
            provider: ProviderName("test".to_string()),
            model: ModelId("test".to_string()),
            stop_reason: StopReason::ToolUse,
            error_message: None,
            timestamp: 1,
            usage: TokenUsage::default(),
        };
        let done_resp = on_llm_done(0, LlmResult::Ok(assistant));
        assert!(done_resp.ok);
        let state_resp = state(0);
        assert!(state_resp.ok);
        assert!(!state_resp.data.unwrap().state.pending_tool_calls.is_empty());

        let abort_resp = abort(0);
        assert!(abort_resp.ok);
        let state_resp = state(0);
        assert!(state_resp.ok);
        let state = state_resp.data.unwrap().state;
        assert!(
            state.pending_tool_calls.is_empty(),
            "abort from WaitingTools should clear pending tool calls"
        );
        assert!(!state.is_streaming);

        destroy_agent(0);
    }

    #[test]
    fn abort_from_ready_to_continue_is_allowed() {
        create_agent(dummy_options());
        prompt(
            0,
            PromptInput {
                prompt: PromptRequest::Text {
                    text: "use tool".to_string(),
                },
                tools: vec![],
            },
        );

        let assistant = AssistantMessage {
            content: vec![Content::ToolCall(ToolCall {
                id: ToolCallId("tc-1".to_string()),
                name: ToolName("test_tool".to_string()),
                arguments: ToolArguments(serde_json::json!({})),
            })],
            api: ApiName("test".to_string()),
            provider: ProviderName("test".to_string()),
            model: ModelId("test".to_string()),
            stop_reason: StopReason::ToolUse,
            error_message: None,
            timestamp: 1,
            usage: TokenUsage::default(),
        };
        let done_resp = on_llm_done(0, LlmResult::Ok(assistant));
        assert!(done_resp.ok);

        // Complete the tool to reach ReadyToContinue
        let tool_done = on_tool_done(
            0,
            "tc-1".to_string(),
            ToolDonePayload::Success {
                result: ToolResult {
                    content: vec![Content::Text(TextContent {
                        text: "ok".to_string(),
                    })],
                    details: None,
                    terminate: None,
                },
            },
        );
        assert!(tool_done.ok);
        let state_resp = state(0);
        assert!(state_resp.ok);
        let agent_state = state_resp.data.unwrap().state;
        assert!(agent_state.pending_tool_calls.is_empty());

        // Abort from ReadyToContinue should succeed
        let abort_resp = abort(0);
        assert!(abort_resp.ok);
        let state_resp = state(0);
        assert!(state_resp.ok);
        assert!(!state_resp.data.unwrap().state.is_streaming);

        destroy_agent(0);
    }

    #[test]
    fn prompt_with_tools_passes_them_through() {
        create_agent(dummy_options());
        let tool = ToolDefinition {
            name: ToolName("test_tool".to_string()),
            label: "Test".to_string(),
            description: "A test tool.".to_string(),
            parameters: JsonSchema(serde_json::json!({})),
            execution_mode: Default::default(),
            tool_run_mode: Default::default(),
        };
        let resp = prompt(
            0,
            PromptInput {
                prompt: PromptRequest::Text {
                    text: "use tool".to_string(),
                },
                tools: vec![tool],
            },
        );
        assert!(resp.ok);
        let actions = resp.data.unwrap().actions;
        assert!(actions
            .iter()
            .any(|a| matches!(a, AgentAction::StreamLlm { .. })));
        let context = match &actions[0] {
            AgentAction::StreamLlm { context, .. } => context,
            other => panic!("expected StreamLlm, got {other:?}"),
        };
        assert!(
            !context.tools.is_empty(),
            "prompt should pass tools through to StreamLlm context"
        );
        assert_eq!(context.tools[0].name, ToolName("test_tool".to_string()));
        destroy_agent(0);
    }

    #[test]
    fn prompt_from_finished_with_tools_passes_them() {
        create_agent(dummy_options());
        // First turn: finish without tools
        prompt(
            0,
            PromptInput {
                prompt: PromptRequest::Text {
                    text: "hello".to_string(),
                },
                tools: vec![],
            },
        );
        let done_resp = on_llm_done(
            0,
            LlmResult::Ok(AssistantMessage {
                content: vec![Content::Text(TextContent {
                    text: "hi".to_string(),
                })],
                api: ApiName("test".to_string()),
                provider: ProviderName("test".to_string()),
                model: ModelId("test".to_string()),
                stop_reason: StopReason::EndTurn,
                error_message: None,
                timestamp: 1,
                usage: TokenUsage::default(),
            }),
        );
        assert!(done_resp.ok);

        // Second turn with tools from Finished state
        let tool = ToolDefinition {
            name: ToolName("test_tool".to_string()),
            label: "Test".to_string(),
            description: "A test tool.".to_string(),
            parameters: JsonSchema(serde_json::json!({})),
            execution_mode: Default::default(),
            tool_run_mode: Default::default(),
        };
        let resp = prompt(
            0,
            PromptInput {
                prompt: PromptRequest::Text {
                    text: "use tool".to_string(),
                },
                tools: vec![tool],
            },
        );
        assert!(resp.ok);
        let actions = resp.data.unwrap().actions;
        let context = match &actions[0] {
            AgentAction::StreamLlm { context, .. } => context,
            other => panic!("expected StreamLlm, got {other:?}"),
        };
        assert!(
            !context.tools.is_empty(),
            "prompt from Finished should pass tools through"
        );
        assert_eq!(context.tools[0].name, ToolName("test_tool".to_string()));
        destroy_agent(0);
    }

    #[test]
    fn prompt_from_aborted_with_tools_passes_them() {
        create_agent(dummy_options());
        // Start a turn then abort
        prompt(
            0,
            PromptInput {
                prompt: PromptRequest::Text {
                    text: "hello".to_string(),
                },
                tools: vec![],
            },
        );
        let abort_resp = abort(0);
        assert!(abort_resp.ok);

        // Prompt from Aborted state with tools
        let tool = ToolDefinition {
            name: ToolName("test_tool".to_string()),
            label: "Test".to_string(),
            description: "A test tool.".to_string(),
            parameters: JsonSchema(serde_json::json!({})),
            execution_mode: Default::default(),
            tool_run_mode: Default::default(),
        };
        let resp = prompt(
            0,
            PromptInput {
                prompt: PromptRequest::Text {
                    text: "use tool".to_string(),
                },
                tools: vec![tool],
            },
        );
        assert!(resp.ok);
        let actions = resp.data.unwrap().actions;
        let context = match &actions[0] {
            AgentAction::StreamLlm { context, .. } => context,
            other => panic!("expected StreamLlm, got {other:?}"),
        };
        assert!(
            !context.tools.is_empty(),
            "prompt from Aborted should pass tools through"
        );
        assert_eq!(context.tools[0].name, ToolName("test_tool".to_string()));
        destroy_agent(0);
    }

    #[test]
    fn projection_strategy_dynamic_roundtrip() {
        let strategy = ProjectionStrategy::Dynamic {
            script: "#{ action: \"project\", text: head(text, 100) }".to_string(),
        };
        let core: pi_core::ProjectionStrategy = strategy.clone().try_into().unwrap();
        let back: ProjectionStrategy = core.try_into().unwrap();
        assert!(matches!(back, ProjectionStrategy::Dynamic { .. }));
    }

    #[test]
    fn tool_projection_state_inline_roundtrip() {
        let state = ToolProjectionState::Inline;
        let core: pi_core::ToolProjectionState = state.clone().try_into().unwrap();
        let back: ToolProjectionState = core.try_into().unwrap();
        assert!(matches!(back, ToolProjectionState::Inline));
    }

    #[test]
    fn tool_projection_state_deferred_roundtrip() {
        let state = ToolProjectionState::Deferred {
            until_turn: 5,
            inserted_at_turn: 2,
        };
        let core: pi_core::ToolProjectionState = state.clone().try_into().unwrap();
        let back: ToolProjectionState = core.try_into().unwrap();
        assert!(matches!(
            back,
            ToolProjectionState::Deferred {
                until_turn: 5,
                inserted_at_turn: 2
            }
        ));
    }

    #[test]
    fn tool_projection_state_replaced_roundtrip() {
        let replacement = ContextReplacement {
            tool_call_id: "tc-1".to_string(),
            tool_name: "read".to_string(),
            artifact_id: "tool-result-tc-1".to_string(),
            original_chars: 5000,
            preview_chars: 200,
            strategy: ProjectionStrategy::Fixed {
                shape: ProjectionShape::Head { max_chars: 200 },
                min_age: 0,
            },
            outcome: ProjectionOutcome {
                text: "hello".to_string(),
            },
        };
        let state = ToolProjectionState::Replaced {
            replacement,
            inserted_at_turn: 0,
        };
        let core: pi_core::ToolProjectionState = state.clone().try_into().unwrap();
        let back: ToolProjectionState = core.try_into().unwrap();
        assert!(matches!(back, ToolProjectionState::Replaced { .. }));
    }

    #[test]
    fn projection_outcome_roundtrip() {
        let outcome = ProjectionOutcome {
            text: "hello".to_string(),
        };
        let core: pi_core::ProjectionOutcome = outcome.clone().try_into().unwrap();
        let back: ProjectionOutcome = core.try_into().unwrap();
        assert_eq!(back.text, "hello");
    }

    #[test]
    fn projection_strategy_fixed_roundtrip() {
        let strategy = ProjectionStrategy::Fixed {
            shape: ProjectionShape::Head { max_chars: 2000 },
            min_age: 2,
        };
        let core: pi_core::ProjectionStrategy = strategy.clone().try_into().unwrap();
        let back: ProjectionStrategy = core.try_into().unwrap();
        assert!(
            matches!(
                back,
                ProjectionStrategy::Fixed {
                    shape: ProjectionShape::Head { max_chars: 2000 },
                    min_age: 2
                }
            ),
            "Fixed strategy should round-trip with shape and min_age"
        );
    }

    #[test]
    fn old_context_state_migration_roundtrip() {
        let old_state = ContextProjectionState {
            tools: std::collections::BTreeMap::new(),
            replacements: {
                let mut map = std::collections::BTreeMap::new();
                map.insert(
                    "tc-old".to_string(),
                    OldContextReplacement {
                        tool_call_id: "tc-old".to_string(),
                        tool_name: "read".to_string(),
                        artifact_id: "art-old".to_string(),
                        original_chars: 5000,
                        preview_chars: 200,
                        strategy: OldContextStrategy::Head { max_chars: 200 },
                    },
                );
                map
            },
            current_turn: 3,
            last_api_usage: None,
            turns_since_compaction: 1,
        };
        let core: pi_core::ContextProjectionState = old_state.try_into().unwrap();
        let entry = core.tools.get("tc-old");
        assert!(
            matches!(entry, Some(pi_core::ToolProjectionState::Replaced { replacement, .. }) if replacement.tool_call_id == "tc-old"),
            "old replacement should migrate to Replaced state, got {:?}",
            entry
        );
    }

    #[test]
    fn project_context_with_old_replacements_migrates() {
        let input = ProjectionInput {
            system_prompt: "You are helpful.".to_string(),
            messages: vec![AgentMessage::User(UserMessage {
                content: vec![Content::Text(TextContent {
                    text: "hello".to_string(),
                })],
                timestamp: 1,
            })],
            budget: ContextProjectionBudget {
                max_tool_result_chars: 50000,
                max_context_tokens: 100000,
                microcompact_after_turns: 5,
                compaction_threshold: 0.75,
            },
            state: ContextProjectionState {
                tools: std::collections::BTreeMap::new(),
                replacements: {
                    let mut map = std::collections::BTreeMap::new();
                    map.insert(
                        "tc-old".to_string(),
                        OldContextReplacement {
                            tool_call_id: "tc-old".to_string(),
                            tool_name: "read".to_string(),
                            artifact_id: "art-old".to_string(),
                            original_chars: 5000,
                            preview_chars: 200,
                            strategy: OldContextStrategy::Head { max_chars: 200 },
                        },
                    );
                    map
                },
                current_turn: 3,
                last_api_usage: None,
                turns_since_compaction: 0,
            },
        };

        let resp = project_context_export(input);
        assert!(resp.ok);
        let data = resp.data.unwrap();
        // The updated state should have migrated the old replacement into tools
        let migrated = data.updated_state.tools.get("tc-old");
        assert!(
            migrated.is_some(),
            "old replacement should be migrated into updated_state.tools"
        );
    }
}
