//! WASM host for pi-core.
//!
//! Exposes the agent state machine through browser-friendly WASM APIs.
//! All functions take and return JSON strings using the typed envelope shape:
//!
//! ```json
//! { "ok": true,  "data": { ... } }
//! { "ok": false, "error": { "code": "...", "message": "..." } }
//! ```

use std::cell::RefCell;

use wasm_bindgen::prelude::*;

use pi_core::{
    Agent, AgentAction, AgentEvent, AgentMessage, AgentOptions, AgentState, LlmChunk, LlmResult,
    ToolCallId, ToolError, ToolResult,
    CancelReason, ToolExecutionUpdate,
    ProjectionInput, project as project_context,
};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

thread_local! {
    static AGENT_SLOTS: RefCell<Vec<Option<Agent>>> = RefCell::new(Vec::new());
}

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
enum HostError {
    #[error("invalid JSON for {name}: {source}")]
    InvalidJson {
        name: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("agent not found: handle {0} is invalid")]
    BadHandle(u32),
}

impl HostError {
    fn code(&self) -> &'static str {
        match self {
            HostError::InvalidJson { .. } => "invalid_json",
            HostError::BadHandle(_) => "bad_handle",
        }
    }
}

// ---------------------------------------------------------------------------
// JSON envelope
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct Envelope<T: Serialize> {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<ErrorBody>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ErrorBody {
    code: String,
    message: String,
}

fn ok_json<T: Serialize>(data: T) -> String {
    serde_json::to_string(&Envelope {
        ok: true,
        data: Some(data),
        error: None,
    })
    .unwrap_or_else(|e| error_json(&HostError::InvalidJson {
        name: "serialize_response",
        source: e,
    }))
}

fn error_json(err: &HostError) -> String {
    warn!(code = err.code(), message = %err, "host error");
    let body = ErrorBody {
        code: err.code().to_string(),
        message: err.to_string(),
    };
    serde_json::to_string(&Envelope::<()> {
        ok: false,
        data: None,
        error: Some(body),
    })
    .unwrap_or_else(|_| r#"{"ok":false,"error":{"code":"serialize_error","message":"failed to serialize error"}}"#.to_string())
}

// ---------------------------------------------------------------------------
// Output types
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Flexible input types (mirrors pi-bindings pattern)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_json<T: for<'de> Deserialize<'de>>(
    json: &str,
    name: &'static str,
) -> Result<T, HostError> {
    serde_json::from_str(json).map_err(|source| HostError::InvalidJson { name, source })
}

// ---------------------------------------------------------------------------
// WASM agent handle
// ---------------------------------------------------------------------------

/// Opaque WASM handle wrapping an `Agent`.
/// Stored in a thread-local slot table via handle indices.

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
        // Try to reuse a vacated slot.
        for (i, slot) in slots.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(agent);
                return i as u32;
            }
        }
        // No free slot; push a new one.
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
// WASM exports
// ---------------------------------------------------------------------------

/// Create a new agent from an `AgentOptions` JSON string.
/// Returns `{ ok: true, data: { handle } }` or an error envelope.
#[wasm_bindgen(js_name = "createAgent")]
pub fn create_agent(options_json: &str) -> String {
    console_error_panic_hook::set_once();

    match parse_json::<AgentOptions>(options_json, "AgentOptions") {
        Ok(options) => {
            debug!("creating wasm agent");
            let agent = Agent::new(options);
            let handle = put_agent(agent);
            ok_json(serde_json::json!({ "handle": handle }))
        }
        Err(err) => error_json(&err),
    }
}

/// Start a new turn with a prompt.
/// `prompt_json` can be a full `AgentMessage` or `{ "text": "..." }`.
#[wasm_bindgen(js_name = "prompt")]
pub fn prompt(handle: u32, prompt_json: &str) -> String {
    console_error_panic_hook::set_once();

    let result = (|| {
        let prompt: AgentMessage =
            parse_json::<PromptRequest>(prompt_json, "PromptRequest")?.into();
        with_agent(handle, |agent| agent.start_turn(prompt))
    })();

    match result {
        Ok((events, actions)) => ok_json(StepOutput { events, actions }),
        Err(err) => error_json(&err),
    }
}

/// Feed a streaming LLM chunk.
#[wasm_bindgen(js_name = "feedLlmChunk")]
pub fn feed_llm_chunk(handle: u32, chunk_json: &str) -> String {
    console_error_panic_hook::set_once();

    let result = (|| {
        let chunk: LlmChunk = parse_json(chunk_json, "LlmChunk")?;
        with_agent(handle, |agent| agent.feed_llm_chunk(chunk))
    })();

    match result {
        Ok(events) => ok_json(EventsOutput { events }),
        Err(err) => error_json(&err),
    }
}

