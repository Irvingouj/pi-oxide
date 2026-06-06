#![allow(non_snake_case)]

mod common;
use common::*;

#[test]
fn text_delta_is_incremental_not_accumulated() {
    let runtime = AgentRuntime::new(dummy_options());
    let AgentRuntime::Idle(idle) = runtime else {
        panic!("expected Idle");
    };
    let (t, a, tn) = empty();
    let t = idle.start_turn(
        AgentMessage::user("hello"),
        vec![],
        t,
        a,
        tn,
        &ContextProjectionBudget::default(),
        "",
    );
    let StartTurnTransition::Streaming(t) = t else {
        panic!("expected Streaming")
    };
    let mut streaming = t.state;

    // Feed Start chunk to initialize the assistant message
    let start_chunk = LlmChunk::Start {
        partial: AssistantMessage::empty(),
    };
    let events = streaming.feed_llm_chunk(start_chunk);
    assert!(events
        .iter()
        .any(|e| matches!(e, AgentEvent::MessageStart { .. })));

    // Feed first text delta
    let events = streaming.feed_llm_chunk(LlmChunk::TextDelta {
        text: "Hello".into(),
    });
    let deltas: Vec<&ContentDelta> = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::MessageUpdate { delta, .. } => Some(delta),
            _ => None,
        })
        .collect();
    assert_eq!(deltas.len(), 1);
    assert!(
        matches!(deltas[0], ContentDelta::TextDelta { text } if text == "Hello"),
        "first delta should be the incremental chunk 'Hello', got {:?}",
        deltas[0]
    );

    // Feed second text delta
    let events = streaming.feed_llm_chunk(LlmChunk::TextDelta {
        text: " world".into(),
    });
    let deltas: Vec<&ContentDelta> = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::MessageUpdate { delta, .. } => Some(delta),
            _ => None,
        })
        .collect();
    assert_eq!(deltas.len(), 1);
    assert!(
        matches!(deltas[0], ContentDelta::TextDelta { text } if text == " world"),
        "second delta should be the incremental chunk ' world', got {:?}",
        deltas[0]
    );

    // Finish the turn so the message is complete
    let (T, A, tn) = empty();
    let transition = streaming.finish_llm(
        LlmResult::done(),
        T,
        A,
        tn,
        &ContextProjectionBudget::default(),
    );
    let (_events, actions, _runtime, _T, _A, _turn_number, _markers) = transition.into_parts();
    assert!(actions.iter().any(|a| matches!(a, AgentAction::Finished)));
}


