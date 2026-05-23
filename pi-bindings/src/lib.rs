//! pi-bindings: Stable C ABI for pi-core.
//!
//! Opaque pointer + JSON wire protocol. No Rust types exposed directly.

pub mod c_api;

use std::cell::RefCell;
use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::Mutex;

use pi_core::{
    Agent, AgentAction, AgentEvent, AgentMessage, AgentOptions, AgentState, LlmChunk, LlmResult,
    ToolCallId, ToolError, ToolResult,
};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

thread_local! {
    static LAST_ERROR: RefCell<Option<BindingErrorBody>> = const { RefCell::new(None) };
}

/// Opaque handle to an Agent.
pub struct PiAgent {
    agent: Mutex<Agent>,
    callback: PiEventCallback,
    user_data: *mut c_void,
}

/// Type of the event callback provided by C hosts.
pub type PiEventCallback = Option<extern "C" fn(event_json: *const c_char, user_data: *mut c_void)>;

#[derive(Debug, thiserror::Error)]
enum BindingError {
    #[error("null pointer for {0}")]
    NullPointer(&'static str),
    #[error("invalid UTF-8 for {0}")]
    InvalidUtf8(&'static str),
    #[error("invalid JSON for {0}: {1}")]
    InvalidJson(&'static str, String),
    #[error("agent lock is poisoned")]
    PoisonedLock,
    #[error("failed to serialize response: {0}")]
    Serialize(String),
    #[error("response contains interior nul byte")]
    InteriorNul,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BindingErrorBody {
    pub code: String,
    pub message: String,
}

impl BindingError {
    fn code(&self) -> &'static str {
        match self {
            BindingError::NullPointer(_) => "null_pointer",
            BindingError::InvalidUtf8(_) => "invalid_utf8",
            BindingError::InvalidJson(_, _) => "invalid_json",
            BindingError::PoisonedLock => "poisoned_lock",
            BindingError::Serialize(_) => "serialize_error",
            BindingError::InteriorNul => "interior_nul",
        }
    }

    fn body(&self) -> BindingErrorBody {
        BindingErrorBody {
            code: self.code().to_string(),
            message: self.to_string(),
        }
    }
}

#[derive(Debug, Serialize)]
struct BindingResponse<T: Serialize> {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<BindingErrorBody>,
}

#[derive(Debug, Serialize)]
struct StepOutput {
    events: Vec<AgentEvent>,
    actions: Vec<AgentAction>,
}

#[derive(Debug, Serialize)]
struct EventsOutput {
    events: Vec<AgentEvent>,
}

#[derive(Debug, Serialize)]
struct StateOutput<'a> {
    state: &'a AgentState,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PromptRequest {
    Message(AgentMessage),
    Text { text: String },
}

impl From<PromptRequest> for AgentMessage {
    fn from(value: PromptRequest) -> Self {
        match value {
            PromptRequest::Message(message) => message,
            PromptRequest::Text { text } => AgentMessage::user(text),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ToolDonePayload {
    Failure { error: ToolError },
    Success { result: ToolResult },
    BareSuccess(ToolResult),
}

impl From<ToolDonePayload> for Result<ToolResult, ToolError> {
    fn from(value: ToolDonePayload) -> Self {
        match value {
            ToolDonePayload::Failure { error } => Err(error),
            ToolDonePayload::Success { result } => Ok(result),
            ToolDonePayload::BareSuccess(result) => Ok(result),
        }
    }
}

fn set_last_error(error: &BindingError) {
    let body = error.body();
    warn!(code = body.code, message = body.message, "pi binding error");
    LAST_ERROR.with(|slot| {
        *slot.borrow_mut() = Some(body);
    });
}

fn read_cstr<'a>(ptr: *const c_char, name: &'static str) -> Result<&'a str, BindingError> {
    if ptr.is_null() {
        return Err(BindingError::NullPointer(name));
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .map_err(|_| BindingError::InvalidUtf8(name))
}

fn parse_json<T: for<'de> Deserialize<'de>>(
    json: &str,
    name: &'static str,
) -> Result<T, BindingError> {
    serde_json::from_str(json).map_err(|err| BindingError::InvalidJson(name, err.to_string()))
}

fn to_c_string(value: impl Serialize) -> Result<*mut c_char, BindingError> {
    let json =
        serde_json::to_string(&value).map_err(|err| BindingError::Serialize(err.to_string()))?;
    CString::new(json)
        .map(CString::into_raw)
        .map_err(|_| BindingError::InteriorNul)
}

fn ok_response<T: Serialize>(data: T) -> *mut c_char {
    match to_c_string(BindingResponse {
        ok: true,
        data: Some(data),
        error: None,
    }) {
        Ok(ptr) => ptr,
        Err(error) => error_response(error),
    }
}

fn error_response(error: BindingError) -> *mut c_char {
    set_last_error(&error);
    let body = error.body();
    to_c_string(BindingResponse::<()> {
        ok: false,
        data: None,
        error: Some(body),
    })
    .unwrap_or(std::ptr::null_mut())
}

fn emit_events(agent: &PiAgent, events: &[AgentEvent]) {
    let Some(callback) = agent.callback else {
        return;
    };

    for event in events {
        match serde_json::to_string(event)
            .ok()
            .and_then(|json| CString::new(json).ok())
        {
            Some(json) => callback(json.as_ptr(), agent.user_data),
            None => warn!("failed to serialize event callback payload"),
        }
    }
}

fn with_agent<T: Serialize>(
    agent: *mut PiAgent,
    op: impl FnOnce(&mut Agent) -> T,
) -> Result<(*const PiAgent, T), BindingError> {
    if agent.is_null() {
        return Err(BindingError::NullPointer("agent"));
    }
    let agent_ref = unsafe { &*agent };
    let mut guard = agent_ref
        .agent
        .lock()
        .map_err(|_| BindingError::PoisonedLock)?;
    let result = op(&mut guard);
    Ok((agent_ref as *const PiAgent, result))
}

/// Create a new Agent.
///
/// `options_json` must be a valid JSON string representing `AgentOptions`.
/// `callback` receives JSON-serialized `AgentEvent` strings.
/// `user_data` is an opaque pointer passed back to the callback.
///
/// Returns an opaque handle, or NULL on parse error. Use `pi_last_error` for
/// the last creation failure on the current thread.
#[no_mangle]
pub extern "C" fn pi_agent_create(
    options_json: *const c_char,
    callback: PiEventCallback,
    user_data: *mut c_void,
) -> *mut PiAgent {
    let result = (|| {
        let options_str = read_cstr(options_json, "options_json")?;
        let options: AgentOptions = parse_json(options_str, "AgentOptions")?;
        debug!("creating pi agent");
        Ok(Box::into_raw(Box::new(PiAgent {
            agent: Mutex::new(Agent::new(options)),
            callback,
            user_data,
        })))
    })();

    match result {
        Ok(agent) => agent,
        Err(error) => {
            set_last_error(&error);
            std::ptr::null_mut()
        }
    }
}

/// Start a new turn with a prompt.
#[no_mangle]
pub extern "C" fn pi_agent_prompt(agent: *mut PiAgent, prompt_json: *const c_char) -> *mut c_char {
    let result = (|| {
        let prompt_str = read_cstr(prompt_json, "prompt_json")?;
        let prompt: AgentMessage = parse_json::<PromptRequest>(prompt_str, "PromptRequest")?.into();
        with_agent(agent, |guard| guard.start_turn(prompt))
    })();

    match result {
        Ok((agent_ref, (events, actions))) => {
            let agent_ref = unsafe { &*agent_ref };
            emit_events(agent_ref, &events);
            ok_response(StepOutput { events, actions })
        }
        Err(error) => error_response(error),
    }
}

/// Feed an LLM streaming chunk.
#[no_mangle]
pub extern "C" fn pi_agent_feed_llm_chunk(
    agent: *mut PiAgent,
    chunk_json: *const c_char,
) -> *mut c_char {
    let result = (|| {
        let chunk_str = read_cstr(chunk_json, "chunk_json")?;
        let chunk: LlmChunk = parse_json(chunk_str, "LlmChunk")?;
        with_agent(agent, |guard| guard.feed_llm_chunk(chunk))
    })();

    match result {
        Ok((agent_ref, events)) => {
            let agent_ref = unsafe { &*agent_ref };
            emit_events(agent_ref, &events);
            ok_response(EventsOutput { events })
        }
        Err(error) => error_response(error),
    }
}

/// Notify the agent that the LLM stream has finished.
#[no_mangle]
pub extern "C" fn pi_agent_on_llm_done(
    agent: *mut PiAgent,
    result_json: *const c_char,
) -> *mut c_char {
    let result = (|| {
        let result_str = read_cstr(result_json, "result_json")?;
        let result: LlmResult = parse_json(result_str, "LlmResult")?;
        with_agent(agent, |guard| guard.on_llm_done(result))
    })();

    match result {
        Ok((agent_ref, (events, actions))) => {
            let agent_ref = unsafe { &*agent_ref };
            emit_events(agent_ref, &events);
            ok_response(StepOutput { events, actions })
        }
        Err(error) => error_response(error),
    }
}

/// Notify the agent that a tool has finished executing.
#[no_mangle]
pub extern "C" fn pi_agent_on_tool_done(
    agent: *mut PiAgent,
    tool_call_id: *const c_char,
    result_json: *const c_char,
) -> *mut c_char {
    let result = (|| {
        let id = ToolCallId::new(read_cstr(tool_call_id, "tool_call_id")?);
        let result_str = read_cstr(result_json, "result_json")?;
        let result: Result<ToolResult, ToolError> =
            parse_json::<ToolDonePayload>(result_str, "ToolDonePayload")?.into();
        with_agent(agent, |guard| guard.on_tool_done(id, result))
    })();

    match result {
        Ok((agent_ref, (events, actions))) => {
            let agent_ref = unsafe { &*agent_ref };
            emit_events(agent_ref, &events);
            ok_response(StepOutput { events, actions })
        }
        Err(error) => error_response(error),
    }
}

/// Inject a steering message mid-run.
#[no_mangle]
pub extern "C" fn pi_agent_steer(agent: *mut PiAgent, message_json: *const c_char) -> *mut c_char {
    let result = (|| {
        let msg_str = read_cstr(message_json, "message_json")?;
        let msg: AgentMessage = parse_json(msg_str, "AgentMessage")?;
        with_agent(agent, |guard| guard.steer(msg))
    })();

    match result {
        Ok((agent_ref, events)) => {
            let agent_ref = unsafe { &*agent_ref };
            emit_events(agent_ref, &events);
            ok_response(EventsOutput { events })
        }
        Err(error) => error_response(error),
    }
}

/// Queue a follow-up message for after the run would otherwise stop.
#[no_mangle]
pub extern "C" fn pi_agent_follow_up(
    agent: *mut PiAgent,
    message_json: *const c_char,
) -> *mut c_char {
    let result = (|| {
        let msg_str = read_cstr(message_json, "message_json")?;
        let msg: AgentMessage = parse_json(msg_str, "AgentMessage")?;
        with_agent(agent, |guard| guard.follow_up(msg))
    })();

    match result {
        Ok((_agent_ref, ())) => ok_response(serde_json::json!({})),
        Err(error) => error_response(error),
    }
}

/// Get a read-only snapshot of the agent state.
#[no_mangle]
pub extern "C" fn pi_agent_state(agent: *const PiAgent) -> *mut c_char {
    if agent.is_null() {
        return error_response(BindingError::NullPointer("agent"));
    }
    let agent_ref = unsafe { &*agent };
    let guard = match agent_ref.agent.lock() {
        Ok(guard) => guard,
        Err(_) => return error_response(BindingError::PoisonedLock),
    };
    ok_response(StateOutput {
        state: guard.state(),
    })
}

/// Return the last binding error for the current thread.
#[no_mangle]
pub extern "C" fn pi_last_error() -> *mut c_char {
    let body = LAST_ERROR.with(|slot| slot.borrow().clone());
    match body {
        Some(error) => to_c_string(error).unwrap_or(std::ptr::null_mut()),
        None => to_c_string(serde_json::json!({})).unwrap_or(std::ptr::null_mut()),
    }
}

/// Reset the agent state.
#[no_mangle]
pub extern "C" fn pi_agent_reset(agent: *mut PiAgent) {
    if agent.is_null() {
        return;
    }
    let agent = unsafe { &*agent };

    if let Ok(mut guard) = agent.agent.lock() {
        guard.reset();
    }
}

/// Destroy an Agent and free its memory.
#[no_mangle]
pub extern "C" fn pi_agent_destroy(agent: *mut PiAgent) {
    if !agent.is_null() {
        unsafe { drop(Box::from_raw(agent)) };
    }
}

/// Free a string allocated by pi-bindings.
#[no_mangle]
pub extern "C" fn pi_free_string(s: *mut c_char) {
    if !s.is_null() {
        unsafe { drop(CString::from_raw(s)) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pi_core::{Model, QueueMode, ThinkingLevel, ToolExecutionMode};
    use std::ffi::CStr;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static CALLBACK_COUNT: AtomicUsize = AtomicUsize::new(0);

    extern "C" fn count_callback(_event_json: *const c_char, _user_data: *mut c_void) {
        CALLBACK_COUNT.fetch_add(1, Ordering::SeqCst);
    }

    fn options_json() -> CString {
        let options = AgentOptions {
            system_prompt: "test".to_string(),
            model: Model {
                id: "test-model".into(),
                name: "Test".into(),
                api: "test".into(),
                provider: "test".into(),
                base_url: None,
                reasoning: false,
                context_window: 4096,
                max_tokens: 1024,
                capabilities: Default::default(),
                cost: Default::default(),
            },
            thinking_level: ThinkingLevel::Off,
            tools: vec![],
            steering_mode: QueueMode::OneAtATime,
            follow_up_mode: QueueMode::OneAtATime,
            tool_execution_mode: ToolExecutionMode::Parallel,
            session_id: None,
            messages: vec![],
        };
        CString::new(serde_json::to_string(&options).unwrap()).unwrap()
    }

    unsafe fn take_json(ptr: *mut c_char) -> serde_json::Value {
        assert!(!ptr.is_null());
        let value = CStr::from_ptr(ptr).to_str().unwrap().to_string();
        pi_free_string(ptr);
        serde_json::from_str(&value).unwrap()
    }

    #[test]
    fn prompt_returns_envelope_and_emits_callback_events() {
        CALLBACK_COUNT.store(0, Ordering::SeqCst);
        let options = options_json();
        let agent = pi_agent_create(options.as_ptr(), Some(count_callback), std::ptr::null_mut());
        assert!(!agent.is_null());

        let prompt = CString::new(r#"{"text":"hello"}"#).unwrap();
        let response = unsafe { take_json(pi_agent_prompt(agent, prompt.as_ptr())) };

        assert_eq!(response["ok"], true);
        assert!(response["data"]["events"].as_array().unwrap().len() >= 2);
        assert_eq!(
            CALLBACK_COUNT.load(Ordering::SeqCst),
            response["data"]["events"].as_array().unwrap().len()
        );

        pi_agent_destroy(agent);
    }

    #[test]
    fn invalid_json_returns_error_envelope() {
        let options = options_json();
        let agent = pi_agent_create(options.as_ptr(), None, std::ptr::null_mut());
        assert!(!agent.is_null());

        let prompt = CString::new("{not-json").unwrap();
        let response = unsafe { take_json(pi_agent_prompt(agent, prompt.as_ptr())) };

        assert_eq!(response["ok"], false);
        assert_eq!(response["error"]["code"], "invalid_json");

        pi_agent_destroy(agent);
    }

    #[test]
    fn tool_done_payload_does_not_sniff_error_substrings() {
        let payload = r#"{
            "content": [{"type":"text","text":"ok"}],
            "details": {"error": "this is just data"}
        }"#;

        let parsed: ToolDonePayload = serde_json::from_str(payload).unwrap();
        assert!(Result::<ToolResult, ToolError>::from(parsed).is_ok());
    }
}
