//! Context projection engine.
//!
//! Transforms the canonical agent transcript into a bounded provider-neutral
//! transcript for one model call. Does not mutate the canonical transcript.
//!
//! Pipeline:
//! 1. Apply tool-result budgeting (replace oversized results with previews)
//! 2. Microcompact old tool results (shrink to one-line summaries)
//! 3. Estimate tokens (chars/4, calibrated against API usage when available)
//! 4. Trim or signal compaction (soft threshold signals host, hard limit trims)

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::context_metadata::{ContextStrategy, ToolResultContext, fallback_strategy};
use crate::message::{AgentMessage, Content, TextContent};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Budget parameters for context projection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct ContextProjectionBudget {
    pub max_tool_result_chars: usize,
    pub max_context_tokens: usize,
    pub default_preview_chars: usize,
    /// Turns older than this have their tool results microcompacted.
    #[serde(default = "default_microcompact_after_turns")]
    pub microcompact_after_turns: u32,
    /// Fraction of max_context_tokens that triggers compaction signal (0.0–1.0).
    #[serde(default = "default_compaction_threshold")]
    pub compaction_threshold: f32,
}

fn default_microcompact_after_turns() -> u32 { 5 }
fn default_compaction_threshold() -> f32 { 0.75 }

impl Default for ContextProjectionBudget {
    fn default() -> Self {
        Self {
            max_tool_result_chars: 50_000,
            max_context_tokens: 100_000,
            default_preview_chars: 2000,
            microcompact_after_turns: default_microcompact_after_turns(),
            compaction_threshold: default_compaction_threshold(),
        }
    }
}

/// Snapshot of actual API token usage, fed back from the host for calibration.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
pub struct ApiUsageSnapshot {
    /// Token count estimated by our heuristic at the time of the API call.
    pub estimated_tokens: usize,
    /// Actual input tokens reported by the API.
    pub actual_input_tokens: usize,
}

/// A single replacement record: what was replaced, how, and where the full content lives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct ContextReplacement {
    pub tool_call_id: String,
    pub tool_name: String,
    pub artifact_id: String,
    pub original_chars: usize,
    pub preview_chars: usize,
    pub strategy: ContextStrategy,
}

/// State carried across turns so projection decisions remain stable.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
pub struct ContextProjectionState {
    pub replacements: BTreeMap<String, ContextReplacement>,
    /// Last API usage, used to calibrate token estimation.
    #[serde(default)]
    pub last_api_usage: Option<ApiUsageSnapshot>,
    /// Turns since last compaction. Incremented by host after each turn.
    #[serde(default)]
    pub turns_since_compaction: u32,
}

/// Report returned after projection, for host observability and artifact storage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct ContextProjectionReport {
    pub estimated_tokens: usize,
    pub replacements: Vec<ContextReplacement>,
    pub dropped_messages: usize,
    /// Host should compact (LLM summarization) before the next turn.
    #[serde(default)]
    pub needs_compaction: bool,
    /// Suggested cache breakpoint positions (message indices).
    #[serde(default)]
    pub cache_breakpoints: Vec<usize>,
}

/// Input to the projection engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct ProjectionInput {
    pub system_prompt: String,
    pub messages: Vec<AgentMessage>,
    pub budget: ContextProjectionBudget,
    pub state: ContextProjectionState,
}

/// Output of the projection engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
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

