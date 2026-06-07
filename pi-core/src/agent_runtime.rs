//! Typestate wrapper around [`Agent`].
//!
//! Each phase struct exposes only valid operations for that phase,
//! making illegal state transitions a compile-time error in Rust hosts.
//!
//! transcript (Vec<TrimmedMessage>) and artifacts (Artifacts) are owned by the host
//! and flow through typestate method parameters and back via Transition.

use crate::agent::{Agent, Phase};
use crate::context_projection::{ChangeMarker, ContextProjectionBudget};
use crate::events::{AgentAction, AgentEvent, CancelReason, ToolExecutionUpdate};
use crate::llm::{LlmChunk, LlmResult};
use crate::message::{AgentMessage, Artifacts, TrimmedMessage};
use crate::session::CompactionPlan;
use crate::tool::{ToolDefinition, ToolError, ToolResult};
use crate::types::ToolCallId;

// ---------------------------------------------------------------------------
// Type aliases
// ---------------------------------------------------------------------------

pub type CoreTransitionParts<T> = (
    Vec<AgentEvent>,
    Vec<AgentAction>,
    T,
    Vec<TrimmedMessage>,
    Artifacts,
    u32,
    Vec<ChangeMarker>,
);

pub type TransitionParts = CoreTransitionParts<AgentRuntime>;

// ---------------------------------------------------------------------------
// Transition types
// ---------------------------------------------------------------------------

/// A state change carrying events, actions, markers, transcript, artifacts, and turn_number.
pub struct Transition<T> {
    pub events: Vec<AgentEvent>,
    pub actions: Vec<AgentAction>,
    pub state: T,
    pub transcript: Vec<TrimmedMessage>,
    pub artifacts: Artifacts,
    pub turn_number: u32,
    pub markers: Vec<ChangeMarker>,
}

impl<T> Transition<T> {
    /// Destructure into the 7-tuple of components.
    pub fn into_parts(self) -> CoreTransitionParts<T> {
        let Transition {
            events,
            actions,
            state,
            transcript,
            artifacts,
            turn_number,
            markers,
        } = self;
        (
            events,
            actions,
            state,
            transcript,
            artifacts,
            turn_number,
            markers,
        )
    }
}

/// Outcome of finishing an LLM stream.
pub enum FinishLlmTransition {
    MoreStreaming(Transition<StreamingAgent>),
    PreToolCall(Transition<PreToolCallAgent>),
    Ready(Transition<ReadyAgent>),
    Finished(Transition<FinishedAgent>),
    Aborted(Transition<AbortedAgent>),
}

impl FinishLlmTransition {
    /// Destructure into the common parts regardless of which phase we land in.
    pub fn into_parts(self) -> TransitionParts {
        match self {
            FinishLlmTransition::MoreStreaming(t) => {
                let (events, actions, state, transcript, artifacts, turn_number, markers) =
                    t.into_parts();
                (
                    events,
                    actions,
                    state.into_runtime(),
                    transcript,
                    artifacts,
                    turn_number,
                    markers,
                )
            }
            FinishLlmTransition::PreToolCall(t) => {
                let (events, actions, state, transcript, artifacts, turn_number, markers) =
                    t.into_parts();
                (
                    events,
                    actions,
                    state.into_runtime(),
                    transcript,
                    artifacts,
                    turn_number,
                    markers,
                )
            }
            FinishLlmTransition::Ready(t) => {
                let (events, actions, state, transcript, artifacts, turn_number, markers) =
                    t.into_parts();
                (
                    events,
                    actions,
                    state.into_runtime(),
                    transcript,
                    artifacts,
                    turn_number,
                    markers,
                )
            }
            FinishLlmTransition::Finished(t) => {
                let (events, actions, state, transcript, artifacts, turn_number, markers) =
                    t.into_parts();
                (
                    events,
                    actions,
                    state.into_runtime(),
                    transcript,
                    artifacts,
                    turn_number,
                    markers,
                )
            }
            FinishLlmTransition::Aborted(t) => {
                let (events, actions, state, transcript, artifacts, turn_number, markers) =
                    t.into_parts();
                (
                    events,
                    actions,
                    state.into_runtime(),
                    transcript,
                    artifacts,
                    turn_number,
                    markers,
                )
            }
        }
    }
}

