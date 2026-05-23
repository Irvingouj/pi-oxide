use serde::{Deserialize, Serialize};

use crate::message::{AgentMessage, ToolCall, ToolResultMessage};
use crate::tool::ToolResult as ToolExecResult;
use crate::types::{SessionId, ToolArguments, ToolCallId, ToolName};

/// Actions that the core requests the host to perform.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentAction {
    StreamLlm {
        context: crate::context::LlmContext,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<SessionId>,
    },
    ExecuteTools {
        calls: Vec<ToolCall>,
    },
    WaitForInput {
        mode: WaitMode,
    },
    Finished {
        messages: Vec<AgentMessage>,
    },
}

/// Events emitted by the core to notify the host of state changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    AgentStart,
    AgentEnd {
        messages: Vec<AgentMessage>,
    },
    TurnStart,
    TurnEnd {
        message: AgentMessage,
        tool_results: Vec<ToolResultMessage>,
    },
    MessageStart {
        message: AgentMessage,
    },
    MessageUpdate {
        message: AgentMessage,
        delta: ContentDelta,
    },
    MessageEnd {
        message: AgentMessage,
    },
    ToolExecutionStart {
        tool_call_id: ToolCallId,
        tool_name: ToolName,
        #[serde(skip_serializing_if = "Option::is_none")]
        args: Option<ToolArguments>,
    },
    ToolExecutionUpdate {
        tool_call_id: ToolCallId,
        partial_result: ToolExecResult,
    },
    ToolExecutionEnd {
        tool_call_id: ToolCallId,
        result: ToolExecResult,
        is_error: bool,
    },
    QueueUpdate {
        steer: Vec<AgentMessage>,
        follow_up: Vec<AgentMessage>,
    },
    SavePoint {
        had_pending_writes: bool,
    },
    Settled,
}

/// A delta describing how a streaming assistant message changed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContentDelta {
    TextStart,
    TextDelta {
        text: String,
    },
    TextEnd,
    ThinkingStart,
    ThinkingDelta {
        text: String,
    },
    ThinkingEnd,
    ToolCallStart {
        tool_call: ToolCall,
    },
    ToolCallDelta {
        tool_call_id: ToolCallId,
        delta: serde_json::Value,
    },
    ToolCallEnd {
        tool_call_id: ToolCallId,
    },
}

/// Controls how queued messages are drained.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum QueueMode {
    #[default]
    OneAtATime,
    All,
}

/// Reasoning / thinking level for models that support it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingLevel {
    #[default]
    Off,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
}

/// What kind of input the core is waiting for.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitMode {
    Steering,
    FollowUp,
    Any,
}
