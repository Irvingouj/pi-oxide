//! pi-llm: LLM provider protocol definitions.
//!
//! Pure types and traits. No network implementation.
//! Hosts bring their own HTTP clients (reqwest, fetch(), etc.)

pub mod schema;
pub mod stream;

pub use pi_core::{Model, ModelCapabilities, ModelCost, ModelProvider};
pub use schema::json_schema_for;
pub use stream::{LlmEvent, LlmStream, StreamError, StreamOptions};
