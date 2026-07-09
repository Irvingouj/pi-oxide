//! Integration test crate for pi-host-tui (pio) end-to-end PTY tests.
//!
//! Spawns the `pio` binary in a real pseudo-terminal, sends keystrokes,
//! and verifies rendered output. Requires Unix (macOS/Linux).

mod helpers;

#[cfg(test)]
mod e2e;

#[cfg(test)]
mod record_replay;

#[cfg(test)]
mod replay_e2e;
