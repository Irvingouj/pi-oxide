use crate::context_projection::{estimate_tokens_for_text, ContextProjectionBudget};
use crate::message::{AgentMessage, Artifacts, CompactionSummary, Content, TrimmedMessage};
use serde::{Deserialize, Serialize};

/// Plan for compacting old trimmed messages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompactionPlan {
    /// Index into T: messages before this should be summarized.
    pub cut_index: usize,
    /// Messages that will be summarized (indices [0..cut_index]).
    pub messages_to_summarize: Vec<TrimmedMessage>,
    /// Estimated tokens to free.
    pub tokens_to_free: usize,
}

/// Estimate tokens for a slice of TrimmedMessage.
fn estimate_tokens_for_trimmed(messages: &[TrimmedMessage]) -> usize {
    let mut chars: usize = 0;
    for msg in messages {
        match msg {
            TrimmedMessage::User(u) => {
                for block in &u.content {
                    if let Content::Text(t) = block {
                        chars += t.text.chars().count();
                    }
                }
            }
            TrimmedMessage::Assistant(a) => {
                for block in &a.content {
                    if let Content::Text(t) = block {
                        chars += t.text.chars().count();
                    }
                    // Tool calls are also counted
                    if let Content::ToolCall(tc) = block {
                        chars += tc.name.as_str().chars().count();
                        chars += serde_json::to_string(&tc.arguments)
                            .map(|s| s.chars().count())
                            .unwrap_or(0);
                    }
                }
            }
            TrimmedMessage::OriginalTool(tool) => {
                for block in &tool.content {
                    if let Content::Text(t) = block {
                        chars += t.text.chars().count();
                    }
                }
            }
            TrimmedMessage::ProjectedTool(tool) => {
                chars += tool.preview.chars().count();
            }
            TrimmedMessage::Compaction(c) => {
                chars += c.summary.chars().count();
            }
        }
    }
    estimate_tokens_for_text(&" ".repeat(chars)) // Reuse the text estimator
}