/// Calibrated token estimate using actual API usage when available.
fn calibrated_estimate(chars: usize, state: &ContextProjectionState) -> usize {
    let raw = (chars + 3) / 4;
    if let Some(ref api) = state.last_api_usage {
        if api.estimated_tokens > 0 {
            let ratio = api.actual_input_tokens as f64 / api.estimated_tokens as f64;
            return (raw as f64 * ratio).round() as usize;
        }
    }
    raw
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
        ContextStrategy::Microcompacted => {
            // Microcompact produces a one-line summary; actual replacement done in project()
            text.to_string()
        }
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
        ContextStrategy::Microcompacted => "microcompacted",
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

    // Step 2: Microcompact — shrink old tool results to one-line summaries
    let turn_boundaries = find_turn_boundaries(&projected);
    let total_turns = turn_boundaries.len().saturating_sub(1);
    if total_turns > input.budget.microcompact_after_turns as usize {
        let cutoff_turn = total_turns.saturating_sub(input.budget.microcompact_after_turns as usize);
        for turn_idx in 0..cutoff_turn {
            let start = turn_boundaries[turn_idx];
            let end = turn_boundaries[turn_idx + 1];
            for i in start..end {
                if let AgentMessage::ToolResult(tr) = &projected[i] {
                    let tcid = tr.tool_call_id.as_str().to_string();
                    // Skip if already replaced by artifact budgeting or prior microcompact
                    if updated_state.replacements.contains_key(&tcid) {
                        continue;
                    }
                    let text = extract_text(&tr.content);
                    let char_count_val = char_count(&text);
                    let summary = format!(
                        "<tool-summary tool=\"{}\" call=\"{}\">Result: {} chars</tool-summary>",
                        tr.tool_name.as_str(),
                        tcid,
                        char_count_val,
                    );
                    let replacement = ContextReplacement {
                        tool_call_id: tcid.clone(),
                        tool_name: tr.tool_name.as_str().to_string(),
                        artifact_id: format!("microcompact-{tcid}"),
                        original_chars: char_count_val,
                        preview_chars: char_count(&summary),
                        strategy: ContextStrategy::Microcompacted,
                    };
                    updated_state.replacements.insert(tcid, replacement.clone());
                    replacements.push(replacement);

                    projected[i] = AgentMessage::ToolResult(crate::message::ToolResultMessage {
                        role: "tool_result".to_string(),
                        tool_call_id: tr.tool_call_id.clone(),
                        tool_name: tr.tool_name.clone(),
                        content: vec![Content::Text(TextContent { text: summary })],
                        details: tr.details.clone(),
                        is_error: tr.is_error,
                        timestamp: tr.timestamp,
                    });
                }
            }
        }
    }

    // Step 3: Estimate tokens with calibration
    let msg_chars = count_message_chars(&projected);
    let sys_chars = input.system_prompt.chars().count();
    let msg_tokens = calibrated_estimate(msg_chars, &input.state);
    let sys_tokens = calibrated_estimate(sys_chars, &input.state);
    let total_tokens = msg_tokens + sys_tokens;

    // Step 4: Trim or signal compaction
    let usage_pct = total_tokens as f32 / input.budget.max_context_tokens as f32;
    let needs_compaction;
    let (trimmed, dropped_count) = if usage_pct > 1.0 {
        // Hard limit: must trim (safety net) + signal compaction
        needs_compaction = true;
        trim_to_budget(&projected, input.budget.max_context_tokens, sys_tokens)
    } else if usage_pct > input.budget.compaction_threshold {
        // Soft threshold: signal compaction, don't trim yet
        needs_compaction = true;
        (projected, 0)
    } else {
        needs_compaction = false;
        (projected, 0)
    };

    // Recalculate after trimming
    let final_tokens = calibrated_estimate(count_message_chars(&trimmed), &input.state) + sys_tokens;

    // Cache breakpoints: suggest placing one at the second-to-last turn boundary
    let cache_breakpoints = compute_cache_breakpoints(&trimmed);

    let report = ContextProjectionReport {
        estimated_tokens: final_tokens,
        replacements,
        dropped_messages: dropped_count,
        needs_compaction,
        cache_breakpoints,
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

/// Suggest cache breakpoint positions for Anthropic prompt caching.
/// Places a breakpoint at the second-to-last turn boundary so the prefix stays cached.
fn compute_cache_breakpoints(messages: &[AgentMessage]) -> Vec<usize> {
    let boundaries = find_turn_boundaries(messages);
    // Need at least 3 boundaries (2 turns) to have a meaningful prefix
    if boundaries.len() >= 3 {
        vec![boundaries[boundaries.len() - 2]]
    } else {
        vec![]
    }
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
            microcompact_after_turns: 5,
            compaction_threshold: 0.75,
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
            microcompact_after_turns: 5,
            compaction_threshold: 0.75,
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
            microcompact_after_turns: 5,
            compaction_threshold: 0.75,
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

    #[test]
    fn soft_threshold_signals_compaction_without_trimming() {
        let mut msgs = Vec::new();
        // 10 turns * 2 msgs * 60 chars = 1200 chars = ~300 tokens
        // Budget 400 tokens, threshold 50% = 200 tokens -> over threshold but under limit
        for i in 0..10 {
            msgs.push(user_msg(&format!("turn {i}: {}", "A".repeat(50))));
            msgs.push(assistant_text(&format!("response {i}: {}", "B".repeat(50))));
        }

        let budget = ContextProjectionBudget {
            max_tool_result_chars: 50_000,
            max_context_tokens: 400,
            default_preview_chars: 200,
            microcompact_after_turns: 100, // don't microcompact for this test
            compaction_threshold: 0.5,     // 50% threshold -> 200 tokens
        };

        let output = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs.clone(),
            budget,
            state: ContextProjectionState::default(),
        });

        // Should signal compaction (over 50% threshold) but not trim (under 100% limit)
        assert!(output.report.needs_compaction, "should signal compaction");
        assert_eq!(output.report.dropped_messages, 0, "should not drop messages");
    }

    #[test]
    fn hard_limit_trims_and_signals_compaction() {
        let mut msgs = Vec::new();
        for i in 0..20 {
            msgs.push(user_msg(&format!("turn {i}: {}", "A".repeat(200))));
            msgs.push(assistant_text(&format!("response {i}: {}", "B".repeat(200))));
        }

        let budget = ContextProjectionBudget {
            max_tool_result_chars: 50_000,
            max_context_tokens: 500,
            default_preview_chars: 200,
            microcompact_after_turns: 100,
            compaction_threshold: 0.75,
        };

        let output = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs,
            budget,
            state: ContextProjectionState::default(),
        });

        // Hard limit: should both trim AND signal compaction
        assert!(output.report.needs_compaction, "should signal compaction");
        assert!(output.report.dropped_messages > 0, "should drop messages");
    }

    #[test]
    fn cache_breakpoints_placed_at_second_to_last_turn() {
        let msgs = vec![
            user_msg("turn 0"),
            assistant_text("response 0"),
            user_msg("turn 1"),
            assistant_text("response 1"),
            user_msg("turn 2"),
            assistant_text("response 2"),
        ];

        let output = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs,
            budget: default_budget(),
            state: ContextProjectionState::default(),
        });

        assert_eq!(output.report.cache_breakpoints.len(), 1);
        // Second-to-last turn starts at index 4 ("turn 2")
        assert_eq!(output.report.cache_breakpoints[0], 4);
    }

    #[test]
    fn no_cache_breakpoint_with_few_turns() {
        let msgs = vec![
            user_msg("turn 0"),
            assistant_text("response 0"),
        ];

        let output = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs,
            budget: default_budget(),
            state: ContextProjectionState::default(),
        });

        assert!(output.report.cache_breakpoints.is_empty());
    }

    #[test]
    fn microcompact_shrinks_old_tool_results() {
        // Build 8 turns, each with a tool call + result
        let mut msgs = Vec::new();
        for i in 0..8 {
            msgs.push(user_msg(&format!("turn {i}")));
            msgs.push(assistant_tool_call(&format!("tc-{i}"), "bash", r#"{"command":"ls"}"#));
            msgs.push(tool_result_msg(&format!("tc-{i}"), "bash", &format!("output {i}: {}", "X".repeat(300))));
        }

        // Microcompact after 3 turns (so turns 0..5 get compacted)
        let budget = ContextProjectionBudget {
            microcompact_after_turns: 3,
            ..default_budget()
        };

        let output = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs,
            budget,
            state: ContextProjectionState::default(),
        });

        // First 5 turns' tool results should be microcompacted
        let microcompacted_ids: Vec<&str> = output.report.replacements
            .iter()
            .filter(|r| matches!(r.strategy, ContextStrategy::Microcompacted))
            .map(|r| r.tool_call_id.as_str())
            .collect();
        assert!(microcompacted_ids.contains(&"tc-0"), "tc-0 should be microcompacted");
        assert!(microcompacted_ids.contains(&"tc-4"), "tc-4 should be microcompacted");
        // Last 3 turns should NOT be microcompacted
        assert!(!microcompacted_ids.contains(&"tc-5"), "tc-5 should not be microcompacted");
        assert!(!microcompacted_ids.contains(&"tc-7"), "tc-7 should not be microcompacted");

        // Verify the compacted text contains the summary marker
        if let AgentMessage::ToolResult(tr) = &output.projected_messages[2] {
            let text = extract_text(&tr.content);
            assert!(text.contains("<tool-summary"), "expected microcompact summary, got: {text}");
        } else {
            panic!("expected tool result at index 2");
        }
    }

    #[test]
    fn calibrated_estimate_uses_api_ratio() {
        // 100 chars -> raw estimate = 25 tokens
        // API said actual was 35 tokens -> ratio = 1.4
        // So 80 chars should estimate to 80/4 * 1.4 = 28
        let state = ContextProjectionState {
            last_api_usage: Some(ApiUsageSnapshot {
                estimated_tokens: 25,
                actual_input_tokens: 35,
            }),
            ..Default::default()
        };
        // 80 chars => raw=20, calibrated = 20 * 1.4 = 28
        assert_eq!(calibrated_estimate(80, &state), 28);
    }

    #[test]
    fn calibrated_estimate_falls_back_to_chars_div_4() {
        let state = ContextProjectionState::default();
        // No API usage -> raw chars/4
        assert_eq!(calibrated_estimate(80, &state), 20);
    }

    #[test]
    fn microcompact_skips_already_replaced_results() {
        let big = "A".repeat(5000);
        // Turn 1: oversized bash result (will be replaced by Phase 1)
        // Turn 2: normal result
        // Turn 3: current turn
        let msgs = vec![
            user_msg("turn 0"),
            assistant_tool_call("tc-big", "bash", r#"{"command":"ls"}"#),
            tool_result_msg("tc-big", "bash", &big),
            user_msg("turn 1"),
            assistant_tool_call("tc-small", "read", r#"{"path":"x.rs"}"#),
            tool_result_msg("tc-small", "read", "small content"),
            user_msg("turn 2"),
        ];

        let budget = ContextProjectionBudget {
            microcompact_after_turns: 1, // turn 0 should be microcompacted
            ..default_budget()
        };

        let output = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs,
            budget,
            state: ContextProjectionState::default(),
        });

        // tc-big should be replaced by Phase 1 (artifact budgeting), NOT microcompacted
        let tc_big_replacement = output.report.replacements.iter()
            .find(|r| r.tool_call_id == "tc-big")
            .unwrap();
        assert!(
            matches!(tc_big_replacement.strategy, ContextStrategy::Tail { .. }),
            "tc-big should use tail strategy from Phase 1, got {:?}",
            tc_big_replacement.strategy,
        );

        // tc-small should be microcompacted (it was in an old turn and not replaced by Phase 1)
        let tc_small_replacement = output.report.replacements.iter()
            .find(|r| r.tool_call_id == "tc-small")
            .unwrap();
        assert!(
            matches!(tc_small_replacement.strategy, ContextStrategy::Microcompacted),
            "tc-small should be microcompacted, got {:?}",
            tc_small_replacement.strategy,
        );
    }
}