/// Notify the agent that the LLM stream has finished.
#[wasm_bindgen(js_name = "onLlmDone")]
pub fn on_llm_done(handle: u32, result_json: &str) -> String {
    console_error_panic_hook::set_once();

    let result = (|| {
        let result: LlmResult = parse_json(result_json, "LlmResult")?;
        with_agent(handle, |agent| agent.on_llm_done(result))
    })();

    match result {
        Ok((events, actions)) => ok_json(StepOutput { events, actions }),
        Err(err) => error_json(&err),
    }
}

/// Notify the agent that a tool has finished executing.
#[wasm_bindgen(js_name = "onToolDone")]
pub fn on_tool_done(handle: u32, tool_call_id: &str, result_json: &str) -> String {
    console_error_panic_hook::set_once();

    let result = (|| {
        let id = ToolCallId::new(tool_call_id);
        let result: Result<ToolResult, ToolError> =
            parse_json::<ToolDonePayload>(result_json, "ToolDonePayload")?.into();
        with_agent(handle, |agent| agent.on_tool_done(id, result))
    })();

    match result {
        Ok((events, actions)) => ok_json(StepOutput { events, actions }),
        Err(err) => error_json(&err),
    }
}

/// Notify the agent that a tool has started executing.
#[wasm_bindgen(js_name = "onToolStarted")]
pub fn on_tool_started(handle: u32, tool_call_id: &str) -> String {
    console_error_panic_hook::set_once();

    let result = (|| {
        let id = ToolCallId::new(tool_call_id);
        Ok(with_agent(handle, |agent| agent.on_tool_started(id))?)
    })();

    match result {
        Ok(events) => ok_json(EventsOutput { events }),
        Err(err) => error_json(&err),
    }
}

/// Send a streaming tool execution update to the agent.
/// Input JSON must match `ToolExecutionUpdate`.
#[wasm_bindgen(js_name = "onToolUpdate")]
pub fn on_tool_update(handle: u32, update_json: &str) -> String {
    console_error_panic_hook::set_once();

    let result = (|| {
        let update: ToolExecutionUpdate = parse_json(update_json, "ToolExecutionUpdate")?;
        Ok(with_agent(handle, |agent| agent.on_tool_update(update))?)
    })();

    match result {
        Ok(events) => ok_json(EventsOutput { events }),
        Err(err) => error_json(&err),
    }
}

/// Notify the agent that a tool was cancelled.
#[wasm_bindgen(js_name = "onToolCancelled")]
pub fn on_tool_cancelled(handle: u32, tool_call_id: &str, reason_json: &str) -> String {
    console_error_panic_hook::set_once();

    let result = (|| {
        let id = ToolCallId::new(tool_call_id);
        let reason: CancelReason = parse_json(reason_json, "CancelReason")?;
        with_agent(handle, |agent| agent.on_tool_cancelled(id, reason))
    })();

    match result {
        Ok((events, actions)) => ok_json(StepOutput { events, actions }),
        Err(err) => error_json(&err),
    }
}

/// Inject a steering message mid-run.
#[wasm_bindgen(js_name = "steer")]
pub fn steer(handle: u32, message_json: &str) -> String {
    console_error_panic_hook::set_once();

    let result = (|| {
        let msg: AgentMessage = parse_json(message_json, "AgentMessage")?;
        with_agent(handle, |agent| agent.steer(msg))
    })();

    match result {
        Ok(events) => ok_json(EventsOutput { events }),
        Err(err) => error_json(&err),
    }
}

/// Queue a follow-up message for after the run would otherwise stop.
#[wasm_bindgen(js_name = "followUp")]
pub fn follow_up(handle: u32, message_json: &str) -> String {
    console_error_panic_hook::set_once();

    let result = (|| {
        let msg: AgentMessage = parse_json(message_json, "AgentMessage")?;
        with_agent(handle, |agent| {
            agent.follow_up(msg);
        })
    })();

    match result {
        Ok(()) => ok_json(serde_json::json!({})),
        Err(err) => error_json(&err),
    }
}

/// Get a read-only snapshot of the agent state.
#[wasm_bindgen(js_name = "state")]
pub fn state(handle: u32) -> String {
    console_error_panic_hook::set_once();

    let result = with_agent(handle, |agent| agent.state().clone());

    match result {
        Ok(state) => ok_json(StateOutput { state: &state }),
        Err(err) => error_json(&err),
    }
}

/// Reset the agent state.
#[wasm_bindgen(js_name = "reset")]
pub fn reset(handle: u32) -> String {
    console_error_panic_hook::set_once();

    let result = with_agent(handle, |agent| {
        agent.reset();
    });

    match result {
        Ok(()) => ok_json(serde_json::json!({})),
        Err(err) => error_json(&err),
    }
}

