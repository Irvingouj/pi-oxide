//! Mobile host scaffold for pi-core.
//!
//! This crate is a thin wrapper that re-exports pi-bindings types.
//! The actual iOS/Android integration consumes the C ABI from pi-bindings.

pub use pi_bindings::*;
