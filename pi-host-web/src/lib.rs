//! WASM host for pi-core.
//!
//! Exposes the agent state machine through typed WASM APIs.
//! Every function returns a `ResultEnvelope<T>` — never throws.

use std::cell::Cell;
use std::cell::RefCell;

use wasm_bindgen::prelude::*;

#[allow(unused_imports)]
use tracing_subscriber::layer::SubscriberExt;
#[allow(unused_imports)]
use tracing_subscriber::util::SubscriberInitExt;

pub mod dto;
pub(crate) use dto::*;

mod host_state;
pub use host_state::{ArtifactSearchResult, HostState};

mod handle_table;
pub(crate) use handle_table::*;

pub mod directive;
pub(crate) use directive::*;

pub mod host_state_api;

pub mod host_agent_api;

pub mod artifact_api;

pub(crate) use pi_core::AgentRuntime;
pub(crate) use tracing::info;

thread_local! {
    pub(crate) static HOST_STATE_SLOTS: RefCell<Vec<Option<HostState>>> = const { RefCell::new(Vec::new()) };
    pub(crate) static HOST_AGENT_SLOTS: RefCell<Vec<Option<HostAgent>>> = const { RefCell::new(Vec::new()) };
    pub(crate) static TRACING_INIT: Cell<bool> = const { Cell::new(false) };
    pub(crate) static LOG_LEVEL: Cell<tracing::Level> = const { Cell::new(tracing::Level::INFO) };
}

pub(crate) fn init_tracing() {
    TRACING_INIT.with(|init| {
        if !init.get() {
            #[cfg(target_arch = "wasm32")]
            {
                let _ = tracing_subscriber::registry()
                    .with(tracing_wasm::WASMLayer::default())
                    .try_init();
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let _ = tracing_subscriber::fmt()
                    .with_max_level(tracing::Level::INFO)
                    .try_init();
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

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub(crate) enum HostError {
    #[error("agent not found: handle {0} is invalid")]
    BadHandle(u32),
    #[error("wrong phase: expected {expected}, got {actual}")]
    WrongPhase {
        expected: &'static str,
        actual: &'static str,
    },
    #[error("invalid session JSON")]
    InvalidSessionJson,
}

impl HostError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            HostError::BadHandle(_) => "bad_handle",
            HostError::WrongPhase { .. } => "wrong_phase",
            HostError::InvalidSessionJson => "invalid_session_json",
        }
    }
    pub(crate) fn to_dto(&self) -> ErrorDto {
        ErrorDto {
            code: self.code().to_string(),
            message: self.to_string(),
        }
    }
}

pub(crate) fn ok<T: serde::Serialize, R: for<'de> serde::Deserialize<'de>>(data: T) -> R {
    let mut map = serde_json::Map::new();
    map.insert("ok".to_string(), serde_json::Value::Bool(true));
    map.insert("data".to_string(), serde_json::to_value(data).unwrap());
    map.insert("error".to_string(), serde_json::Value::Null);
    serde_json::from_value(serde_json::Value::Object(map)).unwrap()
}

pub(crate) fn err<R: for<'de> serde::Deserialize<'de>>(e: &HostError) -> R {
    let mut map = serde_json::Map::new();
    map.insert("ok".to_string(), serde_json::Value::Bool(false));
    map.insert("data".to_string(), serde_json::Value::Null);
    map.insert(
        "error".to_string(),
        serde_json::to_value(e.to_dto()).unwrap(),
    );
    serde_json::from_value(serde_json::Value::Object(map)).unwrap()
}

pub(crate) fn dto_err<R: for<'de> serde::Deserialize<'de>>(e: serde_json::Error) -> R {
    let mut map = serde_json::Map::new();
    map.insert("ok".to_string(), serde_json::Value::Bool(false));
    map.insert("data".to_string(), serde_json::Value::Null);
    map.insert(
        "error".to_string(),
        serde_json::to_value(ErrorDto {
            code: "dto_conversion".to_string(),
            message: format!("DTO conversion failed: {}", e),
        })
        .unwrap(),
    );
    serde_json::from_value(serde_json::Value::Object(map)).unwrap()
}

macro_rules! try_conv {
    ($expr:expr) => {
        match $expr {
            Ok(v) => v,
            Err(e) => return dto_err(e),
        }
    };
}

pub(crate) use try_conv;

pub(crate) fn runtime_phase_name(runtime: &AgentRuntime) -> &'static str {
    match runtime {
        AgentRuntime::Idle(_) => "Idle",
        AgentRuntime::Streaming(_) => "Streaming",
        AgentRuntime::Compacting(_) => "Compacting",
        AgentRuntime::WaitingTools(_) => "WaitingTools",
        AgentRuntime::ReadyToContinue(_) => "ReadyToContinue",
        AgentRuntime::Finished(_) => "Finished",
        AgentRuntime::Aborted(_) => "Aborted",
    }
}
