use serde::{Deserialize, Serialize};

use crate::message::{AgentMessage, ToolCall, ToolResultMessage};
use crate::tool::ToolResult as ToolExecResult;
use crate::types::{SessionId, ToolArguments, ToolCallId, ToolName};

// ---------------------------------------------------------------------------
// Tool execution lifecycle types
// ---------------------------------------------------------------------------

/// Which output stream a tool update chunk came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolOutputStream {
    Stdout,
    Stderr,
    Status,
}

/// Reason a tool was cancelled.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CancelReason {
    UserRequested,
    Timeout,
    AgentAborted,
    DependencyFailed { cause_tool_call_id: ToolCallId },
}

/// A streaming update from a running tool execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolExecutionUpdate {
    pub tool_call_id: ToolCallId,
    pub stream: ToolOutputStream,
    pub chunk: String,
    pub sequence: u64,
    pub timestamp: u64,
}

// ---------------------------------------------------------------------------
// Actions and events
// ---------------------------------------------------------------------------

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
    CancelTools {
        tool_call_ids: Vec<ToolCallId>,
        reason: CancelReason,
    },
    WaitForInput {
        mode: WaitMode,
    },
    Finished,
    Summarize {
        context: crate::context::LlmContext,
        plan: crate::session::CompactionPlan,
    },
}

/// Events emitted by the core to notify the host of state changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    AgentStart,
    AgentEnd,
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
        stream: ToolOutputStream,
        chunk: String,
        sequence: u64,
        timestamp: u64,
    },
    ToolExecutionEnd {
        tool_call_id: ToolCallId,
        result: ToolExecResult,
        is_error: bool,
    },
    ToolExecutionCancelled {
        tool_call_id: ToolCallId,
        reason: CancelReason,
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
