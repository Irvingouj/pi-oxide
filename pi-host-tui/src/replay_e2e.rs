//! Deterministic E2E replay tests — no network, no API keys.

use crate::llm::{LlmProvider, LlmStreamState};
use crate::llm_replay::ReplayLlmClient;

fn load_cassette(name: &str) -> ReplayLlmClient {
    let path = std::path::PathBuf::from(format!("tests/fixtures/{name}"));
    ReplayLlmClient::load(&path).expect("cassette should load")
}

#[test]
fn replay_simple_greeting() {
    let client = load_cassette("explore_project.json");
    assert_eq!(
        client.model_id(),
        "accounts/fireworks/routers/kimi-k2p6-turbo"
    );

    let stream = client
        .stream_sync("system", &[], &[])
        .expect("stream_sync should succeed");

    let chunks: Vec<_> = stream.collect();
    assert!(!chunks.is_empty(), "should have at least one chunk");
}

#[test]
fn replay_preserves_stop_reason() {
    let client = load_cassette("explore_project.json");
    let mut stream = client.stream_sync("system", &[], &[]).unwrap();

    while stream.next().is_some() {}

    assert_eq!(stream.stop_reason(), Some("end_turn"));
}

#[test]
fn replay_exhausts_cassette() {
    let client = load_cassette("explore_project.json");

    // Use the single entry
    let stream = client.stream_sync("system", &[], &[]).unwrap();
    let _: Vec<_> = stream.collect();

    // Second call should fail
    let result = client.stream_sync("system", &[], &[]);
    assert!(result.is_err(), "cassette should be exhausted");
}