/// Destroy an agent and free its resources.
#[wasm_bindgen(js_name = "destroyAgent")]
pub fn destroy_agent(handle: u32) -> String {
    console_error_panic_hook::set_once();

    match take_agent(handle) {
        Ok(_) => ok_json(serde_json::json!({})),
        Err(err) => error_json(&err),
    }
}

/// Project context: run the Rust context projection engine.
///
/// Input JSON must match `ProjectionInput`:
/// ```json
/// {
///   "system_prompt": "...",
///   "messages": [...],
///   "budget": { "max_tool_result_chars": 50000, "max_context_tokens": 100000, "default_preview_chars": 2000 },
///   "state": { "replacements": {} }
/// }
/// ```
///
/// Returns:
/// ```json
/// { "ok": true, "data": { "projected_messages": [...], "updated_state": {...}, "report": {...} } }
/// ```
#[wasm_bindgen(js_name = "projectContext")]
pub fn project_context_export(input_json: &str) -> String {
    console_error_panic_hook::set_once();

    match parse_json::<ProjectionInput>(input_json, "ProjectionInput") {
        Ok(input) => {
            debug!("projecting context");
            let output = project_context(input);
            ok_json(output)
        }
        Err(err) => error_json(&err),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use pi_core::{Model, QueueMode, ThinkingLevel, ToolExecutionMode};

    fn dummy_model() -> Model {
        Model {
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
        }
    }

    fn dummy_options_json() -> String {
        let options = AgentOptions {
            system_prompt: "test agent".to_string(),
            model: dummy_model(),
            thinking_level: ThinkingLevel::Off,
            tools: vec![],
            steering_mode: QueueMode::OneAtATime,
            follow_up_mode: QueueMode::OneAtATime,
            tool_execution_mode: ToolExecutionMode::Parallel,
            session_id: None,
            messages: vec![],
        };
        serde_json::to_string(&options).unwrap()
    }

    fn parse_envelope(json: &str) -> serde_json::Value {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn create_agent_returns_ok_with_handle() {
        let resp = parse_envelope(&create_agent(&dummy_options_json()));
        assert_eq!(resp["ok"], true);
        assert!(resp["data"]["handle"].is_number());
    }

    #[test]
    fn create_agent_with_invalid_json_returns_error_envelope() {
        let resp = parse_envelope(&create_agent("{bad json}"));
        assert_eq!(resp["ok"], false);
        assert_eq!(resp["error"]["code"], "invalid_json");
        assert!(resp["error"]["message"].as_str().unwrap().contains("AgentOptions"));
    }

    #[test]
    fn prompt_returns_stream_llm_action() {
        let create_resp = parse_envelope(&create_agent(&dummy_options_json()));
        let handle = create_resp["data"]["handle"].as_u64().unwrap() as u32;

        let prompt_resp = parse_envelope(&prompt(handle, r#"{"text":"hello"}"#));
        assert_eq!(prompt_resp["ok"], true);
        let actions = prompt_resp["data"]["actions"].as_array().unwrap();
        assert!(actions
            .iter()
            .any(|a| a["type"] == "stream_llm"));

        // Clean up
        destroy_agent(handle);
    }

    #[test]
    fn prompt_with_invalid_json_returns_error_envelope() {
        let create_resp = parse_envelope(&create_agent(&dummy_options_json()));
        let handle = create_resp["data"]["handle"].as_u64().unwrap() as u32;

        let resp = parse_envelope(&prompt(handle, "{not-json}"));
        assert_eq!(resp["ok"], false);
        assert_eq!(resp["error"]["code"], "invalid_json");

        destroy_agent(handle);
    }

    #[test]
    fn bad_handle_returns_error_envelope() {
        let resp = parse_envelope(&prompt(9999, r#"{"text":"hi"}"#));
        assert_eq!(resp["ok"], false);
        assert_eq!(resp["error"]["code"], "bad_handle");
    }

    #[test]
    fn on_llm_done_with_no_tools_finishes() {
        let create_resp = parse_envelope(&create_agent(&dummy_options_json()));
        let handle = create_resp["data"]["handle"].as_u64().unwrap() as u32;

        prompt(handle, r#"{"text":"hello"}"#);

        let done_resp = parse_envelope(&on_llm_done(handle, r#"{"Ok":{"content":[{"type":"text","text":"hi"}],"api":"test","provider":"test","model":"test-model","stop_reason":"end_turn","timestamp":1,"usage":{"input":0,"output":0,"cache_read":0,"cache_write":0,"total_tokens":0}}}"#));
        assert_eq!(done_resp["ok"], true);
        let actions = done_resp["data"]["actions"].as_array().unwrap();
        assert!(actions.iter().any(|a| a["type"] == "finished"));

        destroy_agent(handle);
    }

    #[test]
    fn reset_clears_state() {
        let create_resp = parse_envelope(&create_agent(&dummy_options_json()));
        let handle = create_resp["data"]["handle"].as_u64().unwrap() as u32;

        prompt(handle, r#"{"text":"hello"}"#);
        let reset_resp = parse_envelope(&reset(handle));
        assert_eq!(reset_resp["ok"], true);

        let state_resp = parse_envelope(&state(handle));
        assert_eq!(state_resp["ok"], true);
        assert!(state_resp["data"]["state"]["messages"]
            .as_array()
            .unwrap()
            .is_empty());

        destroy_agent(handle);
    }

    #[test]
    fn state_returns_current_agent_state() {
        let create_resp = parse_envelope(&create_agent(&dummy_options_json()));
        let handle = create_resp["data"]["handle"].as_u64().unwrap() as u32;

        let state_resp = parse_envelope(&state(handle));
        assert_eq!(state_resp["ok"], true);
        assert_eq!(
            state_resp["data"]["state"]["system_prompt"],
            "test agent"
        );

        destroy_agent(handle);
    }

    #[test]
    fn destroy_agent_frees_handle() {
        let create_resp = parse_envelope(&create_agent(&dummy_options_json()));
        let handle = create_resp["data"]["handle"].as_u64().unwrap() as u32;

        let destroy_resp = parse_envelope(&destroy_agent(handle));
        assert_eq!(destroy_resp["ok"], true);

        // Using the handle after destroy should fail
        let state_resp = parse_envelope(&state(handle));
        assert_eq!(state_resp["ok"], false);
        assert_eq!(state_resp["error"]["code"], "bad_handle");
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

    // --- projectContext tests ---

    #[test]
    fn project_context_returns_ok_with_report() {
        let input = r#"{
            "system_prompt": "You are helpful.",
            "messages": [{"role":"user","content":[{"type":"text","text":"hello"}],"timestamp":1}],
            "budget": {"max_tool_result_chars":50000,"max_context_tokens":100000,"default_preview_chars":2000},
            "state": {"replacements":{}}
        }"#;

        let resp = parse_envelope(&project_context_export(input));
        assert_eq!(resp["ok"], true);
        assert!(resp["data"]["projected_messages"].is_array());
        assert!(resp["data"]["updated_state"].is_object());
        assert!(resp["data"]["report"]["estimated_tokens"].is_number());
        assert_eq!(resp["data"]["report"]["replacements"].as_array().unwrap().len(), 0);
        assert_eq!(resp["data"]["report"]["dropped_messages"], 0);
    }

    #[test]
    fn project_context_replaces_oversized_tool_result() {
        let big_text = "A".repeat(5000);
        let input = serde_json::json!({
            "system_prompt": "test",
            "messages": [
                {"role":"user","content":[{"type":"text","text":"run"}],"timestamp":1},
                {"role":"assistant","content":[{"type":"tool_call","id":"tc-1","name":"read","arguments":{}}],"api":"test","provider":"test","model":"test","stop_reason":"tool_use","timestamp":1,"usage":{"input":0,"output":0,"cache_read":0,"cache_write":0,"total_tokens":0}},
                {"role":"tool_result","tool_call_id":"tc-1","tool_name":"read","content":[{"type":"text","text":big_text}],"details":null,"is_error":false,"timestamp":1}
            ],
            "budget": {"max_tool_result_chars":1000,"max_context_tokens":100000,"default_preview_chars":200},
            "state": {"replacements":{}}
        });

        let resp_str = project_context_export(&serde_json::to_string(&input).unwrap());
        let resp = parse_envelope(&resp_str);
        assert_eq!(resp["ok"], true);
        let report = &resp["data"]["report"];
        assert_eq!(report["replacements"].as_array().unwrap().len(), 1);
        assert_eq!(report["replacements"][0]["artifact_id"], "tool-result-tc-1");
        assert_eq!(report["replacements"][0]["tool_name"], "read");

        // Projected message should contain preview marker
        let msgs = resp["data"]["projected_messages"].as_array().unwrap();
        let tool_msg = msgs.iter().find(|m| m["role"] == "tool_result").unwrap();
        let text = tool_msg["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("<context-artifact"));
        assert!(text.contains("head"));
    }

    #[test]
    fn project_context_returns_error_for_invalid_input() {
        let resp = parse_envelope(&project_context_export("{bad json}"));
        assert_eq!(resp["ok"], false);
        assert_eq!(resp["error"]["code"], "invalid_json");
    }
}
