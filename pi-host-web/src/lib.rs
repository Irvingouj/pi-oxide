//! WASM host for pi-core.
//!
//! Exposes the agent state machine through typed WASM APIs.
//! Every function returns a `ResultEnvelope<T>` — never throws.

use std::cell::RefCell;

use wasm_bindgen::prelude::*;

use pi_core::Agent;
use tracing::info;

fn project_context(input: pi_core::ProjectionInput) -> pi_core::ProjectionOutput {
    pi_core::project(input)
}

mod dto;
use dto::*;

thread_local! {
    static AGENT_SLOTS: RefCell<Vec<Option<Agent>>> = RefCell::new(Vec::new());
    static TRACING_INIT: std::cell::Cell<bool> = std::cell::Cell::new(false);
}

fn init_tracing() {
    TRACING_INIT.with(|init| {
        if !init.get() {
            let _ = tracing::subscriber::set_global_default(ConsoleSubscriber);
            init.set(true);
        }
    });
}

#[derive(Debug)]
struct ConsoleSubscriber;

thread_local! {
    static TRACE_BUF: RefCell<Vec<String>> = RefCell::new(Vec::new());
}

#[wasm_bindgen(js_name = "drainTraceLog")]
pub fn drain_trace_log() -> Vec<String> {
    TRACE_BUF.with(|buf| {
        let mut buf = buf.borrow_mut();
        let out = buf.clone();
        buf.clear();
        out
    })
}

impl tracing::Subscriber for ConsoleSubscriber {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        metadata.level() <= &tracing::Level::INFO
    }
    fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> tracing::Id {
        tracing::Id::from_u64(1)
    }
    fn record(&self, _span: &tracing::Id, _values: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _span: &tracing::Id, _follows: &tracing::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let level = *event.metadata().level();
        let mut fields = String::new();
        let mut visitor = FieldVisitor(&mut fields);
        event.record(&mut visitor);
        let msg = format!("[pi-wasm {}] {}", level, fields);
        TRACE_BUF.with(|buf| buf.borrow_mut().push(msg));
    }
    fn enter(&self, _span: &tracing::Id) {}
    fn exit(&self, _span: &tracing::Id) {}
}

struct FieldVisitor<'a>(&'a mut String);
impl tracing::field::Visit for FieldVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if !self.0.is_empty() {
            self.0.push(' ');
        }
        self.0.push_str(field.name());
        self.0.push('=');
        write!(self.0, "{:?}", value).unwrap();
    }
}

use std::fmt::Write;

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
enum HostError {
    #[error("agent not found: handle {0} is invalid")]
    BadHandle(u32),
}

impl HostError {
    fn code(&self) -> &'static str {
        match self {
            HostError::BadHandle(_) => "bad_handle",
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

// ---------------------------------------------------------------------------
// Agent handle table
// ---------------------------------------------------------------------------

fn take_agent(handle: u32) -> Result<Agent, HostError> {
    AGENT_SLOTS.with(|slots| {
        let mut slots = slots.borrow_mut();
        let idx = handle as usize;
        if idx >= slots.len() {
            return Err(HostError::BadHandle(handle));
        }
        slots[idx].take().ok_or(HostError::BadHandle(handle))
    })
}

fn put_agent(agent: Agent) -> u32 {
    AGENT_SLOTS.with(|slots| {
        let mut slots = slots.borrow_mut();
        for (i, slot) in slots.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(agent);
                return i as u32;
            }
        }
        let handle = slots.len() as u32;
        slots.push(Some(agent));
        handle
    })
}

fn with_agent<T>(handle: u32, op: impl FnOnce(&mut Agent) -> T) -> Result<T, HostError> {
    AGENT_SLOTS.with(|slots| {
        let mut slots = slots.borrow_mut();
        let idx = handle as usize;
        if idx >= slots.len() {
            return Err(HostError::BadHandle(handle));
        }
        match &mut slots[idx] {
            Some(agent) => Ok(op(agent)),
            None => Err(HostError::BadHandle(handle)),
        }
    })
}

// ---------------------------------------------------------------------------
// WASM exports — typed DTOs, never strings
// ---------------------------------------------------------------------------

#[wasm_bindgen(js_name = "createAgent")]
pub fn create_agent(options: AgentOptions) -> CreateAgentResult {
    console_error_panic_hook::set_once();
    init_tracing();
    info!("createAgent called");
    let core_options: pi_core::AgentOptions = options.into();
    let agent = Agent::new(core_options);
    let handle = put_agent(agent);
    info!(handle, "agent created");
    ok(CreateAgentOutput { handle })
}

#[wasm_bindgen(js_name = "prompt")]
pub fn prompt(handle: u32, prompt: PromptRequest) -> StepResult {
    console_error_panic_hook::set_once();
    info!(handle, "prompt called");

    let core_prompt: pi_core::AgentMessage = match prompt {
        PromptRequest::Message(m) => m.into(),
        PromptRequest::Text { text } => pi_core::AgentMessage::user(text),
    };

    let result = with_agent(handle, |agent| agent.start_turn(core_prompt));
    match result {
        Ok((events, actions)) => ok(StepOutput {
            events: events.into_iter().map(|e| e.into()).collect(),
            actions: actions.into_iter().map(|a| a.into()).collect(),
        }),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "feedLlmChunk")]
