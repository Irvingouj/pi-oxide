//! Typestate wrapper around [`Agent`].
//!
//! Each phase struct exposes only valid operations for that phase,
//! making illegal state transitions a compile-time error in Rust hosts.
//!
//! The underlying [`Agent`] API is unchanged; this layer is additive.

use crate::agent::{Agent, Phase};
use crate::events::{AgentAction, AgentEvent, CancelReason, ToolExecutionUpdate};
use crate::llm::{LlmChunk, LlmResult};
use crate::message::AgentMessage;
use crate::session::SessionState;
use crate::tool::{ToolError, ToolResult};
use crate::types::ToolCallId;

// ---------------------------------------------------------------------------
// Transition types
// ---------------------------------------------------------------------------

/// A state change carrying events and actions.
pub struct Transition<T> {
    pub events: Vec<AgentEvent>,
    pub actions: Vec<AgentAction>,
    pub state: T,
}

/// Outcome of finishing an LLM stream.
pub enum FinishLlmTransition {
    MoreStreaming(Transition<StreamingAgent>),
    WaitingTools(Transition<WaitingToolsAgent>),
    Ready(Transition<ReadyAgent>),
    Finished(Transition<FinishedAgent>),
    Aborted(Transition<AbortedAgent>),
}

impl FinishLlmTransition {
    /// Destructure into the common parts regardless of which phase we land in.
    pub fn into_parts(self) -> (Vec<AgentEvent>, Vec<AgentAction>, AgentRuntime) {
        match self {
            FinishLlmTransition::MoreStreaming(t) => {
                let Transition {
                    events,
                    actions,
                    state,
                } = t;
                (events, actions, state.into_runtime())
            }
            FinishLlmTransition::WaitingTools(t) => {
                let Transition {
                    events,
                    actions,
                    state,
                } = t;
                (events, actions, state.into_runtime())
            }
            FinishLlmTransition::Ready(t) => {
                let Transition {
                    events,
                    actions,
                    state,
                } = t;
                (events, actions, state.into_runtime())
            }
            FinishLlmTransition::Finished(t) => {
                let Transition {
                    events,
                    actions,
                    state,
                } = t;
                (events, actions, state.into_runtime())
            }
            FinishLlmTransition::Aborted(t) => {
                let Transition {
                    events,
                    actions,
                    state,
                } = t;
                (events, actions, state.into_runtime())
            }
        }
    }
}

/// Outcome of resolving a tool call.
pub enum ToolTransition {
    WaitingTools(Transition<WaitingToolsAgent>),
    Ready(Transition<ReadyAgent>),
    Finished(Transition<FinishedAgent>),
}

impl ToolTransition {
    pub fn into_parts(self) -> (Vec<AgentEvent>, Vec<AgentAction>, AgentRuntime) {
        match self {
            ToolTransition::WaitingTools(t) => {
                let Transition {
                    events,
                    actions,
                    state,
                } = t;
                (events, actions, state.into_runtime())
            }
            ToolTransition::Ready(t) => {
                let Transition {
                    events,
                    actions,
                    state,
                } = t;
                (events, actions, state.into_runtime())
            }
            ToolTransition::Finished(t) => {
                let Transition {
                    events,
                    actions,
                    state,
                } = t;
                (events, actions, state.into_runtime())
            }
        }
    }
}

/// Core's disposition for user input while async tools are pending.
pub enum UserInputDuringTools {
    QueuedFollowUp(Vec<AgentEvent>),
    AppliedSteering(Vec<AgentEvent>),
    InterruptRequested {
        events: Vec<AgentEvent>,
        actions: Vec<AgentAction>,
    },
}

impl UserInputDuringTools {
    pub fn into_events_actions(self) -> (Vec<AgentEvent>, Vec<AgentAction>) {
        match self {
            UserInputDuringTools::QueuedFollowUp(events) => (events, vec![]),
            UserInputDuringTools::AppliedSteering(events) => (events, vec![]),
            UserInputDuringTools::InterruptRequested { events, actions } => (events, actions),
        }
    }
}

// ---------------------------------------------------------------------------
// AgentRuntime — outer enum that hosts store
// ---------------------------------------------------------------------------

pub enum AgentRuntime {
    Idle(IdleAgent),
    Streaming(StreamingAgent),
    WaitingTools(WaitingToolsAgent),
    ReadyToContinue(ReadyAgent),
    Finished(FinishedAgent),
    Aborted(AbortedAgent),
}