/// Outcome of resolving a tool call.
pub enum ToolTransition {
    PreToolCall(Transition<PreToolCallAgent>),
    ExecutingTools(Transition<ExecutingToolsAgent>),
    Ready(Transition<ReadyAgent>),
    Finished(Transition<FinishedAgent>),
}

impl ToolTransition {
    pub fn into_parts(self) -> TransitionParts {
        match self {
            ToolTransition::PreToolCall(t) => {
                let (events, actions, state, transcript, artifacts, turn_number, markers) =
                    t.into_parts();
                (
                    events,
                    actions,
                    state.into_runtime(),
                    transcript,
                    artifacts,
                    turn_number,
                    markers,
                )
            }
            ToolTransition::ExecutingTools(t) => {
                let (events, actions, state, transcript, artifacts, turn_number, markers) =
                    t.into_parts();
                (
                    events,
                    actions,
                    state.into_runtime(),
                    transcript,
                    artifacts,
                    turn_number,
                    markers,
                )
            }
            ToolTransition::Ready(t) => {
                let (events, actions, state, transcript, artifacts, turn_number, markers) =
                    t.into_parts();
                (
                    events,
                    actions,
                    state.into_runtime(),
                    transcript,
                    artifacts,
                    turn_number,
                    markers,
                )
            }
            ToolTransition::Finished(t) => {
                let (events, actions, state, transcript, artifacts, turn_number, markers) =
                    t.into_parts();
                (
                    events,
                    actions,
                    state.into_runtime(),
                    transcript,
                    artifacts,
                    turn_number,
                    markers,
                )
            }
        }
    }
}

/// Outcome of starting a new turn.
pub enum StartTurnTransition {
    Streaming(Transition<StreamingAgent>),
    Compacting(Transition<CompactingAgent>),
}

impl StartTurnTransition {
    pub fn into_parts(self) -> TransitionParts {
        match self {
            StartTurnTransition::Streaming(t) => {
                let (events, actions, state, transcript, artifacts, turn_number, markers) =
                    t.into_parts();
                (
                    events,
                    actions,
                    state.into_runtime(),
                    transcript,
                    artifacts,
                    turn_number,
                    markers,
                )
            }
            StartTurnTransition::Compacting(t) => {
                let (events, actions, state, transcript, artifacts, turn_number, markers) =
                    t.into_parts();
                (
                    events,
                    actions,
                    state.into_runtime(),
                    transcript,
                    artifacts,
                    turn_number,
                    markers,
                )
            }
        }
    }
}

/// Outcome of continuing a turn.
pub enum ContinueTurnTransition {
    Streaming(Transition<StreamingAgent>),
    Compacting(Transition<CompactingAgent>),
}

