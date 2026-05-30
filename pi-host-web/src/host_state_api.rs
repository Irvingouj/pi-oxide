use wasm_bindgen::prelude::*;
use super::*;

#[wasm_bindgen(js_name = "createHostState")]
pub fn create_host_state(budget: ContextProjectionBudget) -> CreateHostStateResult {
    console_error_panic_hook::set_once();
    let core_budget: pi_core::ContextProjectionBudget = try_conv!(budget.try_into());
    let state = HostState::new(String::new(), core_budget);
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
    let result = with_host_state(handle, |state| state.get_persist_data());
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
    // Try new format first
    if let Ok(data) = serde_json::from_str::<crate::host_state::PersistData>(&json) {
        let state = HostState::restore(data);
        let handle = put_host_state(state);
        return ok(CreateHostStateOutput { handle });
    }
    // Try old format
    if let Ok(old) = serde_json::from_str::<crate::dto::OldSessionState>(&json) {
        let state = HostState::migrate_from_old_session(old);
        let handle = put_host_state(state);
        return ok(CreateHostStateOutput { handle });
    }
    err(&HostError::InvalidSessionJson)
}
