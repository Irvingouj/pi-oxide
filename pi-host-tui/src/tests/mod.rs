/// Unit and integration tests for pi-host-tui.
///
/// These tests need access to private crate internals and therefore live
/// inside `src/` rather than the top-level `tests/` integration test directory.

#[cfg(test)]
#[cfg(not(feature = "replay"))]
mod input;

#[cfg(test)]
mod tools;

#[cfg(test)]
mod onboarding;

#[cfg(test)]
mod smoke;

#[cfg(test)]
mod llm;
