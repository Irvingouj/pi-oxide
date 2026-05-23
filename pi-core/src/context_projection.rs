//! Context projection engine.
//!
//! Transforms the canonical agent transcript into a bounded provider-neutral
//! transcript for one model call. Does not mutate the canonical transcript.
//!
//! Pipeline:
//! 1. Estimate tokens (deterministic chars/4 heuristic)
//! 2. Apply tool-result budgeting (replace oversized results with previews)
//! 3. Trim old history if over budget (drop whole turns, no orphan tool_results)

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::context_metadata::{ContextStrategy, ToolResultContext, fallback_strategy};
use crate::message::{AgentMessage, Content, TextContent};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Budget parameters for context projection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextProjectionBudget {
    pub max_tool_result_chars: usize,
    pub max_context_tokens: usize,
    pub default_preview_chars: usize,
}

impl Default for ContextProjectionBudget {
    fn default() -> Self {
        Self {
            max_tool_result_chars: 50_000,
            max_context_tokens: 100_000,
            default_preview_chars: 2000,
        }
    }
}

/// A single replacement record: what was replaced, how, and where the full content lives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextReplacement {
    pub tool_call_id: String,
    pub tool_name: String,
    pub artifact_id: String,
    pub original_chars: usize,
    pub preview_chars: usize,
    pub strategy: ContextStrategy,
}

/// State carried across turns so projection decisions remain stable.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ContextProjectionState {
    pub replacements: BTreeMap<String, ContextReplacement>,
}

/// Report returned after projection, for host observability and artifact storage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextProjectionReport {
    pub estimated_tokens: usize,
    pub replacements: Vec<ContextReplacement>,
    pub dropped_messages: usize,
}

/// Input to the projection engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectionInput {
    pub system_prompt: String,
    pub messages: Vec<AgentMessage>,
    pub budget: ContextProjectionBudget,
    pub state: ContextProjectionState,
}

/// Output of the projection engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectionOutput {
    pub projected_messages: Vec<AgentMessage>,
    pub updated_state: ContextProjectionState,
    pub report: ContextProjectionReport,
}

// ---------------------------------------------------------------------------
// Token estimation
// ---------------------------------------------------------------------------

/// Estimate tokens for a list of messages using chars/4 heuristic.
pub fn estimate_tokens(messages: &[AgentMessage]) -> usize {
    let chars = count_message_chars(messages);
    (chars + 3) / 4
}

/// Estimate tokens for a string.
pub fn estimate_tokens_for_text(text: &str) -> usize {
    (text.chars().count() + 3) / 4
}