pub fn feed_llm_chunk(handle: u32, chunk: LlmChunk) -> EventsResult {
    console_error_panic_hook::set_once();
    info!(handle, "feedLlmChunk called");

    let core_chunk: pi_core::LlmChunk = chunk.into();
    let result = with_agent(handle, |agent| agent.feed_llm_chunk(core_chunk));
    match result {
        Ok(events) => ok(EventsOutput {
            events: events.into_iter().map(|e| e.into()).collect(),
        }),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "onLlmDone")]
pub fn on_llm_done(handle: u32, result: LlmResult) -> StepResult {
    console_error_panic_hook::set_once();
    info!(handle, "onLlmDone called");

    let core_result: pi_core::LlmResult = result.into();
    let result = with_agent(handle, |agent| agent.on_llm_done(core_result));
    match result {
        Ok((events, actions)) => ok(StepOutput {
            events: events.into_iter().map(|e| e.into()).collect(),
            actions: actions.into_iter().map(|a| a.into()).collect(),
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
        ToolDonePayload::Failure { error } => Err(error.into()),
        ToolDonePayload::Success { result } => Ok(result.into()),
        ToolDonePayload::BareSuccess(result) => Ok(result.into()),
    };

    let result = with_agent(handle, |agent| agent.on_tool_done(id, core_result));
    match result {
        Ok((events, actions)) => ok(StepOutput {
            events: events.into_iter().map(|e| e.into()).collect(),
            actions: actions.into_iter().map(|a| a.into()).collect(),
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
    let result = with_agent(handle, |agent| agent.on_tool_started(id));
    match result {
        Ok(events) => ok(EventsOutput {
            events: events.into_iter().map(|e| e.into()).collect(),
        }),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "onToolUpdate")]
pub fn on_tool_update(handle: u32, update: ToolExecutionUpdate) -> EventsResult {
    console_error_panic_hook::set_once();

    let core_update: pi_core::ToolExecutionUpdate = update.into();
    let result = with_agent(handle, |agent| agent.on_tool_update(core_update));
    match result {
        Ok(events) => ok(EventsOutput {
            events: events.into_iter().map(|e| e.into()).collect(),
        }),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "onToolCancelled")]
pub fn on_tool_cancelled(handle: u32, tool_call_id: String, reason: CancelReason) -> StepResult {
    console_error_panic_hook::set_once();

    let id = pi_core::ToolCallId::new(&tool_call_id);
    let core_reason: pi_core::CancelReason = reason.into();
    let result = with_agent(handle, |agent| agent.on_tool_cancelled(id, core_reason));
    match result {
        Ok((events, actions)) => ok(StepOutput {
            events: events.into_iter().map(|e| e.into()).collect(),
            actions: actions.into_iter().map(|a| a.into()).collect(),
        }),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "steer")]
pub fn steer(handle: u32, message: AgentMessage) -> EventsResult {
    console_error_panic_hook::set_once();

    let core_message: pi_core::AgentMessage = message.into();
    let result = with_agent(handle, |agent| agent.steer(core_message));
    match result {
        Ok(events) => ok(EventsOutput {
            events: events.into_iter().map(|e| e.into()).collect(),
        }),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "followUp")]
pub fn follow_up(handle: u32, message: AgentMessage) -> EmptyResult {
    console_error_panic_hook::set_once();

    let core_message: pi_core::AgentMessage = message.into();
    let result = with_agent(handle, |agent| {
        agent.follow_up(core_message);
    });
    match result {
        Ok(()) => ok(()),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "state")]
pub fn state(handle: u32) -> StateResult {
    console_error_panic_hook::set_once();

    let result = with_agent(handle, |agent| agent.state().clone());
    match result {
        Ok(core_state) => ok(StateOutput {
            state: core_state.into(),
        }),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "reset")]
pub fn reset(handle: u32) -> EmptyResult {
    console_error_panic_hook::set_once();

    let result = with_agent(handle, |agent| agent.reset());
    match result {
        Ok(()) => ok(()),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "destroyAgent")]
pub fn destroy_agent(handle: u32) -> EmptyResult {
    console_error_panic_hook::set_once();

    match take_agent(handle) {
        Ok(_) => ok(()),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "projectContext")]
pub fn project_context_export(input: ProjectionInput) -> ProjectionResult {
    console_error_panic_hook::set_once();
    info!("projectContext called");

    let core_input: pi_core::ProjectionInput = input.into();
    let core_output = project_context(core_input);
    let output: ProjectionOutput = core_output.into();
    ok(output)
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
            tools: vec![],
            steering_mode: Default::default(),
            follow_up_mode: Default::default(),
            tool_execution_mode: Default::default(),
            session_id: None,
            messages: vec![],
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
            PromptRequest::Text {
                text: "hello".to_string(),
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
            PromptRequest::Text {
                text: "hi".to_string(),
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
            PromptRequest::Text {
                text: "hello".to_string(),
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
            PromptRequest::Text {
                text: "hello".to_string(),
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
                default_preview_chars: 2000,
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
}
