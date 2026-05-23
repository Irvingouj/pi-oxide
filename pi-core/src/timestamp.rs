//! Platform-aware timestamp for pi-core.
//!
//! On wasm32 without WASI, `SystemTime::now()` panics because there is no
//! system clock. We provide a monotonic counter instead. This keeps pi-core
//! runtime-neutral — the counter is deterministic and requires no external
//! APIs.

#[cfg(target_arch = "wasm32")]
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(target_arch = "wasm32")]
static COUNTER: AtomicU64 = AtomicU64::new(1);

/// Return a monotonically increasing timestamp value.
///
/// On native targets this uses `SystemTime::now()` (millis since epoch).
/// On `wasm32-unknown-unknown` this uses an in-process counter to avoid
/// panicking.
pub fn current_timestamp() -> u64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    #[cfg(target_arch = "wasm32")]
    {
        COUNTER.fetch_add(1, Ordering::Relaxed)
    }
}
