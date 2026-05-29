use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::context::LlmContext;
use crate::events::{AgentAction, AgentEvent, QueueMode, ThinkingLevel};
use crate::llm::Model;
use crate::message::{AgentMessage, Content, ToolCall, ToolResultMessage};
use crate::session::SessionState;
use crate::tool::{ExecutionMode, ToolDefinition};
use crate::types::{SessionId, ToolCallId};
use tracing::{debug, warn};

/// Public agent state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentState {
    pub system_prompt: String,
    pub model: Model,
    pub thinking_level: ThinkingLevel,
    pub messages: Vec<AgentMessage>,
    pub is_streaming: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub streaming_message: Option<AgentMessage>,
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
    #[serde(default)]
    pub messages: Vec<AgentMessage>,
    #[serde(default)]
    pub session_state: Option<SessionState>,
}

/// Internal phase of the agent state machine.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Idle,
    Streaming,
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
    pub(crate) completed_tool_results: Vec<ToolResultMessage>,
    pub(crate) completed_tool_terminations: Vec<bool>,
    pub(crate) steering_mode: QueueMode,
    pub(crate) follow_up_mode: QueueMode,
    pub(crate) session_id: Option<SessionId>,
    pub(crate) session_state: SessionState,
    pub(crate) turn_tools: Vec<ToolDefinition>,
}

impl Agent {
    pub fn new(options: AgentOptions) -> Self {
        let mut session_state = options.session_state.unwrap_or_default();
        if session_state.entries.is_empty() && !options.messages.is_empty() {
            session_state = SessionState::from_messages(&options.messages);
        }
        Self {
            state: AgentState {
                system_prompt: options.system_prompt,
                model: options.model,
                thinking_level: options.thinking_level,
                messages: options.messages,
                is_streaming: false,
                streaming_message: None,
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
            session_state,
            turn_tools: Vec::new(),
        }
    }

    /// Abort the current run.
    pub(crate) fn abort(&mut self) -> Vec<AgentEvent> {
        warn!(phase = ?self.phase, "agent aborted");
        self.steering_queue.clear();
        self.follow_up_queue.clear();
        self.pending_tool_calls.clear();
        self.completed_tool_results.clear();
        self.completed_tool_terminations.clear();
        self.state.pending_tool_calls.clear();
        self.state.is_streaming = false;
        self.state.streaming_message = None;
        self.turn_tools.clear();
        self.phase = Phase::Idle;

        vec![
            AgentEvent::QueueUpdate {
                steer: vec![],
                follow_up: vec![],
            },
            AgentEvent::AgentEnd {
                messages: self.state.messages.clone(),
            },
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

    /// Read-only access to the session tree.
    pub fn session_state(&self) -> &SessionState {
        &self.session_state
    }

    /// Replace the in-memory session tree.
    pub(crate) fn set_session_state(&mut self, state: SessionState) {
        self.session_state = state;
    }

    /// Reset state (clear messages, queues, runtime state).
    pub(crate) fn reset(&mut self) {
        debug!("agent state reset");
        self.state.messages.clear();
        self.state.is_streaming = false;
        self.state.streaming_message = None;
        self.state.pending_tool_calls.clear();
        self.state.error_message = None;
        self.steering_queue.clear();
        self.follow_up_queue.clear();
        self.pending_tool_calls.clear();
        self.completed_tool_results.clear();
        self.session_state = SessionState::default();
        self.completed_tool_terminations.clear();
        self.turn_tools.clear();
        self.phase = Phase::Idle;
    }

    // --- private helpers used by submodules ---

    pub(crate) fn build_llm_context(&self) -> LlmContext {
        LlmContext {
            system_prompt: self.state.system_prompt.clone(),
            messages: self.state.messages.clone(),
            tools: self.turn_tools.clone(),
        }
    }

    pub(crate) fn inject_messages_and_stream(
        &mut self,
        messages: Vec<AgentMessage>,
    ) -> (Vec<AgentEvent>, Vec<AgentAction>) {
        let mut events = Vec::new();

        for msg in messages {
            events.push(AgentEvent::MessageStart {
                message: msg.clone(),
            });
            self.append_session_message(&msg);
            self.state.messages.push(msg);
            events.push(AgentEvent::MessageEnd {
                message: self.state.messages.last().unwrap().clone(),
            });
        }

        self.phase = Phase::Streaming;
        self.state.is_streaming = true;
        events.push(AgentEvent::TurnStart);

        let actions = vec![AgentAction::StreamLlm {
            context: self.build_llm_context(),
            session_id: self.session_id.clone(),
        }];

        (events, actions)
    }

    pub(crate) fn replace_last_assistant_or_push(&mut self, msg: AgentMessage) {
        if let Some(AgentMessage::Assistant(_)) = self.state.messages.last() {
            *self.state.messages.last_mut().unwrap() = msg;
        } else {
            self.state.messages.push(msg);
        }
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
pub(crate) mod session;
pub(crate) mod streaming;
pub(crate) mod tools;
pub(crate) mod turn;
