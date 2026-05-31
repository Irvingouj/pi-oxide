use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::context::LlmContext;
use crate::context_projection::{
    build_llm_context_from_trimmed, ChangeMarker, ContextProjectionBudget,
};
use crate::events::{AgentAction, AgentEvent, QueueMode, ThinkingLevel};
use crate::llm::Model;
use crate::message::{
    AgentMessage, Artifacts, AssistantMessage, Content, ToolCall, TrimmedMessage,
};
use crate::session::CompactionPlan;
use crate::tool::{ExecutionMode, ToolDefinition};
use crate::types::{SessionId, ToolCallId};
use tracing::{debug, warn};

/// Public agent state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentState {
    pub system_prompt: String,
    pub model: Model,
    pub thinking_level: ThinkingLevel,
    pub pending_tool_calls: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// Options for constructing an Agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentOptions {
    pub system_prompt: String,
    pub model: Model,
    #[serde(default)]
    pub thinking_level: ThinkingLevel,
    #[serde(default)]
    pub steering_mode: QueueMode,
    #[serde(default)]
    pub follow_up_mode: QueueMode,
    #[serde(default)]
    pub tool_execution_mode: ExecutionMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
}

/// Internal phase of the agent state machine.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Idle,
    Streaming,
    Compacting,
    WaitForInput,
}

/// The agent state machine. Purely synchronous; the host drives progress.
#[derive(Clone)]
pub struct Agent {
    pub(crate) state: AgentState,
    pub(crate) steering_queue: Vec<AgentMessage>,
    pub(crate) follow_up_queue: Vec<AgentMessage>,
    pub(crate) phase: Phase,
    pub(crate) pending_tool_calls: HashMap<ToolCallId, ToolCall>,
    pub(crate) completed_tool_results: Vec<crate::message::ToolResultMessage>,
    pub(crate) completed_tool_terminations: Vec<bool>,
    pub(crate) steering_mode: QueueMode,
    pub(crate) follow_up_mode: QueueMode,
    pub(crate) session_id: Option<SessionId>,
    pub(crate) turn_tools: Vec<ToolDefinition>,
    /// Tracks the partial AssistantMessage being built during streaming.
    /// Cleared when on_llm_done finalizes the message into T.
    pub(crate) streaming_assistant: Option<AssistantMessage>,
    /// Counter for generating entry IDs.
    pub(crate) entry_counter: u32,
}

impl Agent {
    pub fn new(options: AgentOptions) -> Self {
        Self {
            state: AgentState {
                system_prompt: options.system_prompt,
                model: options.model,
                thinking_level: options.thinking_level,
                pending_tool_calls: Vec::new(),
                error_message: None,
            },
            steering_queue: Vec::new(),
            follow_up_queue: Vec::new(),
            phase: Phase::Idle,
            pending_tool_calls: HashMap::new(),
            completed_tool_results: Vec::new(),
            completed_tool_terminations: Vec::new(),
            steering_mode: options.steering_mode,
            follow_up_mode: options.follow_up_mode,
            session_id: options.session_id,
            turn_tools: Vec::new(),
            streaming_assistant: None,
            entry_counter: 0,
        }
    }

    /// Abort the current run. Returns events and clears ephemeral state.
    /// T/A are NOT touched — they belong to the host.
    pub(crate) fn abort(&mut self) -> Vec<AgentEvent> {
        warn!(phase = ?self.phase, "agent aborted");
        self.steering_queue.clear();
        self.follow_up_queue.clear();
        self.pending_tool_calls.clear();
        self.completed_tool_results.clear();
        self.completed_tool_terminations.clear();
        self.state.pending_tool_calls.clear();
        self.streaming_assistant = None;
        self.turn_tools.clear();
        self.phase = Phase::Idle;

        vec![
            AgentEvent::QueueUpdate {
                steer: vec![],
                follow_up: vec![],
            },
            AgentEvent::AgentEnd,
        ]
    }

    /// Read-only access to current state.
    pub fn state(&self) -> &AgentState {
        &self.state
    }

    /// Mutable access to current state.
    pub fn state_mut(&mut self) -> &mut AgentState {
        &mut self.state
    }

