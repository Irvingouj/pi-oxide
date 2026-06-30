//! Context projection engine.
//!
//! Provides token estimation, projection scanning, and LLM context building
//! for the new T/A data model.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Declarative hint about what changed in the session state so the host
/// knows how to persist efficiently.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChangeMarker {
    /// Compaction mutated the entry tree; prefer full persist.
    CompactionApplied,
    /// Original tool results archived to A during projection scan.
    NewArtifacts { entry_ids: Vec<String> },
}

/// Budget parameters for context projection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextProjectionBudget {
    pub max_tool_result_chars: usize,
    pub max_context_tokens: usize,
    /// Turns older than this have their tool results microcompacted.
    #[serde(default = "default_microcompact_after_turns")]
    pub microcompact_after_turns: u32,
    /// Fraction of max_context_tokens that triggers compaction signal (0.0-1.0).
    #[serde(default = "default_compaction_threshold")]
    pub compaction_threshold: f32,
}

pub fn default_microcompact_after_turns() -> u32 {
    5
}
pub fn default_compaction_threshold() -> f32 {
    0.75
}

impl Default for ContextProjectionBudget {
    fn default() -> Self {
        Self {
            max_tool_result_chars: 50_000,
            max_context_tokens: 100_000,
            microcompact_after_turns: default_microcompact_after_turns(),
            compaction_threshold: default_compaction_threshold(),
        }
    }
}

// ---------------------------------------------------------------------------
// Token estimation
// ---------------------------------------------------------------------------

const CHARS_PER_TOKEN: usize = 4;

/// Estimate tokens for a list of messages using chars/4 heuristic.
pub fn estimate_tokens(messages: &[crate::message::AgentMessage]) -> usize {
    let chars = count_message_chars(messages);
    chars.div_ceil(CHARS_PER_TOKEN)
}

/// Estimate tokens for a string.
pub fn estimate_tokens_for_text(text: &str) -> usize {
    text.chars().count().div_ceil(CHARS_PER_TOKEN)
}

/// Estimate tokens for a slice of TrimmedMessage.
pub fn estimate_tokens_for_trimmed(messages: &[crate::message::TrimmedMessage]) -> usize {
    let mut chars: usize = 0;
    for msg in messages {
        match msg {
            crate::message::TrimmedMessage::User(u) => {
                for block in &u.content {
                    if let crate::message::Content::Text(t) = block {
                        chars += t.text.chars().count();
                    }
                }
            }
            crate::message::TrimmedMessage::Assistant(a) => {
                for block in &a.content {
                    if let crate::message::Content::Text(t) = block {
                        chars += t.text.chars().count();
                    }
                    if let crate::message::Content::ToolCall(tc) = block {
                        chars += tc.name.as_str().chars().count();
                        chars += serde_json::to_string(&tc.arguments)
                            .map(|s| s.chars().count())
                            .unwrap_or(0);
                    }
                }
            }
            crate::message::TrimmedMessage::OriginalTool(tool) => {
                for block in &tool.content {
                    if let crate::message::Content::Text(t) = block {
                        chars += t.text.chars().count();
                    }
                }
            }
            crate::message::TrimmedMessage::ProjectedTool(tool) => {
                chars += tool.preview.chars().count();
            }
            crate::message::TrimmedMessage::Compaction(c) => {
                chars += c.summary.chars().count();
            }
        }
    }
    chars.div_ceil(CHARS_PER_TOKEN)
}

pub fn count_message_chars(messages: &[crate::message::AgentMessage]) -> usize {
    use crate::message::{AgentMessage, Content};
    let mut total = 0;
    for msg in messages {
        match msg {
            AgentMessage::User(u) => {
                for block in &u.content {
                    if let Content::Text(t) = block {
                        total += t.text.chars().count();
                    }
                }
            }
            AgentMessage::Assistant(a) => {
                for block in &a.content {
                    match block {
                        Content::Text(t) => total += t.text.chars().count(),
                        Content::ToolCall(tc) => {
                            total += tc.name.as_str().chars().count();
                            total += serde_json::to_string(&tc.arguments)
                                .map(|s| s.chars().count())
                                .unwrap_or(0);
                        }
                        Content::Image(_) => {}
                    }
                }
            }
            AgentMessage::ToolResult(tr) => {
                for block in &tr.content {
                    if let Content::Text(t) = block {
                        total += t.text.chars().count();
                    }
                }
            }
        }
    }
    total
}

fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

// ---------------------------------------------------------------------------
// New projection model (DESIGN.md)
// ---------------------------------------------------------------------------