impl AgentRuntime {
    /// Wrap an existing agent. The agent must be in a valid phase.
    pub fn from_agent(agent: Agent) -> Self {
        match agent.phase {
            Phase::Idle => {
                if agent.state().pending_tool_calls.is_empty() {
                    AgentRuntime::Idle(IdleAgent { agent })
                } else {
                    AgentRuntime::WaitingTools(WaitingToolsAgent { agent })
                }
            }
            Phase::Streaming => AgentRuntime::Streaming(StreamingAgent { agent }),
            Phase::WaitForInput => AgentRuntime::ReadyToContinue(ReadyAgent { agent }),
        }
    }

    /// Construct from options (starts in Idle).
    pub fn new(options: crate::agent::AgentOptions) -> Self {
        Self::from_agent(Agent::new(options))
    }

    /// Read-only access to public agent state.
    pub fn state(&self) -> &crate::agent::AgentState {
        match self {
            AgentRuntime::Idle(a) => a.agent.state(),
            AgentRuntime::Streaming(a) => a.agent.state(),
            AgentRuntime::WaitingTools(a) => a.agent.state(),
            AgentRuntime::ReadyToContinue(a) => a.agent.state(),
            AgentRuntime::Finished(a) => a.agent.state(),
            AgentRuntime::Aborted(a) => a.agent.state(),
        }
    }

    /// Mutable access to public agent state (use sparingly).
    pub fn state_mut(&mut self) -> &mut crate::agent::AgentState {
        match self {
            AgentRuntime::Idle(a) => a.agent.state_mut(),
            AgentRuntime::Streaming(a) => a.agent.state_mut(),
            AgentRuntime::WaitingTools(a) => a.agent.state_mut(),
            AgentRuntime::ReadyToContinue(a) => a.agent.state_mut(),
            AgentRuntime::Finished(a) => a.agent.state_mut(),
            AgentRuntime::Aborted(a) => a.agent.state_mut(),
        }
    }

    /// Consume the runtime and return the underlying agent.
    pub fn into_agent(self) -> Agent {
        match self {
            AgentRuntime::Idle(a) => a.agent,
            AgentRuntime::Streaming(a) => a.agent,
            AgentRuntime::WaitingTools(a) => a.agent,
            AgentRuntime::ReadyToContinue(a) => a.agent,
            AgentRuntime::Finished(a) => a.agent,
            AgentRuntime::Aborted(a) => a.agent,
        }
    }

    /// Reset the agent back to an idle state regardless of current phase.
    pub fn reset(self) -> Self {
        let mut agent = self.into_agent();
        agent.reset();
        Self::from_agent(agent)
    }

    /// Read-only access to the session tree.
    pub fn session_state(&self) -> &SessionState {
        match self {
            AgentRuntime::Idle(a) => a.agent.session_state(),
            AgentRuntime::Streaming(a) => a.agent.session_state(),
            AgentRuntime::WaitingTools(a) => a.agent.session_state(),
            AgentRuntime::ReadyToContinue(a) => a.agent.session_state(),
            AgentRuntime::Finished(a) => a.agent.session_state(),
            AgentRuntime::Aborted(a) => a.agent.session_state(),
        }
    }

    /// Replace the in-memory session tree.
    pub fn set_session_state(&mut self, state: SessionState) {
        match self {
            AgentRuntime::Idle(a) => a.agent.set_session_state(state),
            AgentRuntime::Streaming(a) => a.agent.set_session_state(state),
            AgentRuntime::WaitingTools(a) => a.agent.set_session_state(state),
            AgentRuntime::ReadyToContinue(a) => a.agent.set_session_state(state),
            AgentRuntime::Finished(a) => a.agent.set_session_state(state),
            AgentRuntime::Aborted(a) => a.agent.set_session_state(state),
        }
    }

    /// Forward a tool execution update if in the WaitingTools phase.
    pub fn on_tool_update(&mut self, update: ToolExecutionUpdate) -> Vec<AgentEvent> {
        match self {
            AgentRuntime::WaitingTools(a) => a.on_tool_update(update),
            _ => vec![],
        }
    }

