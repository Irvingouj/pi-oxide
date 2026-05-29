//! Typed metadata for tool results used by context projection.
//!
//! This metadata lets the projection engine choose a strategy
//! based on structured data rather than guessing from raw text.

use serde::{Deserialize, Serialize};

/// What kind of content a tool result contains.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentKind {
    FileRead,
    CommandOutput,
    Diff,
    SearchResults,
    DirectoryListing,
    GenericText,
}

/// Shape of a projection — how the text is sliced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ProjectionShape {
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
    #[serde(rename = "microcompacted")]
    Microcompacted,
}

/// Strategy that decides when and how to project a tool result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ProjectionStrategy {
    #[serde(rename = "fixed")]
    Fixed {
        shape: ProjectionShape,
        #[serde(default)]
        min_age: u32,
    },
    #[serde(rename = "dynamic")]
    Dynamic { script: String },
}

impl Default for ProjectionStrategy {
    fn default() -> Self {
        ProjectionStrategy::Fixed {
            shape: ProjectionShape::KeepFull,
            min_age: 0,
        }
    }
}

/// Outcome of applying a projection strategy.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProjectionOutcome {
    pub text: String,
}

impl ProjectionOutcome {
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn into_text(self) -> String {
        self.text
    }
}

/// Typed metadata that a host can attach to tool results for projection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResultContext {
    pub content_kind: ContentKind,
    pub strategy: ProjectionStrategy,
    pub original_chars: usize,
    pub truncated_by_tool: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

const DEFAULT_MAX_CHARS: usize = 2000;
const DEFAULT_MIN_AGE: u32 = 2;

/// Fallback strategy when metadata is missing, keyed by tool name.
pub fn fallback_strategy(tool_name: &str) -> ProjectionStrategy {
    match tool_name {
        "read" => ProjectionStrategy::Fixed {
            shape: ProjectionShape::Head {
                max_chars: DEFAULT_MAX_CHARS,
            },
            min_age: DEFAULT_MIN_AGE,
        },
        "bash" => ProjectionStrategy::Fixed {
            shape: ProjectionShape::Tail {
                max_chars: DEFAULT_MAX_CHARS,
            },
            min_age: DEFAULT_MIN_AGE,
        },
        "edit" | "write" => ProjectionStrategy::Fixed {
            shape: ProjectionShape::KeepFull,
            min_age: 0,
        },
        "grep" | "find" | "ls" => ProjectionStrategy::Fixed {
            shape: ProjectionShape::Head {
                max_chars: DEFAULT_MAX_CHARS,
            },
            min_age: DEFAULT_MIN_AGE,
        },
        _ => ProjectionStrategy::Fixed {
            shape: ProjectionShape::Head {
                max_chars: DEFAULT_MAX_CHARS,
            },
            min_age: DEFAULT_MIN_AGE,
        },
    }
}
