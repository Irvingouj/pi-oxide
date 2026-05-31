use super::*;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(js_name = "createHostState")]
pub fn create_host_state(_budget: ContextProjectionBudget) -> CreateHostStateResult {
    console_error_panic_hook::set_once();
    let state = HostState::new(String::new(), String::new());
    let handle = put_host_state(state);
    ok(CreateHostStateOutput { handle })
}

#[wasm_bindgen(js_name = "destroyHostState")]
pub fn destroy_host_state(handle: u32) -> EmptyResult {
    console_error_panic_hook::set_once();
    match take_host_state(handle) {
        Ok(_) => ok(()),
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "getHostStatePersistData")]
pub fn get_host_state_persist_data(handle: u32) -> HostStatePersistDataResult {
    console_error_panic_hook::set_once();
    let transcript: Vec<pi_core::TrimmedMessage> = vec![];
    let artifacts: pi_core::Artifacts = std::collections::BTreeMap::new();
    let result = with_host_state(handle, |state| {
        state.get_persist_data(
            &transcript,
            &artifacts,
            0,
            &pi_core::ContextProjectionBudget::default(),
        )
    });
    match result {
        Ok(data) => {
            let dto_data: PersistData = try_conv!(data.try_into());
            ok(HostStatePersistDataOutput { state: dto_data })
        }
        Err(e) => err(&e),
    }
}

#[wasm_bindgen(js_name = "restoreHostState")]
pub fn restore_host_state(data: PersistData) -> CreateHostStateResult {
    console_error_panic_hook::set_once();
    let core_data: crate::host_state::PersistData = try_conv!(data.try_into());
    let state = HostState::restore(core_data);
    let handle = put_host_state(state);
    ok(CreateHostStateOutput { handle })
}

#[wasm_bindgen(js_name = "restoreHostStateFromJson")]
pub fn restore_host_state_from_json(json: String) -> CreateHostStateResult {
    console_error_panic_hook::set_once();
    // Try new format
    if let Ok(data) = serde_json::from_str::<crate::host_state::PersistData>(&json) {
        let state = HostState::restore(data);
        let handle = put_host_state(state);
        return ok(CreateHostStateOutput { handle });
    }
    err(&HostError::InvalidSessionJson)
}

// ---------------------------------------------------------------------------
// Stateless utility functions
// ---------------------------------------------------------------------------

#[wasm_bindgen(js_name = "estimateTokens")]
pub fn estimate_tokens_export(input: EstimateTokensInput) -> EstimateTokensResult {
    console_error_panic_hook::set_once();

    let core_messages: Vec<pi_core::AgentMessage> = try_conv!(input
        .messages
        .into_iter()
        .map(|m| m.try_into())
        .collect::<Result<Vec<_>, _>>());

    let tokens = pi_core::estimate_tokens(&core_messages);
    ok(EstimateTokensOutput { tokens })
}

#[wasm_bindgen(js_name = "estimateTokensForText")]
pub fn estimate_tokens_for_text_export(text: String) -> EstimateTokensResult {
    console_error_panic_hook::set_once();

    let tokens = pi_core::estimate_tokens_for_text(&text);
    ok(EstimateTokensOutput { tokens })
}
