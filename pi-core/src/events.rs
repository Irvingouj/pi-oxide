use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::message::{AgentMessage, ToolCall, ToolResultMessage};
use crate::tool::ToolResult as ToolExecResult;
use crate::types::{SessionId, ToolArguments, ToolCallId, ToolName};

// ---------------------------------------------------------------------------
// Tool execution lifecycle types
// ---------------------------------------------------------------------------

/// Which output stream a tool update chunk came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ToolOutputStream {
    Stdout,
    Stderr,
    Status,
}

/// Reason a tool was cancelled.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(tag = "type", rename_all = "snake_case")]
pub enum CancelReason {
    UserRequested,
    Timeout,
    AgentAborted,
    DependencyFailed { cause_tool_call_id: ToolCallId },
}

/// A streaming update from a running tool execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct ToolExecutionUpdate {
    pub tool_call_id: ToolCallId,
    pub stream: ToolOutputStream,
    pub chunk: String,
    pub sequence: u64,
    pub timestamp: u64,
}

/// Reference to a background job started by a tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct BackgroundJobRef {
    pub job_id: String,
    pub tool_call_id: ToolCallId,
    pub command_label: String,
}

// ---------------------------------------------------------------------------
// Actions and events
// ---------------------------------------------------------------------------

/// Actions that the core requests the host to perform.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(tag = "type", rename_all = "snake_case")]
#[ts(export)]
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
    Finished {
        messages: Vec<AgentMessage>,
    },
}

/// Events emitted by the core to notify the host of state changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(tag = "type", rename_all = "snake_case")]
#[ts(export)]
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
        #[ts(type = "object | undefined")]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(tag = "kind", rename_all = "snake_case")]
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
        #[ts(type = "object")]
        delta: serde_json::Value,
    },
    ToolCallEnd {
        tool_call_id: ToolCallId,
    },
}

/// Controls how queued messages are drained.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum QueueMode {
    #[default]
    OneAtATime,
    All,
}

/// Reasoning / thinking level for models that support it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
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
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum WaitMode {
    Steering,
    FollowUp,
    Any,
}
