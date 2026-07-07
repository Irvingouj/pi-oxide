//! Host-side agent mediator.
//!
//! Owns the `AgentRuntime` and its associated host state (transcript, artifacts,
//! turn number). Provides a `transition()` method that encapsulates the
//! take/transition/restore pattern repeated across the TUI.
//!
//! Every agent state transition follows the same shape:
//! 1. Take ownership of current runtime + transcript + artifacts + turn_number
//! 2. Match the runtime variant and call the appropriate method
//! 3. `.into_parts()` → 7-tuple
//! 4. Store the new state back
//!
//! `AgentHost::transition()` handles steps 1 and 4, leaving the caller to
//! express only the variant match and method call.

use pi_core::{AgentAction, AgentEvent, AgentRuntime, Artifacts, ChangeMarker, TrimmedMessage};

/// Result of an agent state transition.
///
/// This is the structured output of every `.into_parts()` call on a transition
/// type. Using a struct instead of a 7-tuple makes field access self-documenting
/// and surfaces unused fields at compile time.
pub struct TransitionParts {
    pub events: Vec<AgentEvent>,
    pub actions: Vec<AgentAction>,
    pub runtime: AgentRuntime,
    pub transcript: Vec<TrimmedMessage>,
    pub artifacts: Artifacts,
    pub turn_number: u32,
    pub markers: Vec<ChangeMarker>,
}

impl From<(Vec<AgentEvent>, Vec<AgentAction>, AgentRuntime, Vec<TrimmedMessage>, Artifacts, u32, Vec<ChangeMarker>)> for TransitionParts {
    fn from((events, actions, runtime, transcript, artifacts, turn_number, markers): (Vec<AgentEvent>, Vec<AgentAction>, AgentRuntime, Vec<TrimmedMessage>, Artifacts, u32, Vec<ChangeMarker>)) -> Self {
        Self { events, actions, runtime, transcript, artifacts, turn_number, markers }
    }
}

/// Result of a transition: the side-channel outputs the host needs to handle.
pub type TransitionOutput = (Vec<AgentEvent>, Vec<AgentAction>);

/// Owns the agent runtime and its associated host state.
///
/// The runtime is `Option` because:
/// - Tests may construct `App` without an agent
/// - The transition pattern uses `take()` / put-back
pub struct AgentHost {
    runtime: Option<AgentRuntime>,
    pub transcript: Vec<TrimmedMessage>,
    pub artifacts: Artifacts,
    pub turn_number: u32,
}

impl AgentHost {
    /// Create a new AgentHost with the given runtime and empty state.
    pub fn new(runtime: AgentRuntime) -> Self {
        Self {
            runtime: Some(runtime),
            transcript: Vec::new(),
            artifacts: Artifacts::new(),
            turn_number: 0,
        }
    }

    /// Restore from persisted state.
    pub fn restore(
        runtime: AgentRuntime,
        transcript: Vec<TrimmedMessage>,
        artifacts: Artifacts,
        turn_number: u32,
    ) -> Self {
        Self {
            runtime: Some(runtime),
            transcript,
            artifacts,
            turn_number,
        }
    }

    /// Get a reference to the current runtime.
    pub fn runtime(&self) -> &AgentRuntime {
        self.runtime.as_ref().expect("agent runtime not set")
    }

    /// Get a mutable reference to the current runtime.
    pub fn runtime_mut(&mut self) -> &mut AgentRuntime {
        self.runtime.as_mut().expect("agent runtime not set")
    }

    /// Take the runtime out, leaving `None`. Use when you need to consume
    /// the runtime for a non-transition operation (e.g., `reset()`).
    pub fn take_runtime(&mut self) -> AgentRuntime {
        self.runtime.take().expect("agent runtime not set")
    }

    /// Put a runtime back.
    pub fn set_runtime(&mut self, runtime: AgentRuntime) {
        self.runtime = Some(runtime);
    }