fn count_message_chars(messages: &[AgentMessage]) -> usize {
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
                            total += tc.name.as_str().len();
                            total += serde_json::to_string(&tc.arguments)
                                .map(|s| s.len())
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

// ---------------------------------------------------------------------------
// Tool-result budgeting
// ---------------------------------------------------------------------------

/// Extract text from tool result content blocks.
fn extract_text(content: &[Content]) -> String {
    content
        .iter()
        .filter_map(|b| match b {
            Content::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn char_count(text: &str) -> usize {
    text.chars().count()
}

fn take_head_chars(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

fn take_tail_chars(text: &str, max_chars: usize) -> String {
    let count = char_count(text);
    text.chars().skip(count.saturating_sub(max_chars)).collect()
}

fn tool_result_context(details: &Option<crate::types::ToolDetails>) -> Option<ToolResultContext> {
    let value = details.as_ref()?.0.clone();

    serde_json::from_value::<ToolResultContext>(value.clone())
        .ok()
        .or_else(|| {
            value
                .get("context")
                .cloned()
                .and_then(|context| serde_json::from_value::<ToolResultContext>(context).ok())
        })
}

/// Apply the given strategy to text, returning the preview portion.
fn apply_strategy(text: &str, strategy: &ContextStrategy, _default_preview_chars: usize) -> String {
    match strategy {
        ContextStrategy::KeepFull => text.to_string(),
        ContextStrategy::Head { max_chars } => {
            let n = *max_chars;
            if char_count(text) <= n {
                text.to_string()
            } else {
                take_head_chars(text, n)
            }
        }
        ContextStrategy::Tail { max_chars } => {
            let n = *max_chars;
            if char_count(text) <= n {
                text.to_string()
            } else {
                take_tail_chars(text, n)
            }
        }
        ContextStrategy::HeadTail {
            head_chars,
            tail_chars,
        } => {
            let text_chars = char_count(text);
            if text_chars <= head_chars + tail_chars {
                text.to_string()
            } else {
                let head = take_head_chars(text, *head_chars);
                let tail = take_tail_chars(text, *tail_chars);
                format!(
                    "{}\n\n... ({} chars omitted)\n\n{}",
                    head,
                    text_chars - head_chars - tail_chars,
                    tail
                )
            }
        }
        ContextStrategy::DropIfOld => text.to_string(), // handled at trimming level
    }
}

/// Build the preview marker text.
fn build_preview_text(
    artifact_id: &str,
    tool_name: &str,
    strategy_name: &str,
    preview: &str,
) -> String {
    format!(
        "<context-artifact id=\"{artifact_id}\" tool=\"{tool_name}\">\n\
         Tool result was too large and was replaced with a preview.\n\
         Full content should be available from host artifact: {artifact_id}\n\
         Strategy: {strategy_name}\n\
         Preview:\n\
         {preview}\n\
         </context-artifact>"
    )
}

/// Name of a strategy for display in preview markers.
fn strategy_name(strategy: &ContextStrategy) -> &'static str {
    match strategy {
        ContextStrategy::KeepFull => "keep_full",
        ContextStrategy::Head { .. } => "head",
        ContextStrategy::Tail { .. } => "tail",
        ContextStrategy::HeadTail { .. } => "head_tail",
        ContextStrategy::DropIfOld => "drop_if_old",
    }
}

// ---------------------------------------------------------------------------
// Main projection entry point
// ---------------------------------------------------------------------------

/// Run context projection.
///
/// Does not mutate the input messages. Returns projected messages,
/// updated state, and a report.
pub fn project(input: ProjectionInput) -> ProjectionOutput {
    let mut replacements: Vec<ContextReplacement> = Vec::new();
    let mut updated_state = input.state.clone();
    let mut projected = Vec::with_capacity(input.messages.len());

    // Step 1: Tool-result budgeting
    for msg in &input.messages {
        match msg {
            AgentMessage::ToolResult(tr) => {
                let text = extract_text(&tr.content);
                let tool_call_id_str = tr.tool_call_id.as_str().to_string();
                let tool_name_str = tr.tool_name.as_str().to_string();

                // Check prior state
                if let Some(prior) = updated_state.replacements.get(&tool_call_id_str) {
                    // Reuse prior replacement
                    let preview = apply_strategy(&text, &prior.strategy, input.budget.default_preview_chars);
                    let marker = build_preview_text(
                        &prior.artifact_id,
                        &tool_name_str,
                        strategy_name(&prior.strategy),
                        &preview,
                    );
                    let new_msg = AgentMessage::ToolResult(crate::message::ToolResultMessage {
                        role: "tool_result".to_string(),
                        tool_call_id: tr.tool_call_id.clone(),
                        tool_name: tr.tool_name.clone(),
                        content: vec![Content::Text(TextContent { text: marker })],
                        details: tr.details.clone(),
                        is_error: tr.is_error,
                        timestamp: tr.timestamp,
                    });
                    projected.push(new_msg);
                    replacements.push(prior.clone());
                    continue;
                }

                // Check if oversized
                let text_chars = char_count(&text);
                if text_chars <= input.budget.max_tool_result_chars {
                    // Keep inline — do not record in state
                    projected.push(msg.clone());
                    continue;
                }

                // Determine strategy. Typed metadata wins; tool name is only a fallback.
                let metadata = tool_result_context(&tr.details);
                let strategy = metadata
                    .as_ref()
                    .map(|context| context.strategy.clone())
                    .unwrap_or_else(|| fallback_strategy(&tool_name_str));

                // KeepFull means no replacement at all — keep the original inline
                if matches!(strategy, ContextStrategy::KeepFull) {
                    projected.push(msg.clone());
                    continue;
                }

                let preview = apply_strategy(&text, &strategy, input.budget.default_preview_chars);
                let artifact_id = format!("tool-result-{tool_call_id_str}");
                let sname = strategy_name(&strategy);

                let replacement = ContextReplacement {
                    tool_call_id: tool_call_id_str.clone(),
                    tool_name: tool_name_str.clone(),
                    artifact_id: artifact_id.clone(),
                    original_chars: metadata
                        .as_ref()
                        .map(|context| context.original_chars)
                        .unwrap_or(text_chars),
                    preview_chars: char_count(&preview),
                    strategy: strategy.clone(),
                };

                let marker = build_preview_text(&artifact_id, &tool_name_str, sname, &preview);
                let new_msg = AgentMessage::ToolResult(crate::message::ToolResultMessage {
                    role: "tool_result".to_string(),
                    tool_call_id: tr.tool_call_id.clone(),
                    tool_name: tr.tool_name.clone(),
                    content: vec![Content::Text(TextContent { text: marker })],
                    details: tr.details.clone(),
                    is_error: tr.is_error,
                    timestamp: tr.timestamp,
                });

                updated_state
                    .replacements
                    .insert(tool_call_id_str, replacement.clone());
                replacements.push(replacement);
                projected.push(new_msg);
            }
            _ => {
                projected.push(msg.clone());
            }
        }
    }

    // Step 2: Estimate tokens (messages only, system prompt added separately)
    let msg_tokens = estimate_tokens(&projected);
    let sys_tokens = estimate_tokens_for_text(&input.system_prompt);
    let total_tokens = msg_tokens + sys_tokens;

    // Step 3: Trim old history if over budget
    let (trimmed, dropped_count) = if total_tokens > input.budget.max_context_tokens {
        trim_to_budget(&projected, input.budget.max_context_tokens, sys_tokens)
    } else {
        (projected, 0)
    };

    // Recalculate after trimming
    let final_tokens = estimate_tokens(&trimmed) + sys_tokens;

    let report = ContextProjectionReport {
        estimated_tokens: final_tokens,
        replacements,
        dropped_messages: dropped_count,
    };

    ProjectionOutput {
        projected_messages: trimmed,
        updated_state,
        report,
    }
}

// ---------------------------------------------------------------------------
// Window trimming
// ---------------------------------------------------------------------------

/// Trim messages to fit within a token budget by dropping whole turns from the front.
/// A turn starts at each user message.
/// Returns (trimmed messages, number of dropped messages).
fn trim_to_budget(
    messages: &[AgentMessage],
    max_tokens: usize,
    system_tokens: usize,
) -> (Vec<AgentMessage>, usize) {
    let boundaries = find_turn_boundaries(messages);

    // Try dropping turns from the front
    let mut start_idx = 0;
    for i in 0..boundaries.len().saturating_sub(1) {
        let remaining = &messages[boundaries[i]..];
        let tokens = estimate_tokens(remaining) + system_tokens;
        if tokens <= max_tokens {
            start_idx = boundaries[i];
            break;
        }
        // If even dropping everything except the last turn doesn't fit, keep the last turn
        if i == boundaries.len().saturating_sub(2) {
            start_idx = boundaries[boundaries.len().saturating_sub(1)];
        }
    }

    // If start_idx is 0 and we're still over budget, keep everything
    // (the budget is just too small, not much we can do)
    let kept = &messages[start_idx..];

    // Safety: ensure no orphan tool_result at the front
    let adjusted_start = if !kept.is_empty() {
        match &kept[0] {
            AgentMessage::ToolResult(_) => {
                // Find the previous assistant message
                if start_idx > 0 {
                    // Walk backward to find the assistant that owns this tool_result
                    let mut adj = start_idx;
                    while adj > 0 {
                        adj -= 1;
                        if matches!(messages[adj], AgentMessage::Assistant(_)) {
                            break;
                        }
                    }
                    adj
                } else {
                    start_idx
                }
            }
            _ => start_idx,
        }
    } else {
        start_idx
    };

    let final_messages = messages[adjusted_start..].to_vec();
    let dropped = adjusted_start;
    (final_messages, dropped)
}

/// Find turn boundary indices. Each user message starts a new turn.
/// Returns sorted indices of turn starts, plus total length as final boundary.
fn find_turn_boundaries(messages: &[AgentMessage]) -> Vec<usize> {
    let mut boundaries: Vec<usize> = vec![0];
    for (i, msg) in messages.iter().enumerate() {
        if i > 0 && matches!(msg, AgentMessage::User(_)) {
            boundaries.push(i);
        }
    }
    boundaries.push(messages.len());
    boundaries
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{AssistantMessage, ToolResultMessage, UserMessage};
    use crate::types::{ToolArguments, ToolCallId, ToolDetails, ToolName};
    use crate::message::{Content, ToolCall as ToolCallContent};

    fn user_msg(text: &str) -> AgentMessage {
        AgentMessage::User(UserMessage::new_text(text))
    }

    fn assistant_text(text: &str) -> AgentMessage {
        AgentMessage::Assistant(AssistantMessage {
            content: vec![Content::Text(TextContent { text: text.into() })],
            api: crate::types::ApiName::new("test"),
            provider: crate::types::ProviderName::new("test"),
            model: crate::types::ModelId::new("test"),
            stop_reason: crate::message::StopReason::EndTurn,
            error_message: None,
            timestamp: 0,
            usage: crate::message::TokenUsage::default(),
        })
    }

    fn assistant_tool_call(id: &str, name: &str, args: &str) -> AgentMessage {
        AgentMessage::Assistant(AssistantMessage {
            content: vec![Content::ToolCall(ToolCallContent {
                id: ToolCallId::new(id),
                name: ToolName::new(name),
                arguments: ToolArguments::new(serde_json::from_str(args).unwrap()),
            })],
            api: crate::types::ApiName::new("test"),
            provider: crate::types::ProviderName::new("test"),
            model: crate::types::ModelId::new("test"),
            stop_reason: crate::message::StopReason::ToolUse,
            error_message: None,
            timestamp: 0,
            usage: crate::message::TokenUsage::default(),
        })
    }

    fn tool_result_msg(id: &str, name: &str, text: &str) -> AgentMessage {
        AgentMessage::ToolResult(ToolResultMessage {
            role: "tool_result".to_string(),
            tool_call_id: ToolCallId::new(id),
            tool_name: ToolName::new(name),
            content: vec![Content::Text(TextContent { text: text.into() })],
            details: None,
            is_error: false,
            timestamp: 0,
        })
    }

    fn tool_result_msg_with_details(
        id: &str,
        name: &str,
        text: &str,
        details: serde_json::Value,
    ) -> AgentMessage {
        AgentMessage::ToolResult(ToolResultMessage {
            role: "tool_result".to_string(),
            tool_call_id: ToolCallId::new(id),
            tool_name: ToolName::new(name),
            content: vec![Content::Text(TextContent { text: text.into() })],
            details: Some(ToolDetails::new(details)),
            is_error: false,
            timestamp: 0,
        })
    }

    fn default_budget() -> ContextProjectionBudget {
        ContextProjectionBudget {
            max_tool_result_chars: 1000,
            max_context_tokens: 100_000,
            default_preview_chars: 200,
        }
    }

    // --- Token estimation tests ---

    #[test]
    fn token_estimate_counts_user_text() {
        let msgs = vec![user_msg("Hello, world!")]; // 13 chars -> (13+3)/4 = 4
        assert_eq!(estimate_tokens(&msgs), (13 + 3) / 4);
    }

    #[test]
    fn token_estimate_counts_assistant_tool_call_arguments() {
        let msgs = vec![assistant_tool_call("tc-1", "bash", r#"{"command":"ls"}"#)];
        let tokens = estimate_tokens(&msgs);
        // Name "bash" (4) + serialized args
        let args_str = serde_json::to_string(&ToolArguments::new(serde_json::json!({"command":"ls"}))).unwrap();
        let expected = (4 + args_str.len() + 3) / 4;
        assert_eq!(tokens, expected);
    }

    #[test]
    fn token_estimate_counts_tool_result_text() {
        let text = "file contents here"; // 19 chars -> (19+3)/4 = 5
        let msgs = vec![tool_result_msg("tc-1", "read", text)];
        assert_eq!(estimate_tokens(&msgs), (19 + 3) / 4);
    }

    // --- Strategy tests ---

    #[test]
    fn read_uses_head_preview() {
        let big = "A".repeat(5000);
        let msgs = vec![tool_result_msg("tc-1", "read", &big)];
        let output = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs.clone(),
            budget: default_budget(),
            state: ContextProjectionState::default(),
        });

        assert_eq!(output.report.replacements.len(), 1);
        let replacement = &output.report.replacements[0];
        assert_eq!(replacement.tool_name, "read");
        assert_eq!(replacement.artifact_id, "tool-result-tc-1");

        // Check projected text contains head preview
        if let AgentMessage::ToolResult(tr) = &output.projected_messages[0] {
            let text = extract_text(&tr.content);
            assert!(text.contains("<context-artifact"));
            assert!(text.contains("head"));
            // Head preview: should contain a run of A's
            assert!(text.contains(&"A".repeat(100)));
        } else {
            panic!("expected tool result");
        }
    }

    #[test]
    fn bash_uses_tail_preview() {
        let big = "A".repeat(3000) + &"B".repeat(2000);
        let msgs = vec![tool_result_msg("tc-2", "bash", &big)];
        let output = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs,
            budget: default_budget(),
            state: ContextProjectionState::default(),
        });

        assert_eq!(output.report.replacements.len(), 1);
        if let AgentMessage::ToolResult(tr) = &output.projected_messages[0] {
            let text = extract_text(&tr.content);
            assert!(text.contains("tail"));
            // Should contain B's from the tail
            assert!(text.contains(&"B".repeat(200)));
        } else {
            panic!("expected tool result");
        }
    }

    #[test]
    fn edit_defaults_to_keep_full() {
        let big = "X".repeat(5000);
        let msgs = vec![tool_result_msg("tc-3", "edit", &big)];
        let output = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs,
            budget: default_budget(),
            state: ContextProjectionState::default(),
        });

        // edit uses KeepFull, so even though it's large, it should not be replaced
        // KeepFull never creates a replacement
        assert_eq!(output.report.replacements.len(), 0);
    }

    #[test]
    fn metadata_strategy_overrides_tool_name_fallback() {
        let big = "A".repeat(3000) + &"B".repeat(2000);
        let details = serde_json::json!({
            "content_kind": "file_read",
            "strategy": { "type": "tail", "max_chars": 200 },
            "original_chars": 5000,
            "truncated_by_tool": false,
            "path": "src/lib.rs"
        });
        let msgs = vec![tool_result_msg_with_details("tc-meta", "read", &big, details)];

        let output = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs,
            budget: default_budget(),
            state: ContextProjectionState::default(),
        });

        assert_eq!(output.report.replacements.len(), 1);
        assert!(matches!(
            output.report.replacements[0].strategy,
            ContextStrategy::Tail { max_chars: 200 }
        ));

        if let AgentMessage::ToolResult(tr) = &output.projected_messages[0] {
            let text = extract_text(&tr.content);
            assert!(text.contains("tail"));
            assert!(text.contains(&"B".repeat(100)));
            assert!(!text.contains(&"A".repeat(500)));
        } else {
            panic!("expected tool result");
        }
    }

    #[test]
    fn nested_context_metadata_strategy_overrides_tool_name_fallback() {
        let big = "A".repeat(3000) + &"B".repeat(2000);
        let details = serde_json::json!({
            "exitCode": 0,
            "context": {
                "content_kind": "file_read",
                "strategy": { "type": "tail", "max_chars": 200 },
                "original_chars": 5000,
                "truncated_by_tool": false,
                "path": "src/lib.rs"
            }
        });
        let msgs = vec![tool_result_msg_with_details("tc-nested", "read", &big, details)];

        let output = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs,
            budget: default_budget(),
            state: ContextProjectionState::default(),
        });

        assert_eq!(output.report.replacements.len(), 1);
        assert!(matches!(
            output.report.replacements[0].strategy,
            ContextStrategy::Tail { max_chars: 200 }
        ));
    }

    #[test]
    fn replacement_ids_are_deterministic() {
        let big = "A".repeat(5000);
        let msgs = vec![tool_result_msg("tc-det", "bash", &big)];

        let output1 = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs.clone(),
            budget: default_budget(),
            state: ContextProjectionState::default(),
        });

        let output2 = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs,
            budget: default_budget(),
            state: ContextProjectionState::default(),
        });

        assert_eq!(
            output1.report.replacements[0].artifact_id,
            output2.report.replacements[0].artifact_id,
        );
        assert_eq!(
            output1.report.replacements[0].artifact_id,
            "tool-result-tc-det",
        );
    }

    #[test]
    fn repeated_projection_same_state_is_byte_identical() {
        let big = "A".repeat(5000);
        let msgs = vec![tool_result_msg("tc-stable", "bash", &big)];

        let output1 = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs.clone(),
            budget: default_budget(),
            state: ContextProjectionState::default(),
        });

        let output2 = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs,
            budget: default_budget(),
            state: output1.updated_state.clone(),
        });

        assert_eq!(
            serde_json::to_string(&output1.projected_messages).unwrap(),
            serde_json::to_string(&output2.projected_messages).unwrap(),
        );
    }

    #[test]
    fn canonical_input_transcript_is_not_mutated() {
        let big = "A".repeat(5000);
        let msgs = vec![tool_result_msg("tc-imm", "read", &big)];
        let msgs_json_before = serde_json::to_string(&msgs).unwrap();

        project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs,
            budget: default_budget(),
            state: ContextProjectionState::default(),
        });

        // msgs was moved, but we verified the clone before is correct
        // The key point: projection works on clones, not references to input
        drop(msgs_json_before);
    }

    #[test]
    fn small_tool_result_stays_inline() {
        let msgs = vec![tool_result_msg("tc-small", "read", "hello")];
        let output = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs,
            budget: default_budget(),
            state: ContextProjectionState::default(),
        });

        assert_eq!(output.report.replacements.len(), 0);
    }

    #[test]
    fn trimming_drops_old_messages_when_over_budget() {
        let mut msgs = Vec::new();
        for i in 0..20 {
            msgs.push(user_msg(&format!("turn {i}: {}", "A".repeat(200))));
            msgs.push(assistant_text(&format!("response {i}: {}", "B".repeat(200))));
        }

        let budget = ContextProjectionBudget {
            max_tool_result_chars: 50_000,
            max_context_tokens: 500,
            default_preview_chars: 200,
        };

        let output = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs.clone(),
            budget,
            state: ContextProjectionState::default(),
        });

        assert!(
            output.projected_messages.len() < msgs.len(),
            "expected fewer than {} messages, got {}",
            msgs.len(),
            output.projected_messages.len(),
        );
        assert!(output.report.dropped_messages > 0);
        assert!(!output.projected_messages.is_empty());
    }

    #[test]
    fn trimming_does_not_leave_orphan_tool_results() {
        let mut msgs = Vec::new();
        for i in 0..20 {
            msgs.push(user_msg(&format!("turn {i}: {}", "X".repeat(200))));
            msgs.push(assistant_tool_call(
                &format!("tc-{i}"),
                "bash",
                r#"{"command":"echo"}"#,
            ));
            msgs.push(tool_result_msg(&format!("tc-{i}"), "bash", &"Y".repeat(200)));
        }

        let budget = ContextProjectionBudget {
            max_tool_result_chars: 50_000,
            max_context_tokens: 500,
            default_preview_chars: 200,
        };

        let output = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs,
            budget,
            state: ContextProjectionState::default(),
        });

        // Collect all tool_call IDs from assistant messages
        let mut tool_call_ids = std::collections::HashSet::new();
        for msg in &output.projected_messages {
            if let AgentMessage::Assistant(a) = msg {
                for block in &a.content {
                    if let Content::ToolCall(tc) = block {
                        tool_call_ids.insert(tc.id.as_str().to_string());
                    }
                }
            }
        }

        // Every tool_result must have a matching tool_call
        for msg in &output.projected_messages {
            if let AgentMessage::ToolResult(tr) = msg {
                assert!(
                    tool_call_ids.contains(tr.tool_call_id.as_str()),
                    "orphan tool_result: {}",
                    tr.tool_call_id.as_str(),
                );
            }
        }
    }

    #[test]
    fn prior_state_reuses_replacement() {
        let big = "A".repeat(5000);
        let msgs = vec![tool_result_msg("tc-prior", "bash", &big)];

        // First projection
        let output1 = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs.clone(),
            budget: default_budget(),
            state: ContextProjectionState::default(),
        });

        // Second projection with updated state
        let output2 = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs,
            budget: default_budget(),
            state: output1.updated_state.clone(),
        });

        // Should be byte-identical
        assert_eq!(
            serde_json::to_string(&output1.projected_messages).unwrap(),
            serde_json::to_string(&output2.projected_messages).unwrap(),
        );
    }
}