    /// Reset state (clear queues, runtime state).
    pub(crate) fn reset(&mut self) {
        debug!("agent state reset");
        self.streaming_assistant = None;
        self.state.pending_tool_calls.clear();
        self.state.error_message = None;
        self.steering_queue.clear();
        self.follow_up_queue.clear();
        self.pending_tool_calls.clear();
        self.completed_tool_results.clear();
        self.completed_tool_terminations.clear();
        self.turn_tools.clear();
        self.phase = Phase::Idle;
    }

    // --- context building ---

    /// Build LLM context from transcript. No projection decisions — just convert TrimmedMessages.
    pub(crate) fn build_llm_context(
        &self,
        t: &[TrimmedMessage],
    ) -> (LlmContext, Vec<ChangeMarker>) {
        let context =
            build_llm_context_from_trimmed(t, &self.state.system_prompt, &self.turn_tools);
        // Markers are empty here — projection_scan happens at turn end.
        (context, vec![])
    }

    /// Plan compaction against T.
    pub(crate) fn build_summary_action(
        &self,
        t: &[TrimmedMessage],
        budget: &ContextProjectionBudget,
        compaction_prompt: &str,
    ) -> Option<AgentAction> {
        let plan = crate::session::plan_compaction(t, budget)?;
        let summary_messages = crate::session::build_summary_messages(&plan);
        Some(AgentAction::Summarize {
            context: LlmContext {
                system_prompt: compaction_prompt.to_string(),
                messages: summary_messages,
                tools: vec![],
            },
            plan,
        })
    }

    /// Accept a compaction summary: rewrite T, archive OriginalTool originals to A.
    pub(crate) fn accept_summary(
        &mut self,
        summary_text: String,
        t: Vec<TrimmedMessage>,
        a: &mut Artifacts,
        plan: &CompactionPlan,
    ) -> (
        Vec<AgentEvent>,
        Vec<AgentAction>,
        Vec<ChangeMarker>,
        Vec<TrimmedMessage>,
    ) {
        let t = crate::session::apply_compaction(t, plan.clone(), summary_text, a);
        self.phase = Phase::Streaming;

        let (context, markers) = self.build_llm_context(&t);
        let events = vec![AgentEvent::AgentStart, AgentEvent::TurnStart];
        let actions = vec![AgentAction::StreamLlm {
            context,
            session_id: self.session_id.clone(),
        }];
        (events, actions, markers, t)
    }

    /// Generate a unique entry ID.
    pub(crate) fn next_entry_id(&mut self) -> String {
        let id = format!("entry-{}", self.entry_counter);
        self.entry_counter += 1;
        id
    }

    /// Initialize entry_counter from restored T and A to avoid collisions.
    pub fn initialize_entry_counter(&mut self, t: &[TrimmedMessage], a: &Artifacts) {
        let max_t = t
            .iter()
            .filter_map(|m| match m {
                TrimmedMessage::ProjectedTool(p) => p
                    .entry_id
                    .strip_prefix("entry-")
                    .and_then(|s| s.parse::<u32>().ok()),
                TrimmedMessage::OriginalTool(o) => o
                    .entry_id
                    .strip_prefix("entry-")
                    .and_then(|s| s.parse::<u32>().ok()),
                _ => None,
            })
            .max();
        let max_a = a
            .keys()
            .filter_map(|k| k.strip_prefix("entry-").and_then(|s| s.parse::<u32>().ok()))
            .max();
        self.entry_counter = max_t
            .into_iter()
            .chain(max_a.into_iter())
            .max()
            .unwrap_or(0)
            + 1;
    }
}

pub(crate) fn current_timestamp() -> u64 {
    crate::timestamp::current_timestamp()
}

/// Helper trait for extracting assistant text.
pub trait AssistantTextExt {
    fn assistant_text(&self) -> String;
}

impl AssistantTextExt for AgentMessage {
    fn assistant_text(&self) -> String {
        match self {
            AgentMessage::Assistant(a) => a
                .content
                .iter()
                .filter_map(|c| match c {
                    Content::Text(t) => Some(t.text.clone()),
                    _ => None,
                })
                .collect(),
            _ => String::new(),
        }
    }
}

pub(crate) mod queues;
pub(crate) mod streaming;
pub(crate) mod tools;
pub(crate) mod turn;