    /// Forward a tool started notification if in the WaitingTools phase.
    pub fn on_tool_started(&mut self, id: ToolCallId) -> Vec<AgentEvent> {
        match self {
            AgentRuntime::WaitingTools(a) => a.on_tool_started(id),
            _ => vec![],
        }
    }
}

// ---------------------------------------------------------------------------
// IdleAgent — waiting for a new user turn
// ---------------------------------------------------------------------------

pub struct IdleAgent {
    pub(crate) agent: Agent,
}

impl IdleAgent {
    pub fn into_agent(self) -> Agent {
        self.agent
    }

    pub fn start_turn(mut self, msg: AgentMessage) -> Transition<StreamingAgent> {
        let (events, actions) = self.agent.start_turn(msg);
        Transition {
            events,
            actions,
            state: StreamingAgent { agent: self.agent },
        }
    }

    pub fn steer(&mut self, message: AgentMessage) -> Vec<AgentEvent> {
        self.agent.steer(message)
    }

    pub fn follow_up(&mut self, message: AgentMessage) {
        self.agent.follow_up(message)
    }

    pub fn reset(mut self) -> Transition<IdleAgent> {
        self.agent.reset();
        Transition {
            events: vec![],
            actions: vec![],
            state: IdleAgent { agent: self.agent },
        }
    }

    pub fn into_runtime(self) -> AgentRuntime {
        AgentRuntime::Idle(self)
    }
}

// ---------------------------------------------------------------------------
// StreamingAgent — LLM stream is in progress
// ---------------------------------------------------------------------------

pub struct StreamingAgent {
    pub(crate) agent: Agent,
}

impl StreamingAgent {
    pub fn into_agent(self) -> Agent {
        self.agent
    }

    pub fn feed_llm_chunk(&mut self, chunk: LlmChunk) -> Vec<AgentEvent> {
        self.agent.feed_llm_chunk(chunk)
    }

    pub fn finish_llm(mut self, result: LlmResult) -> FinishLlmTransition {
        let (events, actions) = self.agent.on_llm_done(result);

        // Abort / error path
        if self.agent.state().error_message.is_some() {
            return FinishLlmTransition::Aborted(Transition {
                events,
                actions,
                state: AbortedAgent { agent: self.agent },
            });
        }

        // Steering / follow-up may have triggered more streaming
        let has_stream_llm = actions
            .iter()
            .any(|a| matches!(a, AgentAction::StreamLlm { .. }));
        if has_stream_llm {
            return FinishLlmTransition::MoreStreaming(Transition {
                events,
                actions,
                state: StreamingAgent { agent: self.agent },
            });
        }

        // Tool execution requested — host decides sync vs async; core always waits
        if actions
            .iter()
            .any(|a| matches!(a, AgentAction::ExecuteTools { .. }))
        {
            return FinishLlmTransition::WaitingTools(Transition {
                events,
                actions,
                state: WaitingToolsAgent { agent: self.agent },
            });
        }

        // Turn finished without tools
        let is_finished = actions
            .iter()
            .any(|a| matches!(a, AgentAction::Finished { .. }));
        if is_finished {
            return FinishLlmTransition::Finished(Transition {
                events,
                actions,
                state: FinishedAgent { agent: self.agent },
            });
        }

        // Default: ready to continue
        FinishLlmTransition::Ready(Transition {
            events,
            actions,
            state: ReadyAgent { agent: self.agent },
        })
    }

    pub fn abort(mut self) -> Transition<AbortedAgent> {
        let events = self.agent.abort();
        Transition {
            events,
            actions: vec![],
            state: AbortedAgent { agent: self.agent },
        }
    }

    pub fn into_runtime(self) -> AgentRuntime {
        AgentRuntime::Streaming(self)
    }
}

// ---------------------------------------------------------------------------
// WaitingToolsAgent — deferred tools are in flight
// ---------------------------------------------------------------------------

pub struct WaitingToolsAgent {
    pub(crate) agent: Agent,
}

impl WaitingToolsAgent {
    pub fn into_agent(self) -> Agent {
        self.agent
    }

