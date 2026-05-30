use crate::{
    dto::ArtifactSearchResults,
    handle_table::with_host_agent,
};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(js_name = "hostReadArtifact")]
pub fn host_read_artifact(handle: u32, artifact_id: String) -> Result<String, JsValue> {
    let result = with_host_agent(handle, |host_agent| {
        host_agent.host_state.read_artifact(&artifact_id).map(|s| s.to_string())
    });
    match result {
        Ok(Some(text)) => Ok(text),
        Ok(None) => Ok(String::new()),
        Err(e) => Err(serde_wasm_bindgen::to_value(&e.to_dto()).unwrap()),
    }
}

#[wasm_bindgen(js_name = "hostSearchArtifacts")]
pub fn host_search_artifacts(handle: u32, query: String) -> Result<ArtifactSearchResults, JsValue> {
    let result = with_host_agent(handle, |host_agent| {
        host_agent.host_state.search_artifacts(&query)
    });
    match result {
        Ok(results) => Ok(ArtifactSearchResults { results }),
        Err(e) => Err(serde_wasm_bindgen::to_value(&e.to_dto()).unwrap()),
    }
}