/// Strategy for projecting a tool result.
#[derive(Debug, Clone, PartialEq)]
pub enum NewProjectionStrategy {
    KeepFull,
    Head { min_age: u32, max_chars: usize },
}

/// Look up projection strategy by tool name.
pub fn projection_strategy(_tool_name: &str) -> NewProjectionStrategy {
    NewProjectionStrategy::KeepFull
}

/// Run projection scan over T at turn end.
///
/// Iterates all OriginalTool entries in T. For each:
/// - If strategy is KeepFull -> skip
/// - If strategy is Head { min_age, max_chars }:
///   - age = current_turn - tool.turn
///   - if age >= min_age AND char_count > max_chars -> project
///     (OriginalTool -> ProjectedTool, original stored in A)
///
/// Returns `NewArtifacts` marker with entry_ids of newly archived originals.
pub fn projection_scan(
    t: &mut [crate::message::TrimmedMessage],
    a: &mut crate::message::Artifacts,
    current_turn: u32,
) -> Vec<ChangeMarker> {
    let mut new_artifacts = vec![];

    for msg in t.iter_mut() {
        let crate::message::TrimmedMessage::OriginalTool(tool) = msg else {
            continue;
        };

        let strategy = projection_strategy(tool.tool_name.as_str());
        let char_count = tool.content_char_count();

        match strategy {
            NewProjectionStrategy::KeepFull => {}
            NewProjectionStrategy::Head { min_age, max_chars } => {
                let age = current_turn.saturating_sub(tool.turn);
                if age >= min_age && char_count > max_chars {
                    let preview = tool.preview(max_chars);
                    let artifact_id = tool.entry_id.clone();

                    a.insert(artifact_id.clone(), tool.clone());
                    new_artifacts.push(artifact_id.clone());

                    *msg = crate::message::TrimmedMessage::ProjectedTool(
                        crate::message::ProjectedToolResult {
                            entry_id: tool.entry_id.clone(),
                            tool_call_id: tool.tool_call_id.clone(),
                            tool_name: tool.tool_name.clone(),
                            preview,
                            artifact_id,
                            original_char_count: char_count,
                            is_error: tool.is_error,
                        },
                    );
                }
            }
        }
    }

    if new_artifacts.is_empty() {
        vec![]
    } else {
        vec![ChangeMarker::NewArtifacts {
            entry_ids: new_artifacts,
        }]
    }
}

