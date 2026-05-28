//! Typed metadata for tool results used by context projection.
//!
//! This metadata lets the projection engine choose a strategy
//! based on structured data rather than guessing from raw text.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// What kind of content a tool result contains.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ContentKind {
    FileRead,
    CommandOutput,
    Diff,
    SearchResults,
    DirectoryListing,
    GenericText,
}

/// Projection strategy for a single tool result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ContextStrategy {
    #[serde(rename = "keep_full")]
    KeepFull,
    #[serde(rename = "head")]
    Head { max_chars: usize },
    #[serde(rename = "tail")]
    Tail { max_chars: usize },
    #[serde(rename = "head_tail")]
    HeadTail {
        head_chars: usize,
        tail_chars: usize,
    },
    #[serde(rename = "drop_if_old")]
    DropIfOld,
    /// Applied by microcompact: old tool result replaced with a one-line summary.
    #[serde(rename = "microcompacted")]
    Microcompacted,
    /// Run a Rhai script to transform the tool result.
    /// The script receives the full text plus global context variables.
    #[serde(rename = "script")]
    Script { script: String },
}

/// Typed metadata that a host can attach to tool results for projection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct ToolResultContext {
    pub content_kind: ContentKind,
    pub strategy: ContextStrategy,
    pub original_chars: usize,
    pub truncated_by_tool: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub smart_extract_prompt: Option<String>,
}

/// Fallback strategy when metadata is missing, keyed by tool name.
pub fn fallback_strategy(tool_name: &str) -> ContextStrategy {
    match tool_name {
        "read" => ContextStrategy::Head { max_chars: 2000 },
        "bash" => ContextStrategy::Tail { max_chars: 2000 },
        "edit" | "write" => ContextStrategy::KeepFull,
        "grep" | "find" | "ls" => ContextStrategy::Head { max_chars: 2000 },
        _ => ContextStrategy::Head { max_chars: 2000 },
    }
}