    pub fn on_tool_done(
        mut self,
        id: ToolCallId,
        result: Result<ToolResult, ToolError>,
    ) -> ToolTransition {
        let (events, actions) = self.agent.on_tool_done(id, result);

        if actions
            .iter()
            .any(|a| matches!(a, AgentAction::Finished { .. }))
        {
            return ToolTransition::Finished(Transition {
                events,
                actions,
                state: FinishedAgent { agent: self.agent },
            });
        }

        if self.agent.state().pending_tool_calls.is_empty() {
            return ToolTransition::Ready(Transition {
                events,
                actions,
                state: ReadyAgent { agent: self.agent },
            });
        }

        ToolTransition::WaitingTools(Transition {
            events,
            actions,
            state: WaitingToolsAgent { agent: self.agent },
        })
    }

    pub fn on_tool_update(&mut self, update: ToolExecutionUpdate) -> Vec<AgentEvent> {
        self.agent.on_tool_update(update)
    }

    pub fn on_tool_started(&mut self, id: ToolCallId) -> Vec<AgentEvent> {
        self.agent.on_tool_started(id)
    }

    pub fn cancel_tool(mut self, id: ToolCallId, reason: CancelReason) -> ToolTransition {
        let (events, actions) = self.agent.on_tool_cancelled(id, reason);

        if actions
            .iter()
            .any(|a| matches!(a, AgentAction::Finished { .. }))
        {
            return ToolTransition::Finished(Transition {
                events,
                actions,
                state: FinishedAgent { agent: self.agent },
            });
        }

        if self.agent.state().pending_tool_calls.is_empty() {
            return ToolTransition::Ready(Transition {
                events,
                actions,
                state: ReadyAgent { agent: self.agent },
            });
        }

        ToolTransition::WaitingTools(Transition {
            events,
            actions,
            state: WaitingToolsAgent { agent: self.agent },
        })
    }

    pub fn submit_user_message(&mut self, msg: AgentMessage) -> UserInputDuringTools {
        // Simple heuristic: user messages sent while tools are pending
        // are treated as follow-up queue entries.
        // Future refinement: inspect msg intent for steering vs interrupt.
        self.agent.follow_up(msg);
        UserInputDuringTools::QueuedFollowUp(vec![])
    }

    pub fn into_runtime(self) -> AgentRuntime {
        AgentRuntime::WaitingTools(self)
    }
}

// ---------------------------------------------------------------------------
// ReadyAgent — all tools done; ready to continue the turn
// ---------------------------------------------------------------------------

pub struct ReadyAgent {
    pub(crate) agent: Agent,
}

impl ReadyAgent {
    pub fn into_agent(self) -> Agent {
        self.agent
    }

    pub fn continue_turn(mut self) -> Transition<StreamingAgent> {
        let (events, actions) = self.agent.continue_turn();
        Transition {
            events,
            actions,
            state: StreamingAgent { agent: self.agent },
        }
    }

    pub fn wait_for_input(self) -> Transition<IdleAgent> {
        Transition {
            events: vec![],
            actions: vec![],
            state: IdleAgent { agent: self.agent },
        }
    }

    pub fn steer(&mut self, message: AgentMessage) -> Vec<AgentEvent> {
        self.agent.steer(message)
    }

    pub fn follow_up(&mut self, message: AgentMessage) {
        self.agent.follow_up(message)
    }

    pub fn into_runtime(self) -> AgentRuntime {
        AgentRuntime::ReadyToContinue(self)
    }
}

// ---------------------------------------------------------------------------
// FinishedAgent — turn completed successfully
// ---------------------------------------------------------------------------

pub struct FinishedAgent {
    pub(crate) agent: Agent,
}

impl FinishedAgent {
    pub fn into_agent(self) -> Agent {
        self.agent
    }

    pub fn restart(mut self) -> Transition<IdleAgent> {
        self.agent.reset();
        Transition {
            events: vec![],
            actions: vec![],
            state: IdleAgent { agent: self.agent },
        }
    }

    pub fn into_runtime(self) -> AgentRuntime {
        AgentRuntime::Finished(self)
    }
}

// ---------------------------------------------------------------------------
// AbortedAgent — turn was aborted or errored
// ---------------------------------------------------------------------------

pub struct AbortedAgent {
    pub(crate) agent: Agent,
}

impl AbortedAgent {
    pub fn into_agent(self) -> Agent {
        self.agent
    }

    pub fn restart(mut self) -> Transition<IdleAgent> {
        self.agent.reset();
        Transition {
            events: vec![],
            actions: vec![],
            state: IdleAgent { agent: self.agent },
        }
    }

    pub fn into_runtime(self) -> AgentRuntime {
        AgentRuntime::Aborted(self)
    }
}
