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
use crate::tool::{ToolDefinition, ToolError, ToolResult};
use crate::types::ToolCallId;

// ---------------------------------------------------------------------------
// Transition types
// ---------------------------------------------------------------------------

/// A state change carrying events and actions.
pub struct Transition<T> {
    pub events: Vec<AgentEvent>,
    pub actions: Vec<AgentAction>,
    pub state: T,
    pub session_state: SessionState,
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
    pub fn into_parts(self) -> (Vec<AgentEvent>, Vec<AgentAction>, AgentRuntime, SessionState) {
        match self {
            FinishLlmTransition::MoreStreaming(t) => {
                let Transition {
                    events,
                    actions,
                    state,
                    session_state,
                } = t;
                (events, actions, state.into_runtime(), session_state)
            }
            FinishLlmTransition::WaitingTools(t) => {
                let Transition {
                    events,
                    actions,
                    state,
                    session_state,
                } = t;
                (events, actions, state.into_runtime(), session_state)
            }
            FinishLlmTransition::Ready(t) => {
                let Transition {
                    events,
                    actions,
                    state,
                    session_state,
                } = t;
                (events, actions, state.into_runtime(), session_state)
            }
            FinishLlmTransition::Finished(t) => {
                let Transition {
                    events,
                    actions,
                    state,
                    session_state,
                } = t;
                (events, actions, state.into_runtime(), session_state)
            }
            FinishLlmTransition::Aborted(t) => {
                let Transition {
                    events,
                    actions,
                    state,
                    session_state,
                } = t;
                (events, actions, state.into_runtime(), session_state)
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
    pub fn into_parts(self) -> (Vec<AgentEvent>, Vec<AgentAction>, AgentRuntime, SessionState) {
        match self {
            ToolTransition::WaitingTools(t) => {
                let Transition {
                    events,
                    actions,
                    state,
                    session_state,
                } = t;
                (events, actions, state.into_runtime(), session_state)
            }
            ToolTransition::Ready(t) => {
                let Transition {
                    events,
                    actions,
                    state,
                    session_state,
                } = t;
                (events, actions, state.into_runtime(), session_state)
            }
            ToolTransition::Finished(t) => {
                let Transition {
                    events,
                    actions,
                    state,
                    session_state,
                } = t;
                (events, actions, state.into_runtime(), session_state)
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

    fn as_agent(&self) -> &Agent {
        match self {
            AgentRuntime::Idle(a) => &a.agent,
            AgentRuntime::Streaming(a) => &a.agent,
            AgentRuntime::WaitingTools(a) => &a.agent,
            AgentRuntime::ReadyToContinue(a) => &a.agent,
            AgentRuntime::Finished(a) => &a.agent,
            AgentRuntime::Aborted(a) => &a.agent,
        }
    }

    fn as_agent_mut(&mut self) -> &mut Agent {
        match self {
            AgentRuntime::Idle(a) => &mut a.agent,
            AgentRuntime::Streaming(a) => &mut a.agent,
            AgentRuntime::WaitingTools(a) => &mut a.agent,
            AgentRuntime::ReadyToContinue(a) => &mut a.agent,
            AgentRuntime::Finished(a) => &mut a.agent,
            AgentRuntime::Aborted(a) => &mut a.agent,
        }
    }

    /// Read-only access to public agent state.
    pub fn state(&self) -> &crate::agent::AgentState {
        self.as_agent().state()
    }

    /// Mutable access to public agent state (use sparingly).
    pub fn state_mut(&mut self) -> &mut crate::agent::AgentState {
        self.as_agent_mut().state_mut()
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

    /// Forward a tool execution update.
    pub fn on_tool_update(&mut self, update: ToolExecutionUpdate) -> Vec<AgentEvent> {
        self.as_agent_mut().on_tool_update(update)
    }

    /// Forward a tool started notification.
    pub fn on_tool_started(&mut self, id: ToolCallId) -> Vec<AgentEvent> {
        self.as_agent_mut().on_tool_started(id)
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

    pub fn start_turn(
        mut self,
        msg: AgentMessage,
        tools: Vec<ToolDefinition>,
        mut session_state: SessionState,
    ) -> Transition<StreamingAgent> {
        let (events, actions) = self.agent.start_turn(msg, tools, &mut session_state);
        Transition {
            events,
            actions,
            state: StreamingAgent { agent: self.agent },
            session_state,
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
            session_state: SessionState::default(),
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

    pub fn finish_llm(mut self, result: LlmResult, mut session_state: SessionState) -> FinishLlmTransition {
        let (events, actions) = self.agent.on_llm_done(result, &mut session_state);

        // Abort / error path
        if self.agent.state().error_message.is_some() {
            return FinishLlmTransition::Aborted(Transition {
                events,
                actions,
                state: AbortedAgent { agent: self.agent },
                session_state,
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
                session_state,
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
                session_state,
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
                session_state,
            });
        }

        // Default: ready to continue
        FinishLlmTransition::Ready(Transition {
            events,
            actions,
            state: ReadyAgent { agent: self.agent },
            session_state,
        })
    }

    pub fn abort(mut self, session_state: SessionState) -> Transition<AbortedAgent> {
        let events = self.agent.abort();
        Transition {
            events,
            actions: vec![],
            state: AbortedAgent { agent: self.agent },
            session_state,
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
        mut session_state: SessionState,
    ) -> ToolTransition {
        let (events, actions) = self.agent.on_tool_done(id, result, &mut session_state);

        if actions
            .iter()
            .any(|a| matches!(a, AgentAction::Finished { .. }))
        {
            return ToolTransition::Finished(Transition {
                events,
                actions,
                state: FinishedAgent { agent: self.agent },
                session_state,
            });
        }

        if self.agent.state().pending_tool_calls.is_empty() {
            return ToolTransition::Ready(Transition {
                events,
                actions,
                state: ReadyAgent { agent: self.agent },
                session_state,
            });
        }

        ToolTransition::WaitingTools(Transition {
            events,
            actions,
            state: WaitingToolsAgent { agent: self.agent },
            session_state,
        })
    }

    pub fn on_tool_update(&mut self, update: ToolExecutionUpdate) -> Vec<AgentEvent> {
        self.agent.on_tool_update(update)
    }

    pub fn on_tool_started(&mut self, id: ToolCallId) -> Vec<AgentEvent> {
        self.agent.on_tool_started(id)
    }

    pub fn cancel_tool(mut self, id: ToolCallId, reason: CancelReason, mut session_state: SessionState) -> ToolTransition {
        let (events, actions) = self.agent.on_tool_cancelled(id, reason, &mut session_state);

        if actions
            .iter()
            .any(|a| matches!(a, AgentAction::Finished { .. }))
        {
            return ToolTransition::Finished(Transition {
                events,
                actions,
                state: FinishedAgent { agent: self.agent },
                session_state,
            });
        }

        if self.agent.state().pending_tool_calls.is_empty() {
            return ToolTransition::Ready(Transition {
                events,
                actions,
                state: ReadyAgent { agent: self.agent },
                session_state,
            });
        }

        ToolTransition::WaitingTools(Transition {
            events,
            actions,
            state: WaitingToolsAgent { agent: self.agent },
            session_state,
        })
    }

    pub fn submit_user_message(&mut self, msg: AgentMessage) -> UserInputDuringTools {
        // Simple heuristic: user messages sent while tools are pending
        // are treated as follow-up queue entries.
        // Future refinement: inspect msg intent for steering vs interrupt.
        self.agent.follow_up(msg);
        UserInputDuringTools::QueuedFollowUp(vec![])
    }

    pub fn abort(mut self, session_state: SessionState) -> Transition<AbortedAgent> {
        let events = self.agent.abort();
        Transition {
            events,
            actions: vec![],
            state: AbortedAgent { agent: self.agent },
            session_state,
        }
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

    pub fn continue_turn(mut self, mut session_state: SessionState) -> Transition<StreamingAgent> {
        let (events, actions) = self.agent.continue_turn(&mut session_state);
        Transition {
            events,
            actions,
            state: StreamingAgent { agent: self.agent },
            session_state,
        }
    }

    pub fn wait_for_input(self, session_state: SessionState) -> Transition<IdleAgent> {
        Transition {
            events: vec![],
            actions: vec![],
            state: IdleAgent { agent: self.agent },
            session_state,
        }
    }

    pub fn abort(mut self, session_state: SessionState) -> Transition<AbortedAgent> {
        let events = self.agent.abort();
        Transition {
            events,
            actions: vec![],
            state: AbortedAgent { agent: self.agent },
            session_state,
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
            session_state: SessionState::default(),
        }
    }

    /// Transition back to Idle without clearing conversation history.
    /// Used when the host wants to start a new turn after the previous one finished.
    pub fn into_idle(self, session_state: SessionState) -> (IdleAgent, SessionState) {
        let mut agent = self.agent;
        agent.turn_tools.clear();
        (IdleAgent { agent }, session_state)
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
            session_state: SessionState::default(),
        }
    }

    /// Transition back to Idle without clearing conversation history.
    /// Used when the host wants to start a new turn after an abort/error.
    pub fn into_idle(self, session_state: SessionState) -> (IdleAgent, SessionState) {
        let mut agent = self.agent;
        agent.turn_tools.clear();
        (IdleAgent { agent }, session_state)
    }

    pub fn into_runtime(self) -> AgentRuntime {
        AgentRuntime::Aborted(self)
    }
}