/// Build LLM context from transcript.
///
/// Converts each TrimmedMessage to the appropriate AgentMessage wire format:
/// - User -> AgentMessage::User
/// - Assistant -> AgentMessage::Assistant
/// - OriginalTool -> AgentMessage::ToolResult with full content
/// - ProjectedTool -> AgentMessage::ToolResult with preview wrapped in `<context-artifact>`
/// - Compaction -> AgentMessage::User(summary text)
pub fn build_llm_context_from_trimmed(
    t: &[crate::message::TrimmedMessage],
    system_prompt: &str,
    tools: &[crate::tool::ToolDefinition],
) -> crate::context::LlmContext {
    let mut messages = Vec::with_capacity(t.len());

    for msg in t {
        match msg {
            crate::message::TrimmedMessage::User(u) => {
                messages.push(crate::message::AgentMessage::User(u.clone()));
            }
            crate::message::TrimmedMessage::Assistant(a) => {
                // Empty assistant content (no text, no tool calls) is rejected
                // by Anthropic and carries no information — skip it.
                if a.is_empty() {
                    continue;
                }
                messages.push(crate::message::AgentMessage::Assistant(a.clone()));
            }
            crate::message::TrimmedMessage::OriginalTool(tool) => {
                messages.push(crate::message::AgentMessage::ToolResult(
                    crate::message::ToolResultMessage {
                        role: "tool_result".to_string(),
                        tool_call_id: tool.tool_call_id.clone(),
                        tool_name: tool.tool_name.clone(),
                        content: tool.content.clone(),
                        details: None,
                        is_error: tool.is_error,
                        timestamp: crate::timestamp::current_timestamp(),
                    },
                ));
            }
            crate::message::TrimmedMessage::ProjectedTool(tool) => {
                let preview_text = format!(
                    "<context-artifact id=\"{}\">\n{}\n</context-artifact>",
                    escape_xml(&tool.artifact_id),
                    escape_xml(&tool.preview),
                );
                messages.push(crate::message::AgentMessage::ToolResult(
                    crate::message::ToolResultMessage {
                        role: "tool_result".to_string(),
                        tool_call_id: tool.tool_call_id.clone(),
                        tool_name: tool.tool_name.clone(),
                        content: vec![crate::message::Content::Text(crate::message::TextContent {
                            text: preview_text,
                        })],
                        details: None,
                        is_error: tool.is_error,
                        timestamp: crate::timestamp::current_timestamp(),
                    },
                ));
            }
            crate::message::TrimmedMessage::Compaction(c) => {
                messages.push(crate::message::AgentMessage::User(
                    crate::message::UserMessage::new_text(format!(
                        "Previous conversation summary: {}",
                        c.summary
                    )),
                ));
            }
        }
    }

    crate::context::LlmContext {
        system_prompt: system_prompt.to_string(),
        messages,
        tools: tools.to_vec(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{AgentMessage, Content};

    fn user_msg(text: &str) -> AgentMessage {
        AgentMessage::User(crate::message::UserMessage::new_text(text))
    }

    // --- Token estimation tests ---

    #[test]
    fn token_estimate_counts_user_text() {
        let msgs = vec![user_msg("Hello, world!")]; // 13 chars -> ceil(13/4) = 4
        assert_eq!(estimate_tokens(&msgs), 13_usize.div_ceil(4));
    }

    #[test]
    fn token_estimate_for_text_simple() {
        assert_eq!(estimate_tokens_for_text("hello"), 5_usize.div_ceil(4));
    }

    // --- New projection model tests ---

    fn trimmed_user(text: &str) -> crate::message::TrimmedMessage {
        crate::message::TrimmedMessage::User(crate::message::UserMessage::new_text(text))
    }

    fn trimmed_assistant(text: &str) -> crate::message::TrimmedMessage {
        let mut msg = crate::message::AssistantMessage::empty();
        msg.content = vec![crate::message::Content::Text(crate::message::TextContent {
            text: text.into(),
        })];
        crate::message::TrimmedMessage::Assistant(msg)
    }

    fn trimmed_original_tool(
        entry_id: &str,
        tool_call_id: &str,
        tool_name: &str,
        text: &str,
        turn: u32,
    ) -> crate::message::TrimmedMessage {
        crate::message::TrimmedMessage::OriginalTool(crate::message::OriginalToolResult {
            entry_id: entry_id.to_string(),
            tool_call_id: crate::types::ToolCallId::new(tool_call_id),
            tool_name: crate::types::ToolName::new(tool_name),
            content: vec![crate::message::Content::Text(crate::message::TextContent {
                text: text.to_string(),
            })],
            is_error: false,
            turn,
        })
    }

    #[test]
    fn new_projection_scan_keeps_young_tool() {
        let mut t = vec![
            trimmed_user("hello"),
            trimmed_original_tool("e1", "tc1", "bash", &"A".repeat(5000), 3),
        ];
        let mut a = crate::message::Artifacts::new();
        let markers = projection_scan(&mut t, &mut a, 3); // age=0, min_age=2
        assert!(markers.is_empty(), "young tool should not be projected");
        assert!(matches!(
            t[1],
            crate::message::TrimmedMessage::OriginalTool(_)
        ));
    }

    #[test]
    fn new_projection_scan_respects_keep_full() {
        let mut t = vec![
            trimmed_user("hello"),
            trimmed_original_tool("e1", "tc1", "edit", &"A".repeat(5000), 3),
        ];
        let mut a = crate::message::Artifacts::new();
        let markers = projection_scan(&mut t, &mut a, 100); // edit = KeepFull
        assert!(markers.is_empty());
        assert!(matches!(
            t[1],
            crate::message::TrimmedMessage::OriginalTool(_)
        ));
    }

    #[test]
    fn new_projection_scan_skips_small_tool() {
        let mut t = vec![
            trimmed_user("hello"),
            trimmed_original_tool("e1", "tc1", "bash", "small", 1),
        ];
        let mut a = crate::message::Artifacts::new();
        let markers = projection_scan(&mut t, &mut a, 10); // age=9, but 5<2000
        assert!(markers.is_empty());
    }

    // Boundary: size == max_chars exactly (not >, so should NOT project)
    #[test]
    fn new_projection_scan_exact_max_chars_does_not_project() {
        let mut t = vec![
            trimmed_user("hello"),
            trimmed_original_tool("e1", "tc1", "read", &"A".repeat(2000), 1),
        ];
        let mut a = crate::message::Artifacts::new();
        let markers = projection_scan(&mut t, &mut a, 5); // age=4, min_age=2, 2000==2000
        assert!(markers.is_empty(), "exactly max_chars should NOT project");
        assert!(matches!(
            t[1],
            crate::message::TrimmedMessage::OriginalTool(_)
        ));
    }

    #[test]
    fn new_build_llm_context_original_tool_passes_through() {
        let t = vec![
            trimmed_user("hello"),
            trimmed_original_tool("e1", "tc1", "bash", "output", 0),
            trimmed_assistant("done"),
        ];
        let ctx = build_llm_context_from_trimmed(&t, "system", &[]);
        assert_eq!(ctx.system_prompt, "system");
        assert_eq!(ctx.messages.len(), 3);
        assert!(matches!(ctx.messages[1], AgentMessage::ToolResult(_)));
    }

    #[test]
    fn new_build_llm_context_projected_tool_uses_preview() {
        let t = vec![
            trimmed_user("hello"),
            crate::message::TrimmedMessage::ProjectedTool(crate::message::ProjectedToolResult {
                entry_id: "e1".to_string(),
                tool_call_id: crate::types::ToolCallId::new("tc1"),
                tool_name: crate::types::ToolName::new("bash"),
                preview: "truncated...".to_string(),
                artifact_id: "e1".to_string(),
                original_char_count: 5000,
                is_error: false,
            }),
        ];
        let ctx = build_llm_context_from_trimmed(&t, "system", &[]);
        assert_eq!(ctx.messages.len(), 2);
        if let AgentMessage::ToolResult(tr) = &ctx.messages[1] {
            let text = tr
                .content
                .iter()
                .filter_map(|b| match b {
                    Content::Text(t) => Some(t.text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            assert!(text.contains("<context-artifact id=\"e1\">"));
            assert!(text.contains("truncated..."));
            assert!(text.contains("</context-artifact>"));
        } else {
            panic!("expected tool result");
        }
    }

    #[test]
    fn new_build_llm_context_compaction_to_user_message() {
        let t = vec![
            crate::message::TrimmedMessage::Compaction(crate::message::CompactionSummary {
                summary: "old stuff".to_string(),
                compacted_entry_ids: vec!["e1".to_string()],
                tokens_before: 100,
            }),
            trimmed_user("new question"),
        ];
        let ctx = build_llm_context_from_trimmed(&t, "system", &[]);
        assert_eq!(ctx.messages.len(), 2);
        if let AgentMessage::User(u) = &ctx.messages[0] {
            let text = u
                .content
                .iter()
                .filter_map(|b| match b {
                    Content::Text(t) => Some(t.text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");
            assert!(text.contains("Previous conversation summary: old stuff"));
        } else {
            panic!("expected user message for compaction");
        }
    }

    #[test]
    fn build_llm_context_skips_empty_assistant_message() {
        // An empty assistant message (no text, no tool calls) must not be sent
        // to the provider — Anthropic rejects empty assistant content arrays.
        let mut empty_assistant = crate::message::AssistantMessage::empty();
        let _ = empty_assistant; // content stays Vec::new()
        let t = vec![
            trimmed_user("hello"),
            crate::message::TrimmedMessage::Assistant(crate::message::AssistantMessage::empty()),
            trimmed_user("next question"),
        ];
        let ctx = build_llm_context_from_trimmed(&t, "system", &[]);
        assert_eq!(
            ctx.messages.len(),
            2,
            "empty assistant message must be omitted from LLM context"
        );
    }

    #[test]
    fn estimate_tokens_for_trimmed_simple() {
        let t = vec![trimmed_user("hello world"), trimmed_assistant("hi there")];
        let tokens = estimate_tokens_for_trimmed(&t);
        assert!(tokens > 0);
        // "hello world" (11) + "hi there" (8) = 19 chars -> ceil(19/4) = 5
        assert_eq!(tokens, 19_usize.div_ceil(4));
    }

    #[test]
    fn change_marker_serde_roundtrip() {
        let marker = ChangeMarker::CompactionApplied;
        let json = serde_json::to_string(&marker).unwrap();
        let decoded: ChangeMarker = serde_json::from_str(&json).unwrap();
        assert_eq!(marker, decoded);

        let marker = ChangeMarker::NewArtifacts {
            entry_ids: vec!["entry-1".to_string(), "entry-2".to_string()],
        };
        let json = serde_json::to_string(&marker).unwrap();
        let decoded: ChangeMarker = serde_json::from_str(&json).unwrap();
        assert_eq!(marker, decoded);
    }
}
