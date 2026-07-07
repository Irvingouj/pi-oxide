//! Cassette format — records raw HTTP request/response pairs for LLM API calls.
//!
//! Stores the raw JSON request body and the raw SSE response bytes (base64-encoded)
//! so that replay is a byte-for-byte replica of the real stream.

use serde::{Deserialize, Serialize};

/// A recorded cassette containing one or more LLM API call/response pairs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cassette {
    pub version: u32,
    /// The upstream API target, e.g. "https://api.deepseek.com"
    pub target: String,
    /// Sequential entries in the order they were recorded.
    pub entries: Vec<CassetteEntry>,
}

/// A single request/response pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CassetteEntry {
    pub request: RecordedRequest,
    pub response: RecordedResponse,
}

/// A recorded HTTP request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedRequest {
    /// HTTP method, e.g. "POST"
    pub method: String,
    /// URL path, e.g. "/v1/chat/completions"
    pub url: String,
    /// JSON body as a raw string.
    pub body_json: String,
}

/// A recorded HTTP response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedResponse {
    /// HTTP status code.
    pub status: u16,
    /// Raw SSE response body, base64-encoded.
    pub body_base64: String,
}
