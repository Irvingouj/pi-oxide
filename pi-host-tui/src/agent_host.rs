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

/// A tool call collected from the LLM stream.
pub struct CollectedToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

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

impl
    From<(
        Vec<AgentEvent>,
        Vec<AgentAction>,
        AgentRuntime,
        Vec<TrimmedMessage>,
        Artifacts,
        u32,
        Vec<ChangeMarker>,
    )> for TransitionParts
{
    fn from(
        (events, actions, runtime, transcript, artifacts, turn_number, markers): (
            Vec<AgentEvent>,
            Vec<AgentAction>,
            AgentRuntime,
            Vec<TrimmedMessage>,
            Artifacts,
            u32,
            Vec<ChangeMarker>,
        ),
    ) -> Self {
        Self {
            events,
            actions,
            runtime,
            transcript,
            artifacts,
            turn_number,
            markers,
        }
    }
}

/// Result of a transition: the side-channel outputs the host needs to handle.
pub type TransitionOutput = (Vec<AgentEvent>, Vec<AgentAction>);

/// Owns the agent runtime and its associated host state.
///
/// The runtime is stored directly (not `Option`).  Transitions use
/// `std::mem::replace` with `AgentRuntime::Uninitialized` as a
/// placeholder, so ownership can be moved out without ever producing
/// a `None`.
pub struct AgentHost {
    runtime: AgentRuntime,
    pub transcript: Vec<TrimmedMessage>,
    pub artifacts: Artifacts,
    pub turn_number: u32,
}

impl AgentHost {
    /// Create a new AgentHost with the given runtime and empty state.
    pub fn new(runtime: AgentRuntime) -> Self {
        Self {
            runtime,
            transcript: Vec::new(),
            artifacts: Artifacts::new(),
            turn_number: 0,
        }
    }

    /// Get a reference to the current runtime.
    pub fn runtime(&self) -> &AgentRuntime {
        &self.runtime
    }

    /// Get a mutable reference to the current runtime.
    pub fn runtime_mut(&mut self) -> &mut AgentRuntime {
        &mut self.runtime
    }

    /// Execute a state transition, replacing host state with the result.
    ///
    /// The closure receives the current runtime and state, and must return
    /// a `TransitionParts` (the result of `.into_parts()` converted via `From`).
    ///
    /// Uses `std::mem::replace` with `AgentRuntime::Uninitialized` as a
    /// placeholder so ownership can move out safely without `Option`.
    ///
    /// Returns the events and actions for the host to handle.
    pub fn transition(
        &mut self,
        f: impl FnOnce(AgentRuntime, Vec<TrimmedMessage>, Artifacts, u32) -> TransitionParts,
    ) -> TransitionOutput {
        let runtime = std::mem::replace(&mut self.runtime, AgentRuntime::Uninitialized);
        let transcript = std::mem::take(&mut self.transcript);
        let artifacts = std::mem::take(&mut self.artifacts);
        let turn_number = self.turn_number;

        let parts = f(runtime, transcript, artifacts, turn_number);
        if !parts.markers.is_empty() {
            tracing::debug!(
                marker_count = parts.markers.len(),
                "transition produced change markers"
            );
        }

        self.runtime = parts.runtime;
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
                let (ev, act, state, transcript, artifacts, tn, m) =
                    compacting.abort(transcript, artifacts, turn).into_parts();
                TransitionParts::from((ev, act, state.into_runtime(), transcript, artifacts, tn, m))
            }
            other => {
                tracing::debug!(
                    "abort_compacting_or_pass_through: non-Compacting runtime, passing through"
                );
                TransitionParts::from((vec![], vec![], other, transcript, artifacts, turn, vec![]))
            }
        }
    }

    /// Reset the agent to Idle, clearing transcript and artifacts.
    pub fn reset(&mut self) {
        let runtime = std::mem::replace(&mut self.runtime, AgentRuntime::Uninitialized).reset();
        self.transcript.clear();
        self.artifacts.clear();
        self.turn_number = 0;
        self.runtime = runtime;
    }

    /// Take ownership of the current runtime (replaces with Uninitialized).
    /// Used when the streaming lifecycle needs to extract StreamingAgent.
    pub fn take_runtime(&mut self) -> AgentRuntime {
        std::mem::replace(&mut self.runtime, AgentRuntime::Uninitialized)
    }

    /// Install a new runtime in place of whatever is there now.
    /// Used after the streaming lifecycle transitions to its next state.
    pub fn set_runtime(&mut self, runtime: AgentRuntime) {
        let _ = std::mem::replace(&mut self.runtime, runtime);
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
}
