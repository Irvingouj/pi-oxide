//! WASM host for pi-core.
//!
//! Exposes the agent state machine through typed WASM APIs.
//! Every function returns a `ResultEnvelope<T>` — never throws.

use std::cell::Cell;
use std::cell::RefCell;

use wasm_bindgen::prelude::*;

use pi_core::{
    estimate_tokens, estimate_tokens_for_text, fallback_strategy, Agent,
};
use tracing::info;
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;

fn project_context(input: pi_core::ProjectionInput) -> pi_core::ProjectionOutput {
    pi_core::project(input)
}

mod dto;
use dto::*;

thread_local! {
    static AGENT_SLOTS: RefCell<Vec<Option<Agent>>> = RefCell::new(Vec::new());
    static TRACING_INIT: Cell<bool> = Cell::new(false);
    static LOG_LEVEL: Cell<tracing::Level> = Cell::new(tracing::Level::INFO);
}

fn init_tracing() {
    TRACING_INIT.with(|init| {
        if !init.get() {
            #[cfg(target_arch = "wasm32")]
            {
                let layer = DynamicLevelFilter::new(tracing_wasm::WASMLayer::default());
                let _ = tracing_subscriber::registry().with(layer).try_init();
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let _ = tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).try_init();
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

struct DynamicLevelFilter<L> {
    inner: L,
}

impl<L> DynamicLevelFilter<L> {
    fn new(inner: L) -> Self {
        Self { inner }
    }
}

impl<S, L> Layer<S> for DynamicLevelFilter<L>
where
    S: tracing::Subscriber,
    L: Layer<S>,
{
    fn enabled(&self, metadata: &tracing::Metadata<'_>, ctx: Context<'_, S>) -> bool {
        let level = LOG_LEVEL.with(|c| c.get());
        if metadata.level() > &level {
            return false;
        }
        self.inner.enabled(metadata, ctx)
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: Context<'_, S>) {
        self.inner.on_event(event, ctx)
    }

    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &tracing::Id,
        ctx: Context<'_, S>,
    ) {
        self.inner.on_new_span(attrs, id, ctx)
    }

    fn on_record(
        &self,
        span: &tracing::Id,
        values: &tracing::span::Record<'_>,
        ctx: Context<'_, S>,
    ) {
        self.inner.on_record(span, values, ctx)
    }

    fn on_follows_from(
        &self,
        span: &tracing::Id,
        follows: &tracing::Id,
        ctx: Context<'_, S>,
    ) {
        self.inner.on_follows_from(span, follows, ctx)
    }

    fn on_enter(&self, id: &tracing::Id, ctx: Context<'_, S>) {
        self.inner.on_enter(id, ctx)
    }

    fn on_exit(&self, id: &tracing::Id, ctx: Context<'_, S>) {
        self.inner.on_exit(id, ctx)
    }

    fn on_close(&self, id: tracing::span::Id, ctx: Context<'_, S>) {
        self.inner.on_close(id, ctx)
    }

    fn on_id_change(
        &self,
        old: &tracing::span::Id,
        new: &tracing::span::Id,
        ctx: Context<'_, S>,
    ) {
        self.inner.on_id_change(old, new, ctx)
    }
}

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

#[wasm_bindgen(js_name = "getSessionState")]
pub fn get_session_state(handle: u32) -> SessionStateResult {
    console_error_panic_hook::set_once();

    let result = with_agent(handle, |agent| agent.session_state().clone());
    match result {
        Ok(core_state) => ok(SessionStateOutput {
            state: core_state.into(),
        }),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "setSessionState")]
pub fn set_session_state(handle: u32, state: SessionState) -> EmptyResult {
    console_error_panic_hook::set_once();

    let core_state: pi_core::SessionState = state.into();
    let result = with_agent(handle, |agent| agent.set_session_state(core_state));
    match result {
        Ok(()) => ok(()),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "getSessionBranch")]
pub fn get_session_branch(handle: u32) -> SessionBranchResult {
    console_error_panic_hook::set_once();

    let result = with_agent(handle, |agent| agent.session_branch());
    match result {
        Ok(entries) => ok(SessionBranchOutput {
            entries: entries.into_iter().map(|e| e.into()).collect(),
        }),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "moveTo")]
pub fn move_to(handle: u32, target_id: String, summary: Option<BranchSummary>) -> MoveToResult {
    console_error_panic_hook::set_once();

    let core_summary: Option<pi_core::BranchSummary> = summary.map(|s| s.into());
    let result = with_agent(handle, |agent| agent.move_to(&target_id, core_summary));
    match result {
        Ok(id) => ok(MoveToOutput {
            summary_entry_id: id,
        }),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "appendSessionEntry")]
pub fn append_session_entry(handle: u32, entry: SessionEntry) -> EmptyResult {
    console_error_panic_hook::set_once();

    let core_entry: pi_core::SessionEntry = entry.into();
    let result = with_agent(handle, |agent| agent.append_session_entry(core_entry));
    match result {
        Ok(()) => ok(()),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "estimateTokens")]
pub fn estimate_tokens_export(input: EstimateTokensInput) -> EstimateTokensResult {
    console_error_panic_hook::set_once();

    let core_messages: Vec<pi_core::AgentMessage> = input.messages.into_iter().map(|m| m.into()).collect();
    let tokens = estimate_tokens(&core_messages);
    ok(EstimateTokensOutput { tokens })
}

#[wasm_bindgen(js_name = "estimateTokensForText")]
pub fn estimate_tokens_for_text_export(text: String) -> EstimateTokensResult {
    console_error_panic_hook::set_once();

    let tokens = estimate_tokens_for_text(&text);
    ok(EstimateTokensOutput { tokens })
}

#[wasm_bindgen(js_name = "fallbackStrategy")]
pub fn fallback_strategy_export(tool_name: String) -> ContextStrategy {
    console_error_panic_hook::set_once();

    fallback_strategy(&tool_name).into()
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
            PromptRequest::Text {
                text: "hello".to_string(),
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
    fn fallback_strategy_returns_strategy() {
        let strategy = fallback_strategy_export("ls".to_string());
        assert!(matches!(strategy, ContextStrategy::Head { .. }));
    }
}