/// Extract AgentMessages from TrimmedMessages for summarization context.
fn trimmed_to_agent_messages(messages: &[TrimmedMessage]) -> Vec<AgentMessage> {
    let mut result = Vec::with_capacity(messages.len());
    for msg in messages {
        match msg {
            TrimmedMessage::User(u) => {
                result.push(AgentMessage::User(u.clone()));
            }
            TrimmedMessage::Assistant(a) => {
                if a.is_empty() {
                    continue;
                }
                result.push(AgentMessage::Assistant(a.clone()));
            }
            TrimmedMessage::OriginalTool(tool) => {
                result.push(AgentMessage::ToolResult(
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
            TrimmedMessage::ProjectedTool(tool) => {
                let preview_text = format!(
                    "<context-artifact id=\"{}\">\n{}\n</context-artifact>",
                    tool.artifact_id, tool.preview,
                );
                result.push(AgentMessage::ToolResult(
                    crate::message::ToolResultMessage {
                        role: "tool_result".to_string(),
                        tool_call_id: tool.tool_call_id.clone(),
                        tool_name: tool.tool_name.clone(),
                        content: vec![Content::Text(crate::message::TextContent {
                            text: preview_text,
                        })],
                        details: None,
                        is_error: tool.is_error,
                        timestamp: crate::timestamp::current_timestamp(),
                    },
                ));
            }
            TrimmedMessage::Compaction(c) => {
                result.push(AgentMessage::user(format!(
                    "Previous conversation summary: {}",
                    c.summary
                )));
            }
        }
    }
    result
}

/// Plan which trimmed messages to compact based on the budget.
pub fn plan_compaction(
    t: &[TrimmedMessage],
    budget: &ContextProjectionBudget,
) -> Option<CompactionPlan> {
    if t.is_empty() {
        return None;
    }

    let total_tokens = estimate_tokens_for_trimmed(t);
    let threshold = (budget.compaction_threshold * budget.max_context_tokens as f32) as usize;

    if total_tokens <= threshold {
        return None;
    }

    // Find turn boundaries at User messages
    let mut boundaries = vec![0];
    for (i, msg) in t.iter().enumerate() {
        if i > 0 && matches!(msg, TrimmedMessage::User(_)) {
            boundaries.push(i);
        }
    }
    boundaries.push(t.len());

    let num_turns = boundaries.len().saturating_sub(1);
    if num_turns == 0 {
        return None;
    }

    let target_keep_tokens =
        (budget.compaction_threshold * budget.max_context_tokens as f32 / 2.0) as usize;

    // Start with keeping the last turn
    let mut cut_index = boundaries[num_turns.saturating_sub(1)];
    let mut kept_tokens = estimate_tokens_for_trimmed(&t[cut_index..]);

    // Try to keep more turns going backwards
    for i in (0..num_turns.saturating_sub(1)).rev() {
        let turn_start = boundaries[i];
        let turn_end = boundaries[i + 1];
        let turn_tokens = estimate_tokens_for_trimmed(&t[turn_start..turn_end]);

        if kept_tokens + turn_tokens > target_keep_tokens {
            break;
        }
        kept_tokens += turn_tokens;
        cut_index = turn_start;
    }

    // Adjust for tool split safety: never split a ToolCall/ToolResult pair
    let mut earliest_split = cut_index;
    let mut tool_call_indices: std::collections::HashMap<&str, usize> =
        std::collections::HashMap::new();
    let mut tool_result_indices: std::collections::HashMap<&str, usize> =
        std::collections::HashMap::new();

    for (i, msg) in t.iter().enumerate() {
        match msg {
            TrimmedMessage::Assistant(a) => {
                for block in &a.content {
                    if let Content::ToolCall(tc) = block {
                        tool_call_indices.insert(tc.id.as_str(), i);
                    }
                }
            }
            TrimmedMessage::OriginalTool(tool) => {
                tool_result_indices.insert(tool.tool_call_id.as_str(), i);
            }
            TrimmedMessage::ProjectedTool(tool) => {
                tool_result_indices.insert(tool.tool_call_id.as_str(), i);
            }
            _ => {}
        }
    }

    for (id, call_idx) in &tool_call_indices {
        if let Some(result_idx) = tool_result_indices.get(id) {
            let call_before = *call_idx < cut_index;
            let result_before = *result_idx < cut_index;
            if call_before != result_before {
                earliest_split = earliest_split.min(*call_idx.min(result_idx));
            }
        }
    }

    if earliest_split < cut_index {
        // Snap back to the nearest turn boundary that is <= earliest_split
        cut_index = 0;
        for &boundary in &boundaries {
            if boundary <= earliest_split {
                cut_index = boundary;
            } else {
                break;
            }
        }
    }

    if cut_index == 0 {
        return None;
    }

    let messages_to_summarize = t[..cut_index].to_vec();
    let tokens_to_free = estimate_tokens_for_trimmed(&messages_to_summarize);

    Some(CompactionPlan {
        cut_index,
        messages_to_summarize,
        tokens_to_free,
    })
}

/// Apply a compaction plan to T and archive OriginalTool originals to A.
pub fn apply_compaction(
    t: Vec<TrimmedMessage>,
    plan: CompactionPlan,
    summary: String,
    a: &mut Artifacts,
) -> Vec<TrimmedMessage> {
    let cut_index = plan.cut_index;

    // Archive OriginalTool results that are being compacted
    let compacted_entry_ids: Vec<String> = t[..cut_index]
        .iter()
        .filter_map(|msg| match msg {
            TrimmedMessage::OriginalTool(tool) => Some(tool.entry_id.clone()),
            _ => None,
        })
        .collect();

    // Only archive to A if they're not already there
    for msg in &t[..cut_index] {
        if let TrimmedMessage::OriginalTool(tool) = msg {
            if !a.contains_key(&tool.entry_id) {
                a.insert(tool.entry_id.clone(), tool.clone());
            }
        }
    }

    let tokens_before = estimate_tokens_for_trimmed(&plan.messages_to_summarize);

    let compaction = CompactionSummary {
        summary,
        compacted_entry_ids,
        tokens_before,
    };

    let mut result = Vec::with_capacity(t.len() - cut_index + 1);
    result.push(TrimmedMessage::Compaction(compaction));
    result.extend(t.into_iter().skip(cut_index));
    result
}

/// Extract AgentMessages from TrimmedMessages for building a summarization context.
/// Used by build_summary_action to provide messages for the summarization LLM.
pub fn build_summary_messages(plan: &CompactionPlan) -> Vec<AgentMessage> {
    trimmed_to_agent_messages(&plan.messages_to_summarize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{
        AssistantMessage, Content, OriginalToolResult, StopReason, TextContent, ToolCall,
    };
    use crate::types::{ToolArguments, ToolCallId, ToolName};

    #[test]
    fn plan_compaction_empty() {
        let t: Vec<TrimmedMessage> = vec![];
        let budget = ContextProjectionBudget::default();
        assert!(plan_compaction(&t, &budget).is_none());
    }

    #[test]
    fn plan_compaction_under_budget() {
        let t = vec![
            TrimmedMessage::User(crate::message::UserMessage::new_text("hello")),
            TrimmedMessage::Assistant(AssistantMessage::empty()),
        ];
        let budget = ContextProjectionBudget {
            max_context_tokens: 100_000,
            compaction_threshold: 0.75,
            ..Default::default()
        };
        assert!(plan_compaction(&t, &budget).is_none());
    }

    #[test]
    fn plan_compaction_over_budget() {
        let mut t = Vec::new();
        for i in 0..10 {
            t.push(TrimmedMessage::User(crate::message::UserMessage::new_text(
                "A".repeat(10_000),
            )));
            t.push(TrimmedMessage::Assistant(AssistantMessage {
                content: vec![Content::Text(TextContent {
                    text: "B".repeat(10_000),
                })],
                api: "test".into(),
                provider: "test".into(),
                model: "test".into(),
                stop_reason: StopReason::EndTurn,
                error_message: None,
                timestamp: (i as u64) * 2 + 1,
                usage: Default::default(),
            }));
        }
        let budget = ContextProjectionBudget {
            max_context_tokens: 1_000,
            compaction_threshold: 0.75,
            ..Default::default()
        };
        let plan = plan_compaction(&t, &budget);
        assert!(plan.is_some());
        let plan = plan.unwrap();
        assert!(plan.cut_index > 0);
        assert!(plan.cut_index < t.len());
        assert!(plan.tokens_to_free > 0);
    }

    #[test]
    fn plan_compaction_respects_keep_recent() {
        let mut t = Vec::new();
        for i in 0..10 {
            t.push(TrimmedMessage::User(crate::message::UserMessage::new_text(
                "A".repeat(1_000),
            )));
            t.push(TrimmedMessage::Assistant(AssistantMessage {
                content: vec![Content::Text(TextContent {
                    text: "B".repeat(1_000),
                })],
                api: "test".into(),
                provider: "test".into(),
                model: "test".into(),
                stop_reason: StopReason::EndTurn,
                error_message: None,
                timestamp: (i as u64) * 2 + 1,
                usage: Default::default(),
            }));
        }
        let budget = ContextProjectionBudget {
            max_context_tokens: 1_000,
            compaction_threshold: 0.5,
            ..Default::default()
        };
        let plan = plan_compaction(&t, &budget).unwrap();
        // The last turn starts at index 18, so cut_index should be <= 18
        assert!(plan.cut_index <= 18);
        assert!(plan.cut_index > 0);
    }

    #[test]
    fn plan_compaction_never_cuts_mid_tool() {
        let t = vec![
            TrimmedMessage::User(crate::message::UserMessage::new_text("A".repeat(500))),
            TrimmedMessage::Assistant(AssistantMessage {
                content: vec![
                    Content::Text(TextContent {
                        text: "tool call".to_string(),
                    }),
                    Content::ToolCall(ToolCall {
                        id: ToolCallId::new("tc1"),
                        name: ToolName::new("bash"),
                        arguments: ToolArguments::new(serde_json::json!({})),
                    }),
                ],
                api: "test".into(),
                provider: "test".into(),
                model: "test".into(),
                stop_reason: StopReason::ToolUse,
                error_message: None,
                timestamp: 2,
                usage: Default::default(),
            }),
            TrimmedMessage::OriginalTool(OriginalToolResult {
                entry_id: "t0".to_string(),
                tool_call_id: ToolCallId::new("tc1"),
                tool_name: ToolName::new("bash"),
                content: vec![Content::Text(TextContent {
                    text: "B".repeat(500),
                })],
                is_error: false,
                turn: 0,
            }),
            TrimmedMessage::User(crate::message::UserMessage::new_text("C".repeat(500))),
            TrimmedMessage::Assistant(AssistantMessage {
                content: vec![Content::Text(TextContent {
                    text: "D".repeat(500),
                })],
                api: "test".into(),
                provider: "test".into(),
                model: "test".into(),
                stop_reason: StopReason::EndTurn,
                error_message: None,
                timestamp: 5,
                usage: Default::default(),
            }),
        ];

        let budget = ContextProjectionBudget {
            max_context_tokens: 500,
            compaction_threshold: 0.5,
            ..Default::default()
        };

        let plan = plan_compaction(&t, &budget).unwrap();
        // The tool call is at index 1 and the tool result is at index 2.
        // The cut should never be at index 1 or 2 (which would split the pair).
        assert_ne!(plan.cut_index, 1);
        assert_ne!(plan.cut_index, 2);
    }

    #[test]
    fn apply_compaction_creates_compaction_message() {
        let t = vec![
            TrimmedMessage::User(crate::message::UserMessage::new_text("hello")),
            TrimmedMessage::Assistant(AssistantMessage::empty()),
        ];

        let plan = CompactionPlan {
            cut_index: 1,
            messages_to_summarize: t[..1].to_vec(),
            tokens_to_free: 10,
        };

        let mut a = Artifacts::new();
        let result = apply_compaction(t, plan, "summary".to_string(), &mut a);
        assert_eq!(result.len(), 2);
        assert!(matches!(result[0], TrimmedMessage::Compaction(_)));
        if let TrimmedMessage::Compaction(ref c) = result[0] {
            assert_eq!(c.summary, "summary");
        }
    }

    #[test]
    fn apply_compaction_replaces_old_messages() {
        let t = vec![
            TrimmedMessage::User(crate::message::UserMessage::new_text("hello")),
            TrimmedMessage::Assistant(AssistantMessage::empty()),
            TrimmedMessage::User(crate::message::UserMessage::new_text("world")),
        ];

        let plan = CompactionPlan {
            cut_index: 2,
            messages_to_summarize: t[..2].to_vec(),
            tokens_to_free: 10,
        };

        let mut a = Artifacts::new();
        let result = apply_compaction(t, plan, "summary".to_string(), &mut a);
        assert_eq!(result.len(), 2);
        assert!(matches!(result[0], TrimmedMessage::Compaction(_)));
        assert!(matches!(result[1], TrimmedMessage::User(_)));
    }

    #[test]
    fn apply_compaction_archives_original_tools() {
        let t = vec![
            TrimmedMessage::User(crate::message::UserMessage::new_text("hello")),
            TrimmedMessage::OriginalTool(OriginalToolResult {
                entry_id: "entry-tool-1".to_string(),
                tool_call_id: ToolCallId::new("tc1"),
                tool_name: ToolName::new("bash"),
                content: vec![Content::Text(TextContent {
                    text: "tool output".to_string(),
                })],
                is_error: false,
                turn: 0,
            }),
            TrimmedMessage::User(crate::message::UserMessage::new_text("next")),
        ];

        let plan = CompactionPlan {
            cut_index: 2,
            messages_to_summarize: t[..2].to_vec(),
            tokens_to_free: 10,
        };

        let mut a = Artifacts::new();
        let result = apply_compaction(t, plan, "summary".to_string(), &mut a);
        assert!(
            a.contains_key("entry-tool-1"),
            "OriginalTool should be archived to Artifacts"
        );
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn build_summary_messages_extracts_for_llm() {
        let t = vec![
            TrimmedMessage::User(crate::message::UserMessage::new_text("hello")),
            TrimmedMessage::Assistant(AssistantMessage {
                content: vec![Content::Text(TextContent {
                    text: "hi there".to_string(),
                })],
                api: "test".into(),
                provider: "test".into(),
                model: "test".into(),
                stop_reason: StopReason::EndTurn,
                error_message: None,
                timestamp: 2,
                usage: Default::default(),
            }),
        ];

        let plan = CompactionPlan {
            cut_index: 2,
            messages_to_summarize: t.clone(),
            tokens_to_free: 10,
        };

        let msgs = build_summary_messages(&plan);
        assert_eq!(msgs.len(), 2);
        assert!(matches!(msgs[0], AgentMessage::User(_)));
        assert!(matches!(msgs[1], AgentMessage::Assistant(_)));
    }

    #[test]
    fn build_summary_messages_skips_empty_assistant() {
        // A restored legacy transcript may contain an empty assistant message.
        // The summarization LLM context must omit it, or the summarization
        // request can hit the same "empty assistant content" 400 as the main
        // turn path. Mirrors the guard in build_llm_context_from_trimmed.
        let t = vec![
            TrimmedMessage::User(crate::message::UserMessage::new_text("hello")),
            TrimmedMessage::Assistant(AssistantMessage::empty()),
            TrimmedMessage::User(crate::message::UserMessage::new_text("again")),
        ];

        let plan = CompactionPlan {
            cut_index: 3,
            messages_to_summarize: t,
            tokens_to_free: 10,
        };

        let msgs = build_summary_messages(&plan);
        assert_eq!(
            msgs.len(),
            2,
            "empty assistant must be omitted from summarization context"
        );
        assert!(matches!(msgs[0], AgentMessage::User(_)));
        assert!(matches!(msgs[1], AgentMessage::User(_)));
    }

    #[test]
    fn apply_compaction_with_projected_tool_entries() {
        let t = vec![
            TrimmedMessage::User(crate::message::UserMessage::new_text("A".repeat(1000))),
            // A ProjectedTool — already projected, should be handled in compaction
            TrimmedMessage::ProjectedTool(crate::message::ProjectedToolResult {
                entry_id: "entry-0".to_string(),
                tool_call_id: ToolCallId::new("tc-old"),
                tool_name: ToolName::new("read"),
                preview: "file content preview...".to_string(),
                artifact_id: "entry-0".to_string(),
                original_char_count: 500,
                is_error: false,
            }),
            // An OriginalTool — should be archived to A
            TrimmedMessage::OriginalTool(OriginalToolResult {
                entry_id: "entry-1".to_string(),
                tool_call_id: ToolCallId::new("tc1"),
                tool_name: ToolName::new("bash"),
                content: vec![Content::Text(TextContent {
                    text: "B".repeat(1000),
                })],
                is_error: false,
                turn: 1,
            }),
            TrimmedMessage::User(crate::message::UserMessage::new_text("C".repeat(1000))),
            TrimmedMessage::Assistant(AssistantMessage {
                content: vec![Content::Text(TextContent {
                    text: "D".repeat(1000),
                })],
                api: "test".into(),
                provider: "test".into(),
                model: "test".into(),
                stop_reason: StopReason::EndTurn,
                error_message: None,
                timestamp: 5,
                usage: Default::default(),
            }),
        ];

        let budget = ContextProjectionBudget {
            max_context_tokens: 500,
            compaction_threshold: 0.5,
            ..Default::default()
        };

        let plan = plan_compaction(&t, &budget);
        assert!(
            plan.is_some(),
            "should find a compaction plan with mixed entries"
        );
        let plan = plan.unwrap();
        // cut_index should not split tool pairs (ProjectedTool at index 1, OriginalTool at index 2)
        assert_ne!(plan.cut_index, 1, "should not cut at ProjectedTool");
        assert_ne!(plan.cut_index, 2, "should not cut at OriginalTool");

        let mut a = Artifacts::new();
        let result = apply_compaction(t, plan.clone(), "summary".to_string(), &mut a);

        // The OriginalTool should be archived if it was before the cut
        if plan.cut_index > 2 {
            assert!(
                a.contains_key("entry-1"),
                "OriginalTool should be archived to A"
            );
        }

        // First element should be Compaction
        assert!(matches!(result[0], TrimmedMessage::Compaction(_)));

        // Remaining entries after cut should be intact
        assert!(result.len() > 1, "should have entries after compaction");
    }

    #[test]
    fn build_summary_messages_with_all_variants() {
        use crate::message::ProjectedToolResult;

        let t = vec![
            TrimmedMessage::User(crate::message::UserMessage::new_text("hello")),
            TrimmedMessage::Assistant(AssistantMessage {
                content: vec![Content::Text(TextContent {
                    text: "let me check".to_string(),
                })],
                api: "test".into(),
                provider: "test".into(),
                model: "test".into(),
                stop_reason: StopReason::ToolUse,
                error_message: None,
                timestamp: 1,
                usage: Default::default(),
            }),
            TrimmedMessage::OriginalTool(OriginalToolResult {
                entry_id: "entry-0".to_string(),
                tool_call_id: ToolCallId::new("tc1"),
                tool_name: ToolName::new("bash"),
                content: vec![Content::Text(TextContent {
                    text: "full output here".to_string(),
                })],
                is_error: false,
                turn: 0,
            }),
            TrimmedMessage::ProjectedTool(ProjectedToolResult {
                entry_id: "entry-1".to_string(),
                tool_call_id: ToolCallId::new("tc2"),
                tool_name: ToolName::new("read"),
                preview: "truncated preview...".to_string(),
                artifact_id: "entry-1".to_string(),
                original_char_count: 100,
                is_error: false,
            }),
            TrimmedMessage::Compaction(crate::message::CompactionSummary {
                summary: "previous summary".to_string(),
                compacted_entry_ids: vec!["old-0".to_string()],
                tokens_before: 100,
            }),
        ];

        let plan = CompactionPlan {
            cut_index: t.len(),
            messages_to_summarize: t,
            tokens_to_free: 50,
        };

        let msgs = build_summary_messages(&plan);

        // User message
        assert!(
            msgs.iter().any(|m| matches!(m, AgentMessage::User(_))),
            "should have User message"
        );

        // OriginalTool → full ToolResult
        let tool_result = msgs.iter().find_map(|m| match m {
            AgentMessage::ToolResult(tr) if tr.tool_call_id.as_str() == "tc1" => Some(tr.clone()),
            _ => None,
        });
        assert!(
            tool_result.is_some(),
            "OriginalTool should become ToolResult"
        );
        let tool_text: String = tool_result
            .unwrap()
            .content
            .iter()
            .filter_map(|c| match c {
                Content::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            tool_text, "full output here",
            "OriginalTool should have full content"
        );

        // ProjectedTool → ToolResult with preview
        let projected_result = msgs.iter().find_map(|m| match m {
            AgentMessage::ToolResult(tr) if tr.tool_call_id.as_str() == "tc2" => Some(tr.clone()),
            _ => None,
        });
        assert!(
            projected_result.is_some(),
            "ProjectedTool should become ToolResult"
        );
        let projected_text: String = projected_result
            .unwrap()
            .content
            .iter()
            .filter_map(|c| match c {
                Content::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            projected_text.contains("truncated preview"),
            "ProjectedTool should have preview"
        );
    }
}