    /// Execute a state transition, replacing host state with the result.
    ///
    /// The closure receives the current runtime and state, and must return
    /// a `TransitionParts` (the result of `.into_parts()` converted via `From`).
    ///
    /// Returns the events and actions for the host to handle.
    pub fn transition(
        &mut self,
        f: impl FnOnce(AgentRuntime, Vec<TrimmedMessage>, Artifacts, u32) -> TransitionParts,
    ) -> TransitionOutput {
        let runtime = self.runtime.take().expect("agent runtime not set");
        let transcript = std::mem::take(&mut self.transcript);
        let artifacts = std::mem::take(&mut self.artifacts);
        let turn_number = self.turn_number;

        let parts = f(runtime, transcript, artifacts, turn_number);
        if !parts.markers.is_empty() {
            tracing::debug!(marker_count = parts.markers.len(), "transition produced change markers");
        }

        self.runtime = Some(parts.runtime);
        self.transcript = parts.transcript;
        self.artifacts = parts.artifacts;
        self.turn_number = parts.turn_number;

        (parts.events, parts.actions)
    }

    /// Abort a Compacting runtime, or pass-through other runtime states unchanged.
    /// Used as a fallback in transition closures when the expected variant is absent.
    pub fn abort_compacting_or_pass_through(
        runtime: AgentRuntime,
        transcript: Vec<TrimmedMessage>,
        artifacts: Artifacts,
        turn: u32,
    ) -> TransitionParts {
        match runtime {
            AgentRuntime::Compacting(compacting) => {
                let (ev, act, state, transcript, artifacts, tn, m) = compacting
                    .abort(transcript, artifacts, turn)
                    .into_parts();
                TransitionParts::from((ev, act, state.into_runtime(), transcript, artifacts, tn, m))
            }
            other => {
                tracing::debug!("abort_compacting_or_pass_through: non-Compacting runtime, passing through");
                TransitionParts::from((vec![], vec![], other, transcript, artifacts, turn, vec![]))
            }
        }
    }

    /// Reset the agent to Idle, clearing transcript and artifacts.
    pub fn reset(&mut self) {
        let runtime = self.take_runtime().reset();
        self.transcript.clear();
        self.artifacts.clear();
        self.turn_number = 0;
        self.set_runtime(runtime);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_runtime() -> AgentRuntime {
        pi_core::AgentRuntime::new(pi_core::AgentOptions {
            system_prompt: "test".into(),
            model: pi_core::Model {
                id: pi_core::ModelId::new("test"),
                name: pi_core::ModelName::new("test"),
                api: pi_core::ApiName::new("openai"),
                provider: pi_core::ProviderName::new("openai"),
                base_url: None,
                reasoning: false,
                context_window: 128_000,
                max_tokens: 8192,
                capabilities: Default::default(),
                cost: Default::default(),
            },
            thinking_level: Default::default(),
            steering_mode: Default::default(),
            follow_up_mode: Default::default(),
            tool_execution_mode: Default::default(),
            session_id: None,
        })
    }

    #[test]
    fn new_has_empty_state() {
        let host = AgentHost::new(dummy_runtime());
        assert!(host.transcript.is_empty());
        assert!(host.artifacts.is_empty());
        assert_eq!(host.turn_number, 0);
    }

    #[test]
    fn transition_preserves_state_when_noop() {
        let mut host = AgentHost::new(dummy_runtime());
        host.turn_number = 5;
        host.transcript.push(TrimmedMessage::User(
            pi_core::message::UserMessage::new_text("hello"),
        ));

        let (events, actions) = host.transition(|runtime, transcript, artifacts, turn| {
            TransitionParts::from((vec![], vec![], runtime, transcript, artifacts, turn, vec![]))
        });

        assert!(events.is_empty());
        assert!(actions.is_empty());
        assert_eq!(host.turn_number, 5);
        assert_eq!(host.transcript.len(), 1);
    }

    #[test]
    fn transition_can_modify_state() {
        let mut host = AgentHost::new(dummy_runtime());

        host.transition(|runtime, transcript, artifacts, turn| {
            TransitionParts::from((
                vec![],
                vec![],
                runtime,
                transcript,
                artifacts,
                turn + 1,
                vec![],
            ))
        });

        assert_eq!(host.turn_number, 1);
    }

    #[test]
    fn restore_roundtrip() {
        let runtime = dummy_runtime();
        let transcript = vec![TrimmedMessage::User(
            pi_core::message::UserMessage::new_text("hello"),
        )];
        let artifacts = Artifacts::new();
        let turn_number = 3;

        let host = AgentHost::restore(runtime, transcript.clone(), artifacts, turn_number);
        assert_eq!(host.turn_number, 3);
        assert_eq!(host.transcript.len(), 1);
    }
}
