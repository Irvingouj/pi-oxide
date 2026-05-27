//! Helpers for generating JSON Schema from Rust types.
//!
//! This module provides lightweight schema generation without pulling in
//! heavy dependencies. Hosts may replace this with `schemars` if desired.

use serde_json::{json, Value};

/// Generate a basic JSON Schema object for a tool parameter description.
pub fn json_schema_for(desc: &str, required: &[&str], properties: &[(String, Value)]) -> Value {
    json!({
        "type": "object",
        "description": desc,
        "properties": serde_json::Map::from_iter(
            properties.iter().cloned()
        ),
        "required": required,
    })
}