impl ContinueTurnTransition {
    pub fn into_parts(self) -> TransitionParts {
        match self {
            ContinueTurnTransition::Streaming(t) => {
                let (events, actions, state, transcript, artifacts, turn_number, markers) =
                    t.into_parts();
                (
                    events,
                    actions,
                    state.into_runtime(),
                    transcript,
                    artifacts,
                    turn_number,
                    markers,
                )
            }
            ContinueTurnTransition::Compacting(t) => {
                let (events, actions, state, transcript, artifacts, turn_number, markers) =
                    t.into_parts();
                (
                    events,
                    actions,
                    state.into_runtime(),
                    transcript,
                    artifacts,
                    turn_number,
                    markers,
                )
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
    Compacting(CompactingAgent),
    PreToolCall(PreToolCallAgent),
    ExecutingTools(ExecutingToolsAgent),
    ReadyToContinue(ReadyAgent),
    Finished(FinishedAgent),
    Aborted(AbortedAgent),
}

impl AgentRuntime {
    /// Wrap an existing agent. The agent must be in a valid phase.
    pub fn from_agent(agent: Agent) -> Self {
        match agent.phase {
            Phase::Idle => AgentRuntime::Idle(IdleAgent { agent }),
            Phase::Streaming => AgentRuntime::Streaming(StreamingAgent { agent }),
            Phase::Compacting => {
                unreachable!("Compacting phase is only entered via typestate transitions")
            }
            Phase::PreToolCall => AgentRuntime::PreToolCall(PreToolCallAgent { agent }),
            Phase::ExecutingTools => AgentRuntime::ExecutingTools(ExecutingToolsAgent { agent }),
            Phase::WaitForInput => AgentRuntime::ReadyToContinue(ReadyAgent { agent }),
        }
    }

    /// Wrap an agent that is in the compacting phase.
    pub fn compacting(agent: Agent, plan: CompactionPlan) -> Self {
        AgentRuntime::Compacting(CompactingAgent { agent, plan })
    }

    /// Construct from options (starts in Idle).
    pub fn new(options: crate::agent::AgentOptions) -> Self {
        Self::from_agent(Agent::new(options))
    }

    /// Initialize entry_counter from restored transcript/artifacts to avoid collisions.
    pub fn initialize_entry_counter(&mut self, t: &[TrimmedMessage], a: &Artifacts) {
        self.as_agent_mut().initialize_entry_counter(t, a);
    }

    fn as_agent(&self) -> &Agent {
        match self {
            AgentRuntime::Idle(a) => &a.agent,
            AgentRuntime::Streaming(a) => &a.agent,
            AgentRuntime::Compacting(a) => &a.agent,
            AgentRuntime::PreToolCall(a) => &a.agent,
            AgentRuntime::ExecutingTools(a) => &a.agent,
            AgentRuntime::ReadyToContinue(a) => &a.agent,
            AgentRuntime::Finished(a) => &a.agent,
            AgentRuntime::Aborted(a) => &a.agent,
        }
    }

    fn as_agent_mut(&mut self) -> &mut Agent {
        match self {
            AgentRuntime::Idle(a) => &mut a.agent,
            AgentRuntime::Streaming(a) => &mut a.agent,
            AgentRuntime::Compacting(a) => &mut a.agent,
            AgentRuntime::PreToolCall(a) => &mut a.agent,
            AgentRuntime::ExecutingTools(a) => &mut a.agent,
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
            AgentRuntime::Compacting(a) => a.agent,
            AgentRuntime::PreToolCall(a) => a.agent,
            AgentRuntime::ExecutingTools(a) => a.agent,
            AgentRuntime::ReadyToContinue(a) => a.agent,
            AgentRuntime::Finished(a) => a.agent,
            AgentRuntime::Aborted(a) => a.agent,
        }
    }

    /// Reset the agent back to an idle state regardless of current phase.
    /// transcript/artifacts/turn_number get defaults (empty).
    pub fn reset(self) -> Self {
        let mut agent = self.into_agent();
        agent.reset();
        Self::from_agent(agent)
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

    #[allow(clippy::too_many_arguments)]
    pub fn start_turn(
        mut self,
        msg: AgentMessage,
        tools: Vec<ToolDefinition>,
        transcript: Vec<TrimmedMessage>,
        artifacts: Artifacts,
        turn_number: u32,
        budget: &ContextProjectionBudget,
        compaction_prompt: &str,
    ) -> StartTurnTransition {
        let (events, actions, markers, transcript, turn_number) = self.agent.start_turn(
            msg,
            tools,
            transcript,
            turn_number,
            budget,
            compaction_prompt,
        );

        let maybe_plan = actions.iter().find_map(|a| {
            if let AgentAction::Summarize { plan, .. } = a {
                Some(plan.clone())
            } else {
                None
            }
        });

        if let Some(plan) = maybe_plan {
            return StartTurnTransition::Compacting(Transition {
                events,
                actions,
                state: CompactingAgent {
                    agent: self.agent,
                    plan,
                },
                transcript,
                artifacts,
                turn_number,
                markers,
            });
        }

        StartTurnTransition::Streaming(Transition {
            events,
            actions,
            state: StreamingAgent { agent: self.agent },
            transcript,
            artifacts,
            turn_number,
            markers,
        })
    }

    pub fn steer(&mut self, message: AgentMessage) -> Vec<AgentEvent> {
        self.agent.steer(message)
    }

    pub fn follow_up(&mut self, message: AgentMessage) {
        self.agent.follow_up(message)
    }

    /// Reset returns transcript/artifacts as empty defaults.
    pub fn reset(mut self) -> Transition<IdleAgent> {
        self.agent.reset();
        Transition {
            events: vec![],
            actions: vec![],
            state: IdleAgent { agent: self.agent },
            transcript: vec![],
            artifacts: Artifacts::new(),
            turn_number: 0,
            markers: vec![],
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

    pub fn finish_llm(
        mut self,
        result: LlmResult,
        transcript: Vec<TrimmedMessage>,
        artifacts: Artifacts,
        turn_number: u32,
        budget: &ContextProjectionBudget,
    ) -> FinishLlmTransition {
        let (events, actions, markers, transcript, artifacts) =
            self.agent
                .on_llm_done(result, transcript, artifacts, turn_number, budget);

        // Abort / error path
        if self.agent.state().error_message.is_some() {
            return FinishLlmTransition::Aborted(Transition {
                events,
                actions,
                state: AbortedAgent { agent: self.agent },
                transcript,
                artifacts,
                turn_number,
                markers,
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
                transcript,
                artifacts,
                turn_number,
                markers,
            });
        }

        // Tool execution requested
        if actions
            .iter()
            .any(|a| matches!(a, AgentAction::PrepareToolCalls { .. }))
        {
            return FinishLlmTransition::PreToolCall(Transition {
                events,
                actions,
                state: PreToolCallAgent { agent: self.agent },
                transcript,
                artifacts,
                turn_number,
                markers,
            });
        }

        // Turn finished without tools
        let is_finished = actions.iter().any(|a| matches!(a, AgentAction::Finished));
        if is_finished {
            return FinishLlmTransition::Finished(Transition {
                events,
                actions,
                state: FinishedAgent { agent: self.agent },
                transcript,
                artifacts,
                turn_number,
                markers,
            });
        }

        // Default: ready to continue
        FinishLlmTransition::Ready(Transition {
            events,
            actions,
            state: ReadyAgent { agent: self.agent },
            transcript,
            artifacts,
            turn_number,
            markers,
        })
    }

    pub fn abort(
        mut self,
        transcript: Vec<TrimmedMessage>,
        artifacts: Artifacts,
        turn_number: u32,
    ) -> Transition<AbortedAgent> {
        let events = self.agent.abort();
        Transition {
            events,
            actions: vec![],
            state: AbortedAgent { agent: self.agent },
            transcript,
            artifacts,
            turn_number,
            markers: vec![],
        }
    }

    pub fn into_runtime(self) -> AgentRuntime {
        AgentRuntime::Streaming(self)
    }
}

// ---------------------------------------------------------------------------
// PreToolCallAgent — tools proposed by LLM, awaiting host preparation
// ---------------------------------------------------------------------------

pub struct PreToolCallAgent {
    pub(crate) agent: Agent,
}

impl PreToolCallAgent {
    pub fn into_agent(self) -> Agent {
        self.agent
    }

    pub fn cancel_tool(
        mut self,
        id: ToolCallId,
        reason: CancelReason,
        transcript: Vec<TrimmedMessage>,
        artifacts: Artifacts,
        turn_number: u32,
    ) -> ToolTransition {
        let (events, actions, markers, transcript, artifacts) =
            self.agent
                .on_tool_cancelled(id, reason, transcript, artifacts, turn_number);

        if actions.iter().any(|a| matches!(a, AgentAction::Finished)) {
            return ToolTransition::Finished(Transition {
                events,
                actions,
                state: FinishedAgent { agent: self.agent },
                transcript,
                artifacts,
                turn_number,
                markers,
            });
        }

        if self.agent.state().pending_tool_calls.is_empty() {
            return ToolTransition::Ready(Transition {
                events,
                actions,
                state: ReadyAgent { agent: self.agent },
                transcript,
                artifacts,
                turn_number: turn_number + 1,
                markers,
            });
        }

        ToolTransition::PreToolCall(Transition {
            events,
            actions,
            state: PreToolCallAgent { agent: self.agent },
            transcript,
            artifacts,
            turn_number,
            markers,
        })
    }

    pub fn submit_user_message(&mut self, msg: AgentMessage) -> UserInputDuringTools {
        self.agent.follow_up(msg);
        UserInputDuringTools::QueuedFollowUp(vec![])
    }

    pub fn abort(
        mut self,
        transcript: Vec<TrimmedMessage>,
        artifacts: Artifacts,
        turn_number: u32,
    ) -> Transition<AbortedAgent> {
        let events = self.agent.abort();
        Transition {
            events,
            actions: vec![],
            state: AbortedAgent { agent: self.agent },
            transcript,
            artifacts,
            turn_number,
            markers: vec![],
        }
    }

    pub fn prepare_tool_calls(
        mut self,
        preparations: Vec<crate::tool::ToolCallPreparation>,
        transcript: Vec<TrimmedMessage>,
        artifacts: Artifacts,
        turn_number: u32,
    ) -> ToolTransition {
        let (events, actions, markers, transcript, artifacts) =
            self.agent
                .prepare_tool_calls(preparations, transcript, artifacts, turn_number);

        if self.agent.state().pending_tool_calls.is_empty() {
            return ToolTransition::Ready(Transition {
                events,
                actions,
                state: ReadyAgent { agent: self.agent },
                transcript,
                artifacts,
                turn_number: turn_number + 1,
                markers,
            });
        }

        ToolTransition::ExecutingTools(Transition {
            events,
            actions,
            state: ExecutingToolsAgent { agent: self.agent },
            transcript,
            artifacts,
            turn_number,
            markers,
        })
    }

    pub fn into_runtime(self) -> AgentRuntime {
        AgentRuntime::PreToolCall(self)
    }
}

// ---------------------------------------------------------------------------
// ExecutingToolsAgent — tools approved, awaiting execution results
// ---------------------------------------------------------------------------

pub struct ExecutingToolsAgent {
    pub(crate) agent: Agent,
}

impl ExecutingToolsAgent {
    pub fn into_agent(self) -> Agent {
        self.agent
    }

    pub fn on_tool_done(
        mut self,
        id: ToolCallId,
        result: Result<ToolResult, ToolError>,
        transcript: Vec<TrimmedMessage>,
        artifacts: Artifacts,
        turn_number: u32,
    ) -> ToolTransition {
        let (events, actions, markers, transcript, artifacts) =
            self.agent
                .on_tool_done(id, result, transcript, artifacts, turn_number);

        if actions.iter().any(|a| matches!(a, AgentAction::Finished)) {
            return ToolTransition::Finished(Transition {
                events,
                actions,
                state: FinishedAgent { agent: self.agent },
                transcript,
                artifacts,
                turn_number,
                markers,
            });
        }

        if self.agent.state().pending_tool_calls.is_empty() {
            return ToolTransition::Ready(Transition {
                events,
                actions,
                state: ReadyAgent { agent: self.agent },
                transcript,
                artifacts,
                turn_number: turn_number + 1,
                markers,
            });
        }

        ToolTransition::ExecutingTools(Transition {
            events,
            actions,
            state: ExecutingToolsAgent { agent: self.agent },
            transcript,
            artifacts,
            turn_number,
            markers,
        })
    }

    pub fn on_tool_update(&mut self, update: ToolExecutionUpdate) -> Vec<AgentEvent> {
        self.agent.on_tool_update(update)
    }

    pub fn on_tool_started(&mut self, id: ToolCallId) -> Vec<AgentEvent> {
        self.agent.on_tool_started(id)
    }

    pub fn cancel_tool(
        mut self,
        id: ToolCallId,
        reason: CancelReason,
        transcript: Vec<TrimmedMessage>,
        artifacts: Artifacts,
        turn_number: u32,
    ) -> ToolTransition {
        let (events, actions, markers, transcript, artifacts) =
            self.agent
                .on_tool_cancelled(id, reason, transcript, artifacts, turn_number);

        if actions.iter().any(|a| matches!(a, AgentAction::Finished)) {
            return ToolTransition::Finished(Transition {
                events,
                actions,
                state: FinishedAgent { agent: self.agent },
                transcript,
                artifacts,
                turn_number,
                markers,
            });
        }

        if self.agent.state().pending_tool_calls.is_empty() {
            return ToolTransition::Ready(Transition {
                events,
                actions,
                state: ReadyAgent { agent: self.agent },
                transcript,
                artifacts,
                turn_number: turn_number + 1,
                markers,
            });
        }

        ToolTransition::ExecutingTools(Transition {
            events,
            actions,
            state: ExecutingToolsAgent { agent: self.agent },
            transcript,
            artifacts,
            turn_number,
            markers,
        })
    }

    pub fn submit_user_message(&mut self, msg: AgentMessage) -> UserInputDuringTools {
        self.agent.follow_up(msg);
        UserInputDuringTools::QueuedFollowUp(vec![])
    }

    pub fn abort(
        mut self,
        transcript: Vec<TrimmedMessage>,
        artifacts: Artifacts,
        turn_number: u32,
    ) -> Transition<AbortedAgent> {
        let events = self.agent.abort();
        Transition {
            events,
            actions: vec![],
            state: AbortedAgent { agent: self.agent },
            transcript,
            artifacts,
            turn_number,
            markers: vec![],
        }
    }

    pub fn into_runtime(self) -> AgentRuntime {
        AgentRuntime::ExecutingTools(self)
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

    pub fn continue_turn(
        mut self,
        transcript: Vec<TrimmedMessage>,
        artifacts: Artifacts,
        turn_number: u32,
        budget: &ContextProjectionBudget,
        compaction_prompt: &str,
    ) -> ContinueTurnTransition {
        let (events, actions, markers, transcript) =
            self.agent
                .continue_turn(transcript, turn_number, budget, compaction_prompt);

        let maybe_plan = actions.iter().find_map(|a| {
            if let AgentAction::Summarize { plan, .. } = a {
                Some(plan.clone())
            } else {
                None
            }
        });

        if let Some(plan) = maybe_plan {
            return ContinueTurnTransition::Compacting(Transition {
                events,
                actions,
                state: CompactingAgent {
                    agent: self.agent,
                    plan,
                },
                transcript,
                artifacts,
                turn_number,
                markers,
            });
        }

        ContinueTurnTransition::Streaming(Transition {
            events,
            actions,
            state: StreamingAgent { agent: self.agent },
            transcript,
            artifacts,
            turn_number,
            markers,
        })
    }

    pub fn wait_for_input(
        self,
        transcript: Vec<TrimmedMessage>,
        artifacts: Artifacts,
        turn_number: u32,
    ) -> Transition<IdleAgent> {
        Transition {
            events: vec![],
            actions: vec![],
            state: IdleAgent { agent: self.agent },
            transcript,
            artifacts,
            turn_number,
            markers: vec![],
        }
    }

    pub fn abort(
        mut self,
        transcript: Vec<TrimmedMessage>,
        artifacts: Artifacts,
        turn_number: u32,
    ) -> Transition<AbortedAgent> {
        let events = self.agent.abort();
        Transition {
            events,
            actions: vec![],
            state: AbortedAgent { agent: self.agent },
            transcript,
            artifacts,
            turn_number,
            markers: vec![],
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

    /// Reset returns transcript/artifacts as empty defaults.
    pub fn restart(mut self) -> Transition<IdleAgent> {
        self.agent.reset();
        Transition {
            events: vec![],
            actions: vec![],
            state: IdleAgent { agent: self.agent },
            transcript: vec![],
            artifacts: Artifacts::new(),
            turn_number: 0,
            markers: vec![],
        }
    }

    /// Transition back to Idle without clearing conversation history.
    pub fn into_idle(
        self,
        transcript: Vec<TrimmedMessage>,
        artifacts: Artifacts,
        turn_number: u32,
    ) -> (IdleAgent, Vec<TrimmedMessage>, Artifacts, u32) {
        let mut agent = self.agent;
        agent.turn_tools.clear();
        (IdleAgent { agent }, transcript, artifacts, turn_number)
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

    /// Reset returns transcript/artifacts as empty defaults.
    pub fn restart(mut self) -> Transition<IdleAgent> {
        self.agent.reset();
        Transition {
            events: vec![],
            actions: vec![],
            state: IdleAgent { agent: self.agent },
            transcript: vec![],
            artifacts: Artifacts::new(),
            turn_number: 0,
            markers: vec![],
        }
    }

    /// Transition back to Idle without clearing conversation history.
    pub fn into_idle(
        self,
        transcript: Vec<TrimmedMessage>,
        artifacts: Artifacts,
        turn_number: u32,
    ) -> (IdleAgent, Vec<TrimmedMessage>, Artifacts, u32) {
        let mut agent = self.agent;
        agent.turn_tools.clear();
        (IdleAgent { agent }, transcript, artifacts, turn_number)
    }

    pub fn into_runtime(self) -> AgentRuntime {
        AgentRuntime::Aborted(self)
    }
}

// ---------------------------------------------------------------------------
// CompactingAgent — waiting for host to run summarization LLM
// ---------------------------------------------------------------------------

pub struct CompactingAgent {
    pub(crate) agent: Agent,
    pub(crate) plan: CompactionPlan,
}

impl CompactingAgent {
    pub fn into_agent(self) -> Agent {
        self.agent
    }

    pub fn accept_summary(
        mut self,
        summary_text: String,
        transcript: Vec<TrimmedMessage>,
        mut artifacts: Artifacts,
        turn_number: u32,
        _budget: &ContextProjectionBudget,
    ) -> Transition<StreamingAgent> {
        let (events, actions, mut markers, transcript) =
            self.agent
                .accept_summary(summary_text, transcript, &mut artifacts, &self.plan);
        markers.push(ChangeMarker::CompactionApplied);
        Transition {
            events,
            actions,
            state: StreamingAgent { agent: self.agent },
            transcript,
            artifacts,
            turn_number,
            markers,
        }
    }

    pub fn abort(
        mut self,
        transcript: Vec<TrimmedMessage>,
        artifacts: Artifacts,
        turn_number: u32,
    ) -> Transition<AbortedAgent> {
        let events = self.agent.abort();
        Transition {
            events,
            actions: vec![],
            state: AbortedAgent { agent: self.agent },
            transcript,
            artifacts,
            turn_number,
            markers: vec![],
        }
    }

    pub fn into_runtime(self) -> AgentRuntime {
        AgentRuntime::Compacting(self)
    }
}
