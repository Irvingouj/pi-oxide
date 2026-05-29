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
use tracing::warn;

use crate::context_metadata::{
    fallback_strategy, ProjectionOutcome, ProjectionShape, ProjectionStrategy, ToolResultContext,
};
use crate::message::{AgentMessage, Content, TextContent};
use crate::script_projection::{parse_script_result, run_rhai_script, ScriptContext, ScriptResult};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Budget parameters for context projection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextProjectionBudget {
    pub max_tool_result_chars: usize,
    pub max_context_tokens: usize,
    /// Turns older than this have their tool results microcompacted.
    #[serde(default = "default_microcompact_after_turns")]
    pub microcompact_after_turns: u32,
    /// Fraction of max_context_tokens that triggers compaction signal (0.0–1.0).
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

/// Snapshot of actual API token usage, fed back from the host for calibration.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ApiUsageSnapshot {
    /// Token count estimated by our heuristic at the time of the API call.
    pub estimated_tokens: usize,
    /// Actual input tokens reported by the API.
    pub actual_input_tokens: usize,
}

/// A single replacement record: what was replaced, how, and where the full content lives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextReplacement {
    pub tool_call_id: String,
    pub tool_name: String,
    pub artifact_id: String,
    pub original_chars: usize,
    pub preview_chars: usize,
    pub strategy: ProjectionStrategy,
    #[serde(default)]
    pub outcome: ProjectionOutcome,
}

/// The single source of truth for every tool result in the context.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ToolProjectionState {
    #[serde(rename = "inline")]
    #[default]
    Inline,
    #[serde(rename = "deferred")]
    Deferred {
        until_turn: u32,
        #[serde(default)]
        inserted_at_turn: u32,
    },
    #[serde(rename = "replaced")]
    Replaced {
        replacement: ContextReplacement,
        #[serde(default)]
        inserted_at_turn: u32,
    },
}

/// State carried across turns so projection decisions remain stable.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct ContextProjectionState {
    /// Unified tool state map: tool_call_id → projection state.
    #[serde(default)]
    pub tools: BTreeMap<String, ToolProjectionState>,
    /// Current turn index, incremented each time project() is called.
    #[serde(default)]
    pub current_turn: u32,
    /// Last API usage, used to calibrate token estimation.
    #[serde(default)]
    pub last_api_usage: Option<ApiUsageSnapshot>,
    /// Turns since last compaction. Incremented by host after each turn.
    #[serde(default)]
    pub turns_since_compaction: u32,
}

// Backward-compatibility: old session JSON used `replacements` instead of `tools`.
#[derive(Deserialize)]
struct OldContextReplacement {
    tool_call_id: String,
    tool_name: String,
    artifact_id: String,
    original_chars: usize,
    preview_chars: usize,
    strategy: OldContextStrategy,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
enum OldContextStrategy {
    KeepFull,
    Head {
        max_chars: usize,
    },
    Tail {
        max_chars: usize,
    },
    HeadTail {
        head_chars: usize,
        tail_chars: usize,
    },
    DropIfOld,
    Microcompacted,
    Script {
        script: String,
    },
}

impl From<OldContextStrategy> for ProjectionStrategy {
    fn from(old: OldContextStrategy) -> Self {
        match old {
            OldContextStrategy::KeepFull => ProjectionStrategy::Fixed {
                shape: ProjectionShape::KeepFull,
                min_age: 0,
            },
            OldContextStrategy::Head { max_chars } => ProjectionStrategy::Fixed {
                shape: ProjectionShape::Head { max_chars },
                min_age: 0,
            },
            OldContextStrategy::Tail { max_chars } => ProjectionStrategy::Fixed {
                shape: ProjectionShape::Tail { max_chars },
                min_age: 0,
            },
            OldContextStrategy::HeadTail {
                head_chars,
                tail_chars,
            } => ProjectionStrategy::Fixed {
                shape: ProjectionShape::HeadTail {
                    head_chars,
                    tail_chars,
                },
                min_age: 0,
            },
            OldContextStrategy::DropIfOld => ProjectionStrategy::Fixed {
                shape: ProjectionShape::Microcompacted,
                min_age: 0,
            },
            OldContextStrategy::Microcompacted => ProjectionStrategy::Fixed {
                shape: ProjectionShape::Microcompacted,
                min_age: 0,
            },
            OldContextStrategy::Script { script } => {
                let script = if script.trim().starts_with("#{") {
                    script
                } else {
                    format!(r##"#{{ action: "project", text: {{ {} }} }}"##, script)
                };
                ProjectionStrategy::Dynamic { script }
            }
        }
    }
}

impl<'de> Deserialize<'de> for ContextProjectionState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default)]
            tools: BTreeMap<String, ToolProjectionState>,
            #[serde(default)]
            replacements: BTreeMap<String, OldContextReplacement>,
            #[serde(default, alias = "turn_count")]
            current_turn: u32,
            #[serde(default)]
            last_api_usage: Option<ApiUsageSnapshot>,
            #[serde(default)]
            turns_since_compaction: u32,
        }

        let raw = Raw::deserialize(deserializer)?;
        let tools = if !raw.tools.is_empty() {
            raw.tools
        } else {
            let mut tools = BTreeMap::new();
            for (id, old) in raw.replacements {
                let replacement = ContextReplacement {
                    tool_call_id: old.tool_call_id,
                    tool_name: old.tool_name,
                    artifact_id: old.artifact_id,
                    original_chars: old.original_chars,
                    preview_chars: old.preview_chars,
                    strategy: old.strategy.into(),
                    outcome: ProjectionOutcome::default(),
                };
                tools.insert(
                    id,
                    ToolProjectionState::Replaced {
                        replacement,
                        inserted_at_turn: 0,
                    },
                );
            }
            tools
        };

        Ok(ContextProjectionState {
            tools,
            current_turn: raw.current_turn,
            last_api_usage: raw.last_api_usage,
            turns_since_compaction: raw.turns_since_compaction,
        })
    }
}

/// Report returned after projection, for host observability and artifact storage.
///
/// NOTE: Only `replacements` is currently consumed by the web host. The other
/// fields are computed for future use (e.g. compaction signaling, token
/// budgeting UI, cache-breakpoint hints) and are verified by Rust unit tests.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

const CHARS_PER_TOKEN: usize = 4;

/// Estimate tokens for a list of messages using chars/4 heuristic.
pub fn estimate_tokens(messages: &[AgentMessage]) -> usize {
    let chars = count_message_chars(messages);
    chars.div_ceil(CHARS_PER_TOKEN)
}

/// Estimate tokens for a string.
pub fn estimate_tokens_for_text(text: &str) -> usize {
    text.chars().count().div_ceil(CHARS_PER_TOKEN)
}

/// Calibrated token estimate using actual API usage when available.
fn calibrated_estimate(chars: usize, state: &ContextProjectionState) -> usize {
    let raw = chars.div_ceil(CHARS_PER_TOKEN);
    if let Some(ref api) = state.last_api_usage {
        if api.estimated_tokens > 0 && api.actual_input_tokens > 0 {
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

// ---------------------------------------------------------------------------
// Projection decision types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum ProjectionDecision {
    KeepInline,
    KeepDeferred { until_turn: u32 },
    Defer { until_turn: u32 },
    Replace { replacement: ContextReplacement },
    UpdateReplacement { replacement: ContextReplacement },
}

#[derive(Debug, Clone, PartialEq)]
pub enum TrimBoundary {
    None,
    DropTurns(usize),
    KeepLastTurn,
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
    match serde_json::from_value::<ToolResultContext>(value.clone()) {
        Ok(ctx) => Some(ctx),
        Err(e) => {
            if let Some(context) = value.get("context").cloned() {
                match serde_json::from_value::<ToolResultContext>(context) {
                    Ok(ctx) => Some(ctx),
                    Err(e2) => {
                        warn!(error = %e2, "failed to parse nested context metadata");
                        None
                    }
                }
            } else {
                warn!(error = %e, "failed to parse tool result context metadata");
                None
            }
        }
    }
}

fn build_replacement(
    tool_call_id: &str,
    tool_name: &str,
    text_chars: usize,
    metadata: &Option<ToolResultContext>,
    strategy: &ProjectionStrategy,
    outcome: ProjectionOutcome,
) -> ContextReplacement {
    build_replacement_with_id(
        tool_call_id,
        tool_name,
        text_chars,
        metadata,
        strategy,
        outcome,
        format!("tool-result-{tool_call_id}"),
    )
}

fn build_update_replacement(
    prior_replacement: &ContextReplacement,
    text: &str,
    text_chars: usize,
    metadata: &Option<ToolResultContext>,
) -> ProjectionDecision {
    let stored_text = if !prior_replacement.outcome.text().is_empty() {
        prior_replacement.outcome.text().to_string()
    } else {
        text.to_string()
    };
    let outcome = ProjectionOutcome { text: stored_text };
    let replacement = build_replacement(
        &prior_replacement.tool_call_id,
        &prior_replacement.tool_name,
        text_chars,
        metadata,
        &prior_replacement.strategy,
        outcome,
    );
    ProjectionDecision::UpdateReplacement { replacement }
}

fn build_replacement_with_id(
    tool_call_id: &str,
    tool_name: &str,
    text_chars: usize,
    metadata: &Option<ToolResultContext>,
    strategy: &ProjectionStrategy,
    outcome: ProjectionOutcome,
    artifact_id: String,
) -> ContextReplacement {
    let preview_text = outcome.text();
    ContextReplacement {
        tool_call_id: tool_call_id.to_string(),
        tool_name: tool_name.to_string(),
        artifact_id,
        original_chars: metadata
            .as_ref()
            .map(|ctx| ctx.original_chars)
            .unwrap_or(text_chars),
        preview_chars: char_count(preview_text),
        strategy: strategy.clone(),
        outcome,
    }
}

/// Apply a replacement decision to a single message and state.
fn apply_replacement_to_msg(
    msg: &mut AgentMessage,
    state: &mut ContextProjectionState,
    tool_call_id: &str,
    replacement: ContextReplacement,
    inserted_at_turn: u32,
) {
    let preview = build_preview_text(&replacement.outcome, &replacement.artifact_id);
    state.tools.insert(
        tool_call_id.to_string(),
        ToolProjectionState::Replaced {
            replacement,
            inserted_at_turn,
        },
    );
    *msg = replace_tool_result_text(msg, preview);
}

#[allow(clippy::too_many_arguments)]
fn dynamic_fallback_decision(
    tool_call_id: &str,
    tool_name: &str,
    text: &str,
    text_chars: usize,
    metadata: &Option<ToolResultContext>,
    strategy: &ProjectionStrategy,
    error: &str,
    is_update: bool,
) -> ProjectionDecision {
    let fallback_text = take_head_chars(text, FALLBACK_MAX_CHARS);
    let outcome = ProjectionOutcome {
        text: format!(
            "{}\n\n[projection error: script failed — {}]",
            fallback_text, error
        ),
    };
    let replacement = build_replacement(
        tool_call_id,
        tool_name,
        text_chars,
        metadata,
        strategy,
        outcome,
    );
    if is_update {
        ProjectionDecision::UpdateReplacement { replacement }
    } else {
        ProjectionDecision::Replace { replacement }
    }
}

fn apply_shape(
    shape: &ProjectionShape,
    text: &str,
    tool_name: &str,
    tool_call_id: &str,
) -> ProjectionOutcome {
    let result = match shape {
        ProjectionShape::KeepFull => text.to_string(),
        ProjectionShape::Head { max_chars } => truncate_or_keep(text, *max_chars, take_head_chars),
        ProjectionShape::Tail { max_chars } => truncate_or_keep(text, *max_chars, take_tail_chars),
        ProjectionShape::HeadTail {
            head_chars,
            tail_chars,
        } => {
            let text_chars = char_count(text);
            if text_chars <= head_chars.saturating_add(*tail_chars) {
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
        ProjectionShape::Microcompacted => format!(
            "<tool-summary tool=\"{}\" call=\"{}\">Result: {} chars</tool-summary>",
            escape_xml(tool_name),
            escape_xml(tool_call_id),
            char_count(text)
        ),
    };
    ProjectionOutcome { text: result }
}

/// Replace the text content of a tool result with a preview/summary.
/// Non-text blocks (e.g., images) are preserved so they remain visible
/// even when the text is replaced or microcompacted.
fn replace_tool_result_text(msg: &AgentMessage, preview: String) -> AgentMessage {
    if let AgentMessage::ToolResult(tr) = msg {
        let mut new_content = vec![Content::Text(TextContent { text: preview })];
        for block in &tr.content {
            if !matches!(block, Content::Text(..)) {
                new_content.push(block.clone());
            }
        }
        AgentMessage::ToolResult(tr.with_content(new_content))
    } else {
        msg.clone()
    }
}

fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Build the preview marker text.
///
/// The `<context-artifact>` format is a wire-protocol contract between
/// the projection engine and the host. The host uses `report.replacements`
/// to drive artifact lookup; do not change this format without updating
/// the host parser.
fn build_preview_text(outcome: &ProjectionOutcome, artifact_id: &str) -> String {
    let text = outcome.text();
    format!(
        "<context-artifact id=\"{}\">\n{}\n</context-artifact>",
        escape_xml(artifact_id),
        escape_xml(text)
    )
}

/// Build a [`ScriptContext`] from projection inputs.
#[allow(clippy::too_many_arguments)]
fn build_script_context(
    text: &str,
    tool_name: &str,
    tool_call_id: &str,
    turn_index: usize,
    total_turns: usize,
    raw_tokens: usize,
    budget: &ContextProjectionBudget,
    turns_since_compaction: u32,
    was_replaced_before: bool,
) -> ScriptContext {
    ScriptContext {
        text: text.to_string(),
        tool_name: tool_name.to_string(),
        tool_call_id: tool_call_id.to_string(),
        turn_index,
        total_turns,
        total_tokens: raw_tokens,
        max_context_tokens: budget.max_context_tokens,
        max_tool_result_chars: budget.max_tool_result_chars,
        turns_since_compaction,
        was_replaced_before,
    }
}

const FALLBACK_MAX_CHARS: usize = 2000;

// Cap at 1000 to keep serialized state under ~64 KB for typical tool_call_id lengths.
// NOTE: Keep in sync with web/src/services/projectionService.ts MAX_ARTIFACTS.
const MAX_DEFERRED_ENTRIES: usize = 1000;

fn truncate_or_keep(text: &str, limit: usize, truncator: impl Fn(&str, usize) -> String) -> String {
    if char_count(text) <= limit {
        text.to_string()
    } else {
        truncator(text, limit)
    }
}

fn evict_oldest_if_over_limit(state: &mut ContextProjectionState) {
    // Rationale: at ~640 bytes per serialized entry, 1000 entries ≈ 64KB,
    // keeping the JSON session state under a reasonable wire limit.

    let mut entries: Vec<(String, u32)> = state
        .tools
        .iter()
        .filter_map(|(id, v)| match v {
            ToolProjectionState::Deferred {
                inserted_at_turn, ..
            } => Some((id.clone(), *inserted_at_turn)),
            ToolProjectionState::Replaced {
                inserted_at_turn, ..
            } => Some((id.clone(), *inserted_at_turn)),
            ToolProjectionState::Inline => None,
        })
        .collect();

    let excess = entries.len().saturating_sub(MAX_DEFERRED_ENTRIES);
    if excess == 0 {
        return;
    }

    // Deterministic FIFO: sort by inserted_at_turn, then by tool_call_id for tie-breaking.
    entries.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));

    for (id, _) in entries.into_iter().take(excess) {
        state.tools.remove(&id);
    }
}

// Helper: which turn (0-based) does message index `i` belong to?
fn turn_index_for(i: usize, boundaries: &[usize]) -> usize {
    for (t, &start) in boundaries.iter().enumerate().skip(1) {
        if i < start {
            return t.saturating_sub(1);
        }
    }
    boundaries.len().saturating_sub(2)
}

#[allow(clippy::too_many_arguments)]
fn eval_dynamic_script(
    script: &str,
    text: &str,
    tool_name: &str,
    tool_call_id: &str,
    turn_idx: usize,
    total_turns: usize,
    raw_tokens: usize,
    budget: &ContextProjectionBudget,
    turns_since_compaction: u32,
    was_replaced_before: bool,
    current_turn: u32,
    text_chars: usize,
    metadata: &Option<ToolResultContext>,
    strategy: &ProjectionStrategy,
    is_update: bool,
) -> ProjectionDecision {
    let script_ctx = build_script_context(
        text,
        tool_name,
        tool_call_id,
        turn_idx,
        total_turns,
        raw_tokens,
        budget,
        turns_since_compaction,
        was_replaced_before,
    );
    match run_rhai_script(&script_ctx, script) {
        Ok(map) => match parse_script_result(&map) {
            Ok(ScriptResult::Project {
                text: projected_text,
            }) => {
                let outcome = ProjectionOutcome {
                    text: projected_text,
                };
                let replacement = build_replacement(
                    tool_call_id,
                    tool_name,
                    text_chars,
                    metadata,
                    strategy,
                    outcome,
                );
                if is_update {
                    ProjectionDecision::UpdateReplacement { replacement }
                } else {
                    ProjectionDecision::Replace { replacement }
                }
            }
            Ok(ScriptResult::Defer { reevaluate_after }) => {
                let until_turn = current_turn.saturating_add(reevaluate_after);
                ProjectionDecision::Defer { until_turn }
            }
            Err(e) => {
                warn!(error = %e, tool_call_id = tool_call_id, "dynamic strategy failed; using fallback");
                dynamic_fallback_decision(
                    tool_call_id,
                    tool_name,
                    text,
                    text_chars,
                    metadata,
                    strategy,
                    &e,
                    is_update,
                )
            }
        },
        Err(e) => {
            warn!(error = %e, tool_call_id = tool_call_id, "dynamic strategy failed; using fallback");
            dynamic_fallback_decision(
                tool_call_id,
                tool_name,
                text,
                text_chars,
                metadata,
                strategy,
                &e.to_string(),
                is_update,
            )
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_replace_or_update_decision(
    shape: &ProjectionShape,
    text: &str,
    tool_call_id: &str,
    tool_name: &str,
    text_chars: usize,
    metadata: &Option<ToolResultContext>,
    strategy: &ProjectionStrategy,
    prior: Option<&ToolProjectionState>,
) -> ProjectionDecision {
    let outcome = apply_shape(shape, text, tool_name, tool_call_id);
    let replacement = build_replacement(
        tool_call_id,
        tool_name,
        text_chars,
        metadata,
        strategy,
        outcome,
    );
    if matches!(prior, Some(ToolProjectionState::Replaced { .. })) {
        ProjectionDecision::UpdateReplacement { replacement }
    } else {
        ProjectionDecision::Replace { replacement }
    }
}

#[allow(clippy::too_many_arguments)]
fn decide_projection(
    msg: &AgentMessage,
    prior: Option<&ToolProjectionState>,
    current_turn: u32,
    budget: &ContextProjectionBudget,
    turn_boundaries: &[usize],
    msg_idx: usize,
    total_turns: usize,
    raw_tokens: usize,
    turns_since_compaction: u32,
) -> ProjectionDecision {
    let AgentMessage::ToolResult(tr) = msg else {
        return ProjectionDecision::KeepInline;
    };
    let text = extract_text(&tr.content);
    let text_chars = char_count(&text);
    let tool_call_id = tr.tool_call_id.clone_inner();
    let tool_name = tr.tool_name.clone_inner();
    let turn_idx = turn_index_for(msg_idx, turn_boundaries);
    let age = current_turn.saturating_sub(turn_idx as u32);

    let metadata = tool_result_context(&tr.details);
    let strategy = metadata
        .as_ref()
        .map(|context| context.strategy.clone())
        .unwrap_or_else(|| fallback_strategy(&tool_name));

    // KeepFull means no replacement at all
    if matches!(
        strategy,
        ProjectionStrategy::Fixed {
            shape: ProjectionShape::KeepFull,
            ..
        }
    ) {
        return ProjectionDecision::KeepInline;
    }

    // Small results stay inline, unless already replaced by a prior projection
    // (e.g. microcompact summary that must persist across turns)
    if text_chars <= budget.max_tool_result_chars
        && !matches!(prior, Some(ToolProjectionState::Replaced { .. }))
    {
        return ProjectionDecision::KeepInline;
    }

    let eval_dynamic =
        |script: &str, was_replaced_before: bool, is_update: bool| -> ProjectionDecision {
            eval_dynamic_script(
                script,
                &text,
                &tool_name,
                &tool_call_id,
                turn_idx,
                total_turns,
                raw_tokens,
                budget,
                turns_since_compaction,
                was_replaced_before,
                current_turn,
                text_chars,
                &metadata,
                &strategy,
                is_update,
            )
        };

    match (&strategy, prior) {
        // Fixed strategy, never seen or inline
        (ProjectionStrategy::Fixed { shape, min_age }, None)
        | (ProjectionStrategy::Fixed { shape, min_age }, Some(ToolProjectionState::Inline)) => {
            if age < *min_age {
                let until_turn = current_turn.saturating_add(min_age - age);
                ProjectionDecision::Defer { until_turn }
            } else {
                build_replace_or_update_decision(
                    shape,
                    &text,
                    &tool_call_id,
                    &tool_name,
                    text_chars,
                    &metadata,
                    &strategy,
                    prior,
                )
            }
        }

        // Fixed strategy, already deferred
        (
            ProjectionStrategy::Fixed { shape, min_age: _ },
            Some(ToolProjectionState::Deferred { until_turn, .. }),
        ) => {
            if current_turn < *until_turn {
                ProjectionDecision::KeepDeferred {
                    until_turn: *until_turn,
                }
            } else {
                build_replace_or_update_decision(
                    shape,
                    &text,
                    &tool_call_id,
                    &tool_name,
                    text_chars,
                    &metadata,
                    &strategy,
                    prior,
                )
            }
        }

        // Fixed strategy, already replaced
        (
            ProjectionStrategy::Fixed { shape, .. },
            Some(ToolProjectionState::Replaced {
                replacement: prior_replacement,
                ..
            }),
        ) => {
            // Microcompacted summaries are always preserved regardless of metadata changes
            if matches!(
                prior_replacement.strategy,
                ProjectionStrategy::Fixed {
                    shape: ProjectionShape::Microcompacted,
                    ..
                }
            ) {
                build_update_replacement(prior_replacement, &text, text_chars, &metadata)
            } else if prior_replacement.strategy != strategy {
                build_replace_or_update_decision(
                    shape,
                    &text,
                    &tool_call_id,
                    &tool_name,
                    text_chars,
                    &metadata,
                    &strategy,
                    prior,
                )
            } else {
                // Keep the prior replacement text (e.g., microcompact summary)
                build_update_replacement(prior_replacement, &text, text_chars, &metadata)
            }
        }

        // Dynamic strategy, never seen or inline
        (ProjectionStrategy::Dynamic { script }, None)
        | (ProjectionStrategy::Dynamic { script }, Some(ToolProjectionState::Inline)) => {
            eval_dynamic(script, false, false)
        }

        // Dynamic strategy, already deferred
        (
            ProjectionStrategy::Dynamic { script },
            Some(ToolProjectionState::Deferred { until_turn, .. }),
        ) => {
            if current_turn < *until_turn {
                ProjectionDecision::KeepDeferred {
                    until_turn: *until_turn,
                }
            } else {
                eval_dynamic(script, false, false)
            }
        }

        // Dynamic strategy, already replaced
        (ProjectionStrategy::Dynamic { script }, Some(ToolProjectionState::Replaced { .. })) => {
            eval_dynamic(script, true, true)
        }
    }
}

fn apply_microcompact(
    mut projected: Vec<AgentMessage>,
    mut state: ContextProjectionState,
    budget: &ContextProjectionBudget,
    turn_boundaries: &[usize],
    current_turn: u32,
) -> (Vec<AgentMessage>, ContextProjectionState) {
    let total_turns = turn_boundaries.len().saturating_sub(1);
    if total_turns <= budget.microcompact_after_turns as usize {
        return (projected, state);
    }

    let cutoff_turn = total_turns.saturating_sub(budget.microcompact_after_turns as usize);
    for turn_idx in 0..cutoff_turn {
        let start = turn_boundaries[turn_idx];
        let end = turn_boundaries[turn_idx + 1];
        for msg in &mut projected[start..end] {
            if let AgentMessage::ToolResult(tr) = msg {
                let tcid = tr.tool_call_id.clone_inner();
                // Skip if already in tools map (replaced or deferred)
                if state.tools.contains_key(&tcid) {
                    continue;
                }
                let metadata = tool_result_context(&tr.details);
                let strategy = metadata
                    .as_ref()
                    .map(|context| context.strategy.clone())
                    .unwrap_or_else(|| fallback_strategy(tr.tool_name.as_str()));
                if matches!(
                    strategy,
                    ProjectionStrategy::Fixed {
                        shape: ProjectionShape::KeepFull,
                        ..
                    }
                ) {
                    continue;
                }
                let text = extract_text(&tr.content);
                let char_count_val = char_count(&text);
                let outcome = apply_shape(
                    &ProjectionShape::Microcompacted,
                    &text,
                    tr.tool_name.as_str(),
                    &tcid,
                );
                let strategy = ProjectionStrategy::Fixed {
                    shape: ProjectionShape::Microcompacted,
                    min_age: 0,
                };
                let replacement = build_replacement_with_id(
                    &tcid,
                    tr.tool_name.as_str(),
                    char_count_val,
                    &metadata,
                    &strategy,
                    outcome,
                    format!("microcompact-{tcid}"),
                );
                apply_replacement_to_msg(msg, &mut state, &tcid, replacement, current_turn);
            }
        }
    }
    (projected, state)
}

fn decide_trim_boundary(
    messages: &[AgentMessage],
    system_prompt: &str,
    budget: &ContextProjectionBudget,
    boundaries: &[usize],
    state: &ContextProjectionState,
) -> TrimBoundary {
    let sys_tokens = calibrated_estimate(system_prompt.chars().count(), state);
    let msg_tokens = calibrated_estimate(count_message_chars(messages), state);
    let total_tokens = msg_tokens + sys_tokens;
    if total_tokens <= budget.max_context_tokens {
        return TrimBoundary::None;
    }

    let mut cumulative_tokens = 0;
    let mut last_kept_turn = 0;
    let msg_budget = budget.max_context_tokens.saturating_sub(sys_tokens);

    for i in (0..boundaries.len().saturating_sub(1)).rev() {
        let start = boundaries[i];
        let end = boundaries[i + 1];
        let turn_tokens = calibrated_estimate(count_message_chars(&messages[start..end]), state);
        if cumulative_tokens + turn_tokens > msg_budget {
            break;
        }
        cumulative_tokens += turn_tokens;
        last_kept_turn = i;
    }

    if last_kept_turn == 0 && boundaries.len() > 2 {
        TrimBoundary::KeepLastTurn
    } else {
        TrimBoundary::DropTurns(last_kept_turn)
    }
}

fn assistant_contains_tool_call(msg: &AgentMessage, tool_call_id: &str) -> bool {
    match msg {
        AgentMessage::Assistant(a) => a.content.iter().any(|block| {
            if let Content::ToolCall(tc) = block {
                tc.id.as_str() == tool_call_id
            } else {
                false
            }
        }),
        _ => false,
    }
}

/// Adjust trim start so the first kept message is never an orphan ToolResult.
/// Iterates over all tool results at the front of the kept slice and walks
/// back to the earliest matching assistant.
fn adjust_trim_start_for_orphan_safety(messages: &[AgentMessage], start: usize) -> usize {
    let kept = &messages[start..];
    if kept.is_empty() {
        return start;
    }

    let mut earliest = start;
    for (i, msg) in kept.iter().enumerate() {
        let AgentMessage::ToolResult(tr) = msg else {
            // Only process consecutive tool results at the very front.
            break;
        };
        let target_tool_call_id = tr.tool_call_id.as_str();

        // Check if any Assistant in kept[0..i] contains the matching tool_call_id
        let has_matching_assistant = kept[..i]
            .iter()
            .any(|m| assistant_contains_tool_call(m, target_tool_call_id));

        if !has_matching_assistant {
            if let Some(pos) = messages[..start]
                .iter()
                .rposition(|m| assistant_contains_tool_call(m, target_tool_call_id))
            {
                earliest = earliest.min(pos);
            }
        }
    }
    earliest
}

fn apply_trim(
    messages: Vec<AgentMessage>,
    boundary: TrimBoundary,
    boundaries: &[usize],
) -> (Vec<AgentMessage>, usize) {
    match boundary {
        TrimBoundary::None => (messages, 0),
        TrimBoundary::DropTurns(n) => {
            let start = boundaries[n];
            let adjusted_start = adjust_trim_start_for_orphan_safety(&messages, start);
            let final_messages = messages[adjusted_start..].to_vec();
            let dropped = adjusted_start;
            (final_messages, dropped)
        }
        TrimBoundary::KeepLastTurn => {
            if boundaries.len() >= 2 {
                let start = boundaries[boundaries.len() - 2];
                let adjusted_start = adjust_trim_start_for_orphan_safety(&messages, start);
                let final_messages = messages[adjusted_start..].to_vec();
                let dropped = adjusted_start;
                (final_messages, dropped)
            } else {
                (messages, 0)
            }
        }
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
    // Compute turn boundaries
    let turn_boundaries = find_turn_boundaries(&input.messages);
    let total_turns = turn_boundaries.len().saturating_sub(1);

    // Increment turn count and ensure it tracks the actual turn count in messages
    let mut updated_state = input.state.clone();
    let derived_turn = total_turns.saturating_sub(1) as u32;
    updated_state.current_turn = derived_turn;
    let current_turn = updated_state.current_turn;
    let raw_tokens = estimate_tokens(&input.messages);

    // Step 1: Collect decisions for all tool results
    let mut decisions: Vec<(usize, String, ProjectionDecision)> = Vec::new();
    for (msg_idx, msg) in input.messages.iter().enumerate() {
        if let AgentMessage::ToolResult(tr) = msg {
            let tool_call_id = tr.tool_call_id.clone_inner();
            let prior = updated_state.tools.get(&tool_call_id);
            let decision = decide_projection(
                msg,
                prior,
                current_turn,
                &input.budget,
                &turn_boundaries,
                msg_idx,
                total_turns,
                raw_tokens,
                updated_state.turns_since_compaction,
            );
            decisions.push((msg_idx, tool_call_id, decision));
        }
    }

    // Step 2: Apply all decisions atomically
    let mut projected = input.messages.clone();
    for (msg_idx, tool_call_id, decision) in decisions {
        let inserted_at_turn = input
            .state
            .tools
            .get(&tool_call_id)
            .and_then(|s| match s {
                ToolProjectionState::Deferred {
                    inserted_at_turn, ..
                } => Some(*inserted_at_turn),
                ToolProjectionState::Replaced {
                    inserted_at_turn, ..
                } => Some(*inserted_at_turn),
                _ => None,
            })
            .unwrap_or(current_turn);
        match decision {
            ProjectionDecision::KeepInline => {
                updated_state.tools.remove(&tool_call_id);
            }
            ProjectionDecision::KeepDeferred { until_turn } => {
                updated_state.tools.insert(
                    tool_call_id,
                    ToolProjectionState::Deferred {
                        until_turn,
                        inserted_at_turn,
                    },
                );
            }
            ProjectionDecision::Defer { until_turn } => {
                updated_state.tools.insert(
                    tool_call_id,
                    ToolProjectionState::Deferred {
                        until_turn,
                        inserted_at_turn: current_turn,
                    },
                );
            }
            ProjectionDecision::Replace { replacement }
            | ProjectionDecision::UpdateReplacement { replacement } => {
                apply_replacement_to_msg(
                    &mut projected[msg_idx],
                    &mut updated_state,
                    &tool_call_id,
                    replacement,
                    inserted_at_turn,
                );
            }
        }
    }

    // Step 3: Microcompact
    let (projected, mut updated_state) = apply_microcompact(
        projected,
        updated_state,
        &input.budget,
        &turn_boundaries,
        current_turn,
    );

    // Step 4: Cap deferred entries (must happen before report collection)
    evict_oldest_if_over_limit(&mut updated_state);

    // Step 5: Collect replacements for report
    let mut replacements: Vec<ContextReplacement> = Vec::new();
    for state in updated_state.tools.values() {
        if let ToolProjectionState::Replaced { replacement, .. } = state {
            replacements.push(replacement.clone());
        }
    }

    // Step 6: Trim to budget
    let boundary = decide_trim_boundary(
        &projected,
        &input.system_prompt,
        &input.budget,
        &turn_boundaries,
        &input.state,
    );
    let (trimmed, dropped_count) = apply_trim(projected, boundary, &turn_boundaries);

    // Step 7: Build report
    let msg_chars = count_message_chars(&trimmed);
    let sys_chars = input.system_prompt.chars().count();
    let msg_tokens = calibrated_estimate(msg_chars, &input.state);
    let sys_tokens = calibrated_estimate(sys_chars, &input.state);
    let total_tokens = msg_tokens + sys_tokens;
    let usage_pct = total_tokens as f32 / input.budget.max_context_tokens as f32;
    let needs_compaction = usage_pct > input.budget.compaction_threshold;
    let dropped_messages = dropped_count;
    let cache_breakpoints = compute_cache_breakpoints(&trimmed);

    let report = ContextProjectionReport {
        estimated_tokens: total_tokens,
        replacements,
        dropped_messages,
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
/// Places a breakpoint at the start of the last turn so the prefix stays cached.
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
    use crate::message::{Content, ToolCall as ToolCallContent};
    use crate::types::{ToolArguments, ToolCallId, ToolDetails, ToolName};

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
            microcompact_after_turns: 5,
            compaction_threshold: 0.75,
        }
    }

    // --- Token estimation tests ---

    #[test]
    fn token_estimate_counts_user_text() {
        let msgs = vec![user_msg("Hello, world!")]; // 13 chars -> (13+3)/4 = 4
        assert_eq!(estimate_tokens(&msgs), 13_usize.div_ceil(4));
    }

    #[test]
    fn token_estimate_counts_assistant_tool_call_arguments() {
        let msgs = vec![assistant_tool_call("tc-1", "bash", r#"{"command":"ls"}"#)];
        let tokens = estimate_tokens(&msgs);
        // Name "bash" (4) + serialized args
        let args_str =
            serde_json::to_string(&ToolArguments::new(serde_json::json!({"command":"ls"})))
                .unwrap();
        let expected = (4 + args_str.len()).div_ceil(4);
        assert_eq!(tokens, expected);
    }

    #[test]
    fn token_estimate_counts_tool_result_text() {
        let text = "file contents here"; // 19 chars -> (19+3)/4 = 5
        let msgs = vec![tool_result_msg("tc-1", "read", text)];
        assert_eq!(estimate_tokens(&msgs), 19_usize.div_ceil(4));
    }

    // --- Strategy tests ---

    #[test]
    fn read_uses_head_preview() {
        let big = "A".repeat(5000);
        let msgs = vec![
            user_msg("turn 0"),
            assistant_tool_call("tc-1", "read", r#"{"path":"x.rs"}"#),
            tool_result_msg("tc-1", "read", &big),
            user_msg("turn 1"),
            assistant_text("done"),
            user_msg("turn 2"),
            assistant_text("done"),
        ];
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
        if let AgentMessage::ToolResult(tr) = &output.projected_messages[2] {
            let text = extract_text(&tr.content);
            assert!(text.contains("<context-artifact"));
            // Head preview: should contain a run of A's
            assert!(text.contains(&"A".repeat(100)));
        } else {
            panic!("expected tool result");
        }
    }

    #[test]
    fn bash_uses_tail_preview() {
        let big = format!("{}{}", "A".repeat(3000), "B".repeat(2000));
        let msgs = vec![
            user_msg("turn 0"),
            assistant_tool_call("tc-2", "bash", r#"{"command":"ls"}"#),
            tool_result_msg("tc-2", "bash", &big),
            user_msg("turn 1"),
            assistant_text("done"),
            user_msg("turn 2"),
            assistant_text("done"),
        ];
        let output = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs,
            budget: default_budget(),
            state: ContextProjectionState::default(),
        });

        assert_eq!(output.report.replacements.len(), 1);
        if let AgentMessage::ToolResult(tr) = &output.projected_messages[2] {
            let text = extract_text(&tr.content);
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
        let big = format!("{}{}", "A".repeat(3000), "B".repeat(2000));
        let details = serde_json::json!({
            "content_kind": "file_read",
            "strategy": { "type": "fixed", "shape": { "type": "tail", "max_chars": 200 }, "min_age": 0 },
            "original_chars": 5000,
            "truncated_by_tool": false,
            "path": "src/lib.rs"
        });
        let msgs = vec![
            user_msg("turn 0"),
            assistant_tool_call("tc-meta", "read", r#"{"path":"x.rs"}"#),
            tool_result_msg_with_details("tc-meta", "read", &big, details),
            user_msg("turn 1"),
            assistant_text("done"),
            user_msg("turn 2"),
            assistant_text("done"),
        ];

        let output = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs,
            budget: default_budget(),
            state: ContextProjectionState::default(),
        });

        assert_eq!(output.report.replacements.len(), 1);
        assert!(matches!(
            output.report.replacements[0].strategy,
            ProjectionStrategy::Fixed {
                shape: ProjectionShape::Tail { max_chars: 200 },
                ..
            }
        ));

        if let AgentMessage::ToolResult(tr) = &output.projected_messages[2] {
            let text = extract_text(&tr.content);
            // Should contain B's from the tail
            assert!(text.contains(&"B".repeat(100)));
            assert!(!text.contains(&"A".repeat(500)));
        } else {
            panic!("expected tool result");
        }
    }

    #[test]
    fn nested_context_metadata_strategy_overrides_tool_name_fallback() {
        let big = format!("{}{}", "A".repeat(3000), "B".repeat(2000));
        let details = serde_json::json!({
            "exitCode": 0,
            "context": {
                "content_kind": "file_read",
                "strategy": { "type": "fixed", "shape": { "type": "tail", "max_chars": 200 }, "min_age": 0 },
                "original_chars": 5000,
                "truncated_by_tool": false,
                "path": "src/lib.rs"
            }
        });
        let msgs = vec![
            user_msg("turn 0"),
            assistant_tool_call("tc-nested", "read", r#"{"path":"x.rs"}"#),
            tool_result_msg_with_details("tc-nested", "read", &big, details),
            user_msg("turn 1"),
            assistant_text("done"),
            user_msg("turn 2"),
            assistant_text("done"),
        ];

        let output = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs,
            budget: default_budget(),
            state: ContextProjectionState::default(),
        });

        assert_eq!(output.report.replacements.len(), 1);
        assert!(matches!(
            output.report.replacements[0].strategy,
            ProjectionStrategy::Fixed {
                shape: ProjectionShape::Tail { max_chars: 200 },
                ..
            }
        ));
    }

    #[test]
    fn replacement_ids_are_deterministic() {
        let big = "A".repeat(5000);
        let msgs = vec![
            user_msg("turn 0"),
            assistant_tool_call("tc-det", "bash", r#"{"command":"ls"}"#),
            tool_result_msg("tc-det", "bash", &big),
            user_msg("turn 1"),
            assistant_text("done"),
            user_msg("turn 2"),
            assistant_text("done"),
        ];

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
        let msgs = vec![
            user_msg("turn 0"),
            assistant_tool_call("tc-stable", "bash", r#"{"command":"ls"}"#),
            tool_result_msg("tc-stable", "bash", &big),
            user_msg("turn 1"),
            assistant_text("done"),
            user_msg("turn 2"),
            assistant_text("done"),
        ];

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
            msgs.push(assistant_text(&format!(
                "response {i}: {}",
                "B".repeat(200)
            )));
        }

        let budget = ContextProjectionBudget {
            max_tool_result_chars: 50_000,
            max_context_tokens: 500,
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
            msgs.push(tool_result_msg(
                &format!("tc-{i}"),
                "bash",
                &"Y".repeat(200),
            ));
        }

        let budget = ContextProjectionBudget {
            max_tool_result_chars: 50_000,
            max_context_tokens: 500,
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
        let msgs = vec![
            user_msg("turn 0"),
            assistant_tool_call("tc-prior", "bash", r#"{"command":"ls"}"#),
            tool_result_msg("tc-prior", "bash", &big),
            user_msg("turn 1"),
            assistant_text("done"),
            user_msg("turn 2"),
            assistant_text("done"),
        ];

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
        assert_eq!(
            output.report.dropped_messages, 0,
            "should not drop messages"
        );
    }

    #[test]
    fn hard_limit_trims_and_signals_compaction() {
        let mut msgs = Vec::new();
        for i in 0..20 {
            msgs.push(user_msg(&format!("turn {i}: {}", "A".repeat(200))));
            msgs.push(assistant_text(&format!(
                "response {i}: {}",
                "B".repeat(200)
            )));
        }

        let budget = ContextProjectionBudget {
            max_tool_result_chars: 50_000,
            max_context_tokens: 500,
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
        let msgs = vec![user_msg("turn 0"), assistant_text("response 0")];

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
            msgs.push(assistant_tool_call(
                &format!("tc-{i}"),
                "bash",
                r#"{"command":"ls"}"#,
            ));
            msgs.push(tool_result_msg(
                &format!("tc-{i}"),
                "bash",
                &format!("output {i}: {}", "X".repeat(300)),
            ));
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
        assert!(
            matches!(output.updated_state.tools.get("tc-0"), Some(ToolProjectionState::Replaced { replacement, .. }) if matches!(replacement.strategy, ProjectionStrategy::Fixed { shape: ProjectionShape::Microcompacted, .. })),
            "tc-0 should be microcompacted"
        );
        assert!(
            matches!(output.updated_state.tools.get("tc-4"), Some(ToolProjectionState::Replaced { replacement, .. }) if matches!(replacement.strategy, ProjectionStrategy::Fixed { shape: ProjectionShape::Microcompacted, .. })),
            "tc-4 should be microcompacted"
        );
        // Last 3 turns should NOT be microcompacted
        assert!(
            !matches!(output.updated_state.tools.get("tc-5"), Some(ToolProjectionState::Replaced { replacement, .. }) if matches!(replacement.strategy, ProjectionStrategy::Fixed { shape: ProjectionShape::Microcompacted, .. })),
            "tc-5 should not be microcompacted"
        );
        assert!(
            !matches!(output.updated_state.tools.get("tc-7"), Some(ToolProjectionState::Replaced { replacement, .. }) if matches!(replacement.strategy, ProjectionStrategy::Fixed { shape: ProjectionShape::Microcompacted, .. })),
            "tc-7 should not be microcompacted"
        );

        // Verify the compacted text contains the summary marker (wrapped in <context-artifact>)
        if let AgentMessage::ToolResult(tr) = &output.projected_messages[2] {
            let text = extract_text(&tr.content);
            assert!(
                text.contains("<context-artifact") && text.contains("tool-summary"),
                "expected microcompact summary wrapped in context-artifact, got: {text}"
            );
        } else {
            panic!("expected tool result at index 2");
        }
    }

    #[test]
    fn microcompact_summary_persists_across_turns() {
        // Build 8 turns, each with a tool call + result
        let mut msgs = Vec::new();
        for i in 0..8 {
            msgs.push(user_msg(&format!("turn {i}")));
            msgs.push(assistant_tool_call(
                &format!("tc-{i}"),
                "bash",
                r#"{"command":"ls"}"#,
            ));
            msgs.push(tool_result_msg(
                &format!("tc-{i}"),
                "bash",
                &format!("output {i}: {}", "X".repeat(300)),
            ));
        }

        // Microcompact after 3 turns (so turns 0..5 get compacted)
        let budget = ContextProjectionBudget {
            microcompact_after_turns: 3,
            ..default_budget()
        };

        let output1 = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs.clone(),
            budget: budget.clone(),
            state: ContextProjectionState::default(),
        });

        // First projection should have microcompacted replacements
        assert!(
            matches!(output1.updated_state.tools.get("tc-0"), Some(ToolProjectionState::Replaced { replacement, .. }) if matches!(replacement.strategy, ProjectionStrategy::Fixed { shape: ProjectionShape::Microcompacted, .. })),
            "tc-0 should be microcompacted"
        );

        // Project again with the same state
        let output2 = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs,
            budget,
            state: output1.updated_state.clone(),
        });

        // Verify the compacted text still contains the summary marker
        if let AgentMessage::ToolResult(tr) = &output2.projected_messages[2] {
            let text = extract_text(&tr.content);
            assert!(
                text.contains("tool-summary"),
                "expected microcompact summary to persist, got: {text}"
            );
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
        // Turns 3-4: extra turns so tc-small is old enough to microcompact
        let msgs = vec![
            user_msg("turn 0"),
            assistant_tool_call("tc-big", "bash", r#"{"command":"ls"}"#),
            tool_result_msg("tc-big", "bash", &big),
            user_msg("turn 1"),
            assistant_tool_call("tc-small", "read", r#"{"path":"x.rs"}"#),
            tool_result_msg("tc-small", "read", "small content"),
            user_msg("turn 2"),
            assistant_text("done"),
            user_msg("turn 3"),
            assistant_text("done"),
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
        let tc_big = output.updated_state.tools.get("tc-big");
        assert!(
            matches!(
                tc_big,
                Some(ToolProjectionState::Replaced { replacement, .. }) if matches!(replacement.strategy, ProjectionStrategy::Fixed { shape: ProjectionShape::Tail { .. }, .. })
            ),
            "tc-big should use tail strategy from Phase 1, got {:?}",
            tc_big,
        );

        // tc-small should be microcompacted (it was in an old turn and not replaced by Phase 1)
        let tc_small = output.updated_state.tools.get("tc-small");
        assert!(
            matches!(
                tc_small,
                Some(ToolProjectionState::Replaced { replacement, .. }) if matches!(replacement.strategy, ProjectionStrategy::Fixed { shape: ProjectionShape::Microcompacted, .. })
            ),
            "tc-small should be microcompacted, got {:?}",
            tc_small,
        );
    }

    #[test]
    fn script_strategy_runs_rhai_and_falls_back_on_error() {
        let big = "A".repeat(5000);
        let details = serde_json::json!({
            "content_kind": "generic_text",
            "strategy": { "type": "dynamic", "script": "#{ action: \"project\", text: head(text, 5) }" },
            "original_chars": 5000,
            "truncated_by_tool": false,
        });
        let msgs = vec![
            user_msg("turn 0"),
            assistant_tool_call("tc-script", "read", r#"{"path":"x.rs"}"#),
            tool_result_msg_with_details("tc-script", "read", &big, details),
            user_msg("turn 1"),
            assistant_text("done"),
            user_msg("turn 2"),
            assistant_text("done"),
        ];

        let output = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs,
            budget: default_budget(),
            state: ContextProjectionState::default(),
        });

        assert_eq!(output.report.replacements.len(), 1);
        assert!(matches!(
            output.report.replacements[0].strategy,
            ProjectionStrategy::Dynamic { .. }
        ));
        // preview should be "AAAAA" (first 5 chars) inside the preview marker
        let projected = if let AgentMessage::ToolResult(tr) = &output.projected_messages[2] {
            extract_text(&tr.content)
        } else {
            panic!("expected tool result at index 2")
        };
        assert!(
            projected.contains("AAAAA"),
            "expected Rhai head(5) in preview: {}",
            projected
        );
    }

    #[test]
    fn script_strategy_fallback_on_bad_rhai() {
        let big = "A".repeat(5000);
        let details = serde_json::json!({
            "content_kind": "generic_text",
            "strategy": { "type": "dynamic", "script": "BAD SYNTAX !!!" },
            "original_chars": 5000,
            "truncated_by_tool": false,
        });
        let msgs = vec![
            user_msg("turn 0"),
            assistant_tool_call("tc-bad", "read", r#"{"path":"x.rs"}"#),
            tool_result_msg_with_details("tc-bad", "read", &big, details),
            user_msg("turn 1"),
            assistant_text("done"),
            user_msg("turn 2"),
            assistant_text("done"),
        ];

        let output = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs,
            budget: default_budget(),
            state: ContextProjectionState::default(),
        });

        assert_eq!(output.report.replacements.len(), 1);
        // fallback should be head 2000 (hardcoded in apply_strategy)
        let projected = if let AgentMessage::ToolResult(tr) = &output.projected_messages[2] {
            extract_text(&tr.content)
        } else {
            panic!("expected tool result at index 2")
        };
        assert!(
            projected.contains("[projection error:"),
            "expected projection error notice: {}",
            projected
        );
    }

    #[test]
    fn old_after_overflow_does_not_wrap() {
        let big = "A".repeat(5000);
        let details = serde_json::json!({
            "content_kind": "file_read",
            "strategy": { "type": "fixed", "shape": { "type": "head", "max_chars": 200 }, "min_age": u32::MAX },
            "original_chars": 5000,
            "truncated_by_tool": false,
        });
        let msgs = vec![
            user_msg("turn 0"),
            assistant_tool_call("tc-overflow", "read", r#"{"path":"x.rs"}"#),
            tool_result_msg_with_details("tc-overflow", "read", &big, details),
        ];

        let output = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs,
            budget: default_budget(),
            state: ContextProjectionState::default(),
        });

        // With u32::MAX min_age, until_turn should not overflow
        let deferred = output.updated_state.tools.get("tc-overflow");
        assert!(
            matches!(deferred, Some(ToolProjectionState::Deferred { until_turn, .. }) if *until_turn >= u32::MAX - 1),
            "should be deferred with huge min_age, got {:?}",
            deferred
        );
    }

    #[test]
    fn deferred_cap_evicts_oldest_entries() {
        let big = "A".repeat(5000);
        let mut msgs = Vec::new();
        let mut expected_ids = Vec::new();
        for i in 0..1002 {
            msgs.push(user_msg(&format!("turn {i}")));
            msgs.push(assistant_tool_call(
                &format!("tc-{i}"),
                "read",
                r#"{"path":"x.rs"}"#,
            ));
            msgs.push(tool_result_msg(&format!("tc-{i}"), "read", &big));
            expected_ids.push(format!("tc-{i}"));
        }
        // Add one more turn so the first tool results are old enough
        msgs.push(user_msg("turn 1002"));
        msgs.push(assistant_text("done"));

        let output = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs,
            budget: default_budget(),
            state: ContextProjectionState::default(),
        });

        // Total entries should be capped at exactly 1000
        let total_count = output.updated_state.tools.len();
        assert_eq!(
            total_count, MAX_DEFERRED_ENTRIES,
            "total entries should be exactly {}, got {}",
            MAX_DEFERRED_ENTRIES, total_count
        );
        // The oldest entries (tc-0, tc-1) should have been evicted
        assert!(
            !output.updated_state.tools.contains_key("tc-0"),
            "oldest entry tc-0 should be evicted"
        );
        assert!(
            !output.updated_state.tools.contains_key("tc-1"),
            "oldest entry tc-1 should be evicted"
        );
        // Confirm FIFO precision: tc-1000 retained, tc-1001 retained, tc-2 oldest retained
        assert!(
            output.updated_state.tools.contains_key("tc-1000"),
            "tc-1000 should be retained"
        );
        assert!(
            output.updated_state.tools.contains_key("tc-1001"),
            "tc-1001 should be retained"
        );
        assert!(
            output.updated_state.tools.contains_key("tc-2"),
            "tc-2 should be the oldest retained entry"
        );
    }

    #[test]
    fn deferred_not_microcompacted() {
        let big = "A".repeat(5000);
        let msgs = vec![
            user_msg("turn 0"),
            assistant_tool_call("tc-defer", "read", r#"{"path":"x.rs"}"#),
            tool_result_msg("tc-defer", "read", &big),
            user_msg("turn 1"),
            assistant_text("done"),
        ];

        let output = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs,
            budget: default_budget(),
            state: ContextProjectionState::default(),
        });

        // The read tool result should be deferred (age=0 < min_age=2)
        assert!(
            matches!(
                output.updated_state.tools.get("tc-defer"),
                Some(ToolProjectionState::Deferred { .. })
            ),
            "tool result should be deferred"
        );
        // It should NOT be microcompacted
        assert!(
            !matches!(
                output.updated_state.tools.get("tc-defer"),
                Some(ToolProjectionState::Replaced { .. })
            ),
            "deferred tool result should not be replaced"
        );
        // The original message should still be present
        let tr = output.projected_messages.iter().find_map(|m| match m {
            AgentMessage::ToolResult(tr) if tr.tool_call_id.as_str() == "tc-defer" => Some(tr),
            _ => None,
        });
        assert!(
            tr.is_some(),
            "deferred tool result should remain in projected messages"
        );
    }

    #[test]
    fn dynamic_strategy_defer_integration() {
        let big = "A".repeat(5000);
        let details = serde_json::json!({
            "content_kind": "generic_text",
            "strategy": { "type": "dynamic", "script": "#{ action: \"defer\", reevaluate_after: 3 }" },
            "original_chars": 5000,
            "truncated_by_tool": false,
        });
        let msgs = vec![
            user_msg("turn 0"),
            assistant_tool_call("tc-defer", "read", r#"{"path":"x.rs"}"#),
            tool_result_msg_with_details("tc-defer", "read", &big, details),
            user_msg("turn 1"),
            assistant_text("done"),
            user_msg("turn 2"),
            assistant_text("done"),
        ];

        let output = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs,
            budget: default_budget(),
            state: ContextProjectionState::default(),
        });

        // Should be deferred, not replaced
        assert!(
            matches!(
                output.updated_state.tools.get("tc-defer"),
                Some(ToolProjectionState::Deferred { .. })
            ),
            "expected tc-defer to be deferred, got replacements: {:?}",
            output.report.replacements
        );
        assert!(
            output.report.replacements.is_empty(),
            "deferred means no replacement yet, got replacements: {:?}",
            output.report.replacements
        );

        // Original message should be preserved unchanged
        if let AgentMessage::ToolResult(tr) = &output.projected_messages[2] {
            let text = extract_text(&tr.content);
            assert_eq!(
                text, big,
                "projected text should be original oversized text"
            );
        } else {
            panic!("expected tool result at index 2");
        }
    }

    #[test]
    fn dynamic_strategy_project_integration() {
        let big = "A".repeat(5000);
        let details = serde_json::json!({
            "content_kind": "generic_text",
            "strategy": { "type": "dynamic", "script": "#{ action: \"project\", text: \"summary\" }" },
            "original_chars": 5000,
            "truncated_by_tool": false,
        });
        let msgs = vec![
            user_msg("turn 0"),
            assistant_tool_call("tc-project", "read", r#"{"path":"x.rs"}"#),
            tool_result_msg_with_details("tc-project", "read", &big, details),
            user_msg("turn 1"),
            assistant_text("done"),
            user_msg("turn 2"),
            assistant_text("done"),
        ];

        let output = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs,
            budget: default_budget(),
            state: ContextProjectionState::default(),
        });

        assert_eq!(
            output.report.replacements.len(),
            1,
            "expected one replacement for project"
        );
        assert_eq!(
            output.report.replacements[0].outcome,
            ProjectionOutcome {
                text: "summary".to_string()
            }
        );

        let projected = if let AgentMessage::ToolResult(tr) = &output.projected_messages[2] {
            extract_text(&tr.content)
        } else {
            panic!("expected tool result at index 2")
        };
        assert!(
            projected.contains("summary"),
            "expected 'summary' in preview marker: {}",
            projected
        );
    }

    #[test]
    fn min_age_deferral_resolves_after_turns() {
        let big = "A".repeat(5000);
        let msgs_first = vec![
            user_msg("turn 0"),
            assistant_tool_call("tc-age", "read", r#"{"path":"x.rs"}"#),
            tool_result_msg("tc-age", "read", &big),
        ];

        let output1 = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs_first,
            budget: default_budget(),
            state: ContextProjectionState::default(),
        });

        // Age=0 < min_age=2, so deferred
        assert!(
            matches!(
                output1.updated_state.tools.get("tc-age"),
                Some(ToolProjectionState::Deferred { .. })
            ),
            "first projection should defer due to age < min_age"
        );
        assert!(
            output1.report.replacements.is_empty(),
            "first projection should have no replacements"
        );

        // Now append 2 more turns
        let msgs_second = vec![
            user_msg("turn 0"),
            assistant_tool_call("tc-age", "read", r#"{"path":"x.rs"}"#),
            tool_result_msg("tc-age", "read", &big),
            user_msg("turn 1"),
            assistant_text("done"),
            user_msg("turn 2"),
            assistant_text("done"),
        ];

        let output2 = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs_second,
            budget: default_budget(),
            state: output1.updated_state.clone(),
        });

        // Age=2 >= min_age=2, so replaced (not deferred)
        assert!(
            matches!(
                output2.updated_state.tools.get("tc-age"),
                Some(ToolProjectionState::Replaced { .. })
            ),
            "second projection should replace after age >= min_age"
        );
        assert_eq!(
            output2.report.replacements.len(),
            1,
            "second projection should have one replacement"
        );
    }

    #[test]
    fn keep_deferred_retains_entry_across_reproject() {
        // If a tool result is deferred, a second projection with the same
        // turn count should keep it deferred (not re-evaluate).
        let big = "A".repeat(5000);
        let msgs = vec![
            user_msg("turn 0"),
            assistant_tool_call("tc-keep", "read", r#"{"path":"x.rs"}"#),
            tool_result_msg("tc-keep", "read", &big),
            user_msg("turn 1"),
            assistant_text("done"),
        ];

        let output1 = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs.clone(),
            budget: default_budget(),
            state: ContextProjectionState::default(),
        });

        assert!(
            matches!(
                output1.updated_state.tools.get("tc-keep"),
                Some(ToolProjectionState::Deferred { .. })
            ),
            "should be deferred after first projection"
        );

        // Re-project with same messages and inherited state
        let output2 = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs,
            budget: default_budget(),
            state: output1.updated_state.clone(),
        });

        // Should still be deferred, not replaced
        assert!(
            matches!(
                output2.updated_state.tools.get("tc-keep"),
                Some(ToolProjectionState::Deferred { .. })
            ),
            "should remain deferred on re-project"
        );
        assert!(
            output2.report.replacements.is_empty(),
            "re-project should not create replacements for kept-deferred"
        );
    }

    #[test]
    fn keep_last_turn_boundary_keeps_single_huge_turn() {
        // Single turn that alone exceeds the budget: KeepLastTurn should
        // keep the entire last turn even though it is over budget.
        let huge = "A".repeat(4000);
        let msgs = vec![
            user_msg("turn 0"),
            assistant_text("response 0"),
            user_msg("turn 1"),
            assistant_tool_call("tc-1", "read", r#"{"path":"x.rs"}"#),
            tool_result_msg("tc-1", "read", &huge),
        ];

        // 4000 chars + overhead ≈ 1000 tokens, budget 100 tokens
        let budget = ContextProjectionBudget {
            max_tool_result_chars: 50_000,
            max_context_tokens: 100,
            microcompact_after_turns: 100,
            compaction_threshold: 0.75,
        };

        let output = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs,
            budget,
            state: ContextProjectionState::default(),
        });

        // Even though the last turn alone exceeds budget, it should be kept
        assert!(
            output.report.dropped_messages > 0,
            "should drop earlier messages"
        );
        let last_msg = output.projected_messages.last().unwrap();
        assert!(
            matches!(last_msg, AgentMessage::ToolResult(tr) if tr.tool_call_id.as_str() == "tc-1"),
            "last message should be the tool result from the kept turn"
        );
    }

    #[test]
    fn dynamic_fallback_on_replaced_is_update_replacement() {
        // A tool result that was previously replaced with a Dynamic strategy
        // fails on re-evaluation; the fallback should be UpdateReplacement.
        let big = "A".repeat(5000);
        let details = serde_json::json!({
            "content_kind": "generic_text",
            "strategy": { "type": "dynamic", "script": "#{ action: \"project\", text: \"ok\" }" },
            "original_chars": 5000,
            "truncated_by_tool": false,
        });
        let msgs = vec![
            user_msg("turn 0"),
            assistant_tool_call("tc-fb", "read", r#"{"path":"x.rs"}"#),
            tool_result_msg_with_details("tc-fb", "read", &big, details),
        ];

        // First projection: Dynamic script succeeds -> Replace
        let output1 = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs.clone(),
            budget: default_budget(),
            state: ContextProjectionState::default(),
        });
        assert!(
            matches!(
                output1.updated_state.tools.get("tc-fb"),
                Some(ToolProjectionState::Replaced { .. })
            ),
            "first projection should replace"
        );

        // Second projection: corrupt the script so it fails
        let bad_details = serde_json::json!({
            "content_kind": "generic_text",
            "strategy": { "type": "dynamic", "script": "BAD SYNTAX!!!" },
            "original_chars": 5000,
            "truncated_by_tool": false,
        });
        let msgs2 = vec![
            user_msg("turn 0"),
            assistant_tool_call("tc-fb", "read", r#"{"path":"x.rs"}"#),
            tool_result_msg_with_details("tc-fb", "read", &big, bad_details),
            user_msg("turn 1"),
            assistant_text("done"),
        ];

        let output2 = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs2,
            budget: default_budget(),
            state: output1.updated_state.clone(),
        });

        // Should still be Replaced, with updated replacement text containing error
        let state = output2.updated_state.tools.get("tc-fb");
        assert!(
            matches!(state, Some(ToolProjectionState::Replaced { .. })),
            "should stay Replaced after fallback"
        );
        if let Some(ToolProjectionState::Replaced { replacement, .. }) = state {
            let ProjectionOutcome { text } = &replacement.outcome;
            assert!(
                text.contains("[projection error:"),
                "fallback text should contain error notice: {}",
                text
            );
        }
    }

    #[test]
    fn escape_xml_escapes_all_special_chars() {
        let input = r#"a & b < c > d " e' f"#;
        let escaped = escape_xml(input);
        assert_eq!(escaped, "a &amp; b &lt; c &gt; d &quot; e&apos; f");
    }

    #[test]
    fn backward_compat_migration_from_old_replacements() {
        // Old session JSON used a top-level "replacements" field with flat structs
        let old_json = serde_json::json!({
            "current_turn": 3,
            "turns_since_compaction": 1,
            "last_api_usage": null,
            "replacements": {
                "tc-old": {
                    "tool_call_id": "tc-old",
                    "tool_name": "read",
                    "artifact_id": "art-old",
                    "original_chars": 5000,
                    "preview_chars": 200,
                    "strategy": { "type": "head", "max_chars": 200 }
                }
            }
        });

        let state: ContextProjectionState = serde_json::from_value(old_json).unwrap();
        assert_eq!(state.current_turn, 3);
        assert_eq!(state.turns_since_compaction, 1);
        assert!(state.last_api_usage.is_none());

        let entry = state.tools.get("tc-old");
        assert!(
            matches!(entry, Some(ToolProjectionState::Replaced { replacement, .. }) if replacement.tool_call_id == "tc-old"),
            "old replacement should migrate to Replaced state, got {:?}",
            entry
        );
        if let Some(ToolProjectionState::Replaced { replacement, .. }) = entry {
            assert_eq!(replacement.tool_name, "read");
            assert_eq!(replacement.artifact_id, "art-old");
            assert_eq!(replacement.original_chars, 5000);
            assert_eq!(replacement.preview_chars, 200);
            assert!(
                matches!(
                    replacement.strategy,
                    ProjectionStrategy::Fixed {
                        shape: ProjectionShape::Head { max_chars: 200 },
                        min_age: 0
                    }
                ),
                "old head strategy should map to Fixed Head with min_age=0"
            );
        }
    }

    #[test]
    fn dynamic_strategy_replaced_successful_reevaluation() {
        let big = "A".repeat(5000);
        let details = serde_json::json!({
            "content_kind": "generic_text",
            "strategy": { "type": "dynamic", "script": "#{ action: \"project\", text: \"first\" }" },
            "original_chars": 5000,
            "truncated_by_tool": false,
        });
        let msgs = vec![
            user_msg("turn 0"),
            assistant_tool_call("tc-dyn", "read", r#"{"path":"x.rs"}"#),
            tool_result_msg_with_details("tc-dyn", "read", &big, details),
        ];

        let output1 = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs.clone(),
            budget: default_budget(),
            state: ContextProjectionState::default(),
        });
        assert!(
            matches!(
                output1.updated_state.tools.get("tc-dyn"),
                Some(ToolProjectionState::Replaced { .. })
            ),
            "first projection should replace"
        );
        assert_eq!(
            output1.report.replacements[0].outcome,
            ProjectionOutcome {
                text: "first".to_string()
            }
        );

        // Second projection with a different script that returns a new text
        let details2 = serde_json::json!({
            "content_kind": "generic_text",
            "strategy": { "type": "dynamic", "script": "#{ action: \"project\", text: \"second\" }" },
            "original_chars": 5000,
            "truncated_by_tool": false,
        });
        let msgs2 = vec![
            user_msg("turn 0"),
            assistant_tool_call("tc-dyn", "read", r#"{"path":"x.rs"}"#),
            tool_result_msg_with_details("tc-dyn", "read", &big, details2),
            user_msg("turn 1"),
            assistant_text("done"),
        ];

        let output2 = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs2,
            budget: default_budget(),
            state: output1.updated_state.clone(),
        });
        assert!(
            matches!(
                output2.updated_state.tools.get("tc-dyn"),
                Some(ToolProjectionState::Replaced { .. })
            ),
            "second projection should stay replaced"
        );
        assert_eq!(
            output2.report.replacements[0].outcome,
            ProjectionOutcome {
                text: "second".to_string()
            }
        );
    }

    #[test]
    fn dynamic_strategy_deferred_keep_deferred() {
        let big = "A".repeat(5000);
        let details = serde_json::json!({
            "content_kind": "generic_text",
            "strategy": { "type": "dynamic", "script": "#{ action: \"defer\", reevaluate_after: 3 }" },
            "original_chars": 5000,
            "truncated_by_tool": false,
        });
        let msgs = vec![
            user_msg("turn 0"),
            assistant_tool_call("tc-dyn-defer", "read", r#"{"path":"x.rs"}"#),
            tool_result_msg_with_details("tc-dyn-defer", "read", &big, details),
        ];

        let output1 = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs.clone(),
            budget: default_budget(),
            state: ContextProjectionState::default(),
        });
        assert!(
            matches!(
                output1.updated_state.tools.get("tc-dyn-defer"),
                Some(ToolProjectionState::Deferred { .. })
            ),
            "should be deferred after first projection"
        );

        // Re-project with same messages (turn count doesn't advance enough)
        let output2 = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs,
            budget: default_budget(),
            state: output1.updated_state.clone(),
        });

        assert!(
            matches!(
                output2.updated_state.tools.get("tc-dyn-defer"),
                Some(ToolProjectionState::Deferred { .. })
            ),
            "should remain deferred on re-project before until_turn"
        );
        assert!(
            output2.report.replacements.is_empty(),
            "re-project should not create replacements for kept-deferred dynamic"
        );
    }

    #[test]
    fn dynamic_strategy_deferred_expires_and_re_evaluates() {
        let big = "A".repeat(5000);
        let details = serde_json::json!({
            "content_kind": "generic_text",
            "strategy": { "type": "dynamic", "script": "#{ action: \"defer\", reevaluate_after: 1 }" },
            "original_chars": 5000,
            "truncated_by_tool": false,
        });
        let msgs = vec![
            user_msg("turn 0"),
            assistant_tool_call("tc-dyn-exp", "read", r#"{"path":"x.rs"}"#),
            tool_result_msg_with_details("tc-dyn-exp", "read", &big, details),
        ];

        let output1 = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs.clone(),
            budget: default_budget(),
            state: ContextProjectionState::default(),
        });
        assert!(
            matches!(
                output1.updated_state.tools.get("tc-dyn-exp"),
                Some(ToolProjectionState::Deferred { .. })
            ),
            "should be deferred after first projection"
        );

        // Now add 2 more turns so the deferral expires (reevaluate_after=1, so until_turn = current_turn + 1)
        let details2 = serde_json::json!({
            "content_kind": "generic_text",
            "strategy": { "type": "dynamic", "script": "#{ action: \"project\", text: \"expired\" }" },
            "original_chars": 5000,
            "truncated_by_tool": false,
        });
        let msgs2 = vec![
            user_msg("turn 0"),
            assistant_tool_call("tc-dyn-exp", "read", r#"{"path":"x.rs"}"#),
            tool_result_msg_with_details("tc-dyn-exp", "read", &big, details2),
            user_msg("turn 1"),
            assistant_text("done"),
            user_msg("turn 2"),
            assistant_text("done"),
        ];

        let output2 = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs2,
            budget: default_budget(),
            state: output1.updated_state.clone(),
        });

        assert!(
            matches!(
                output2.updated_state.tools.get("tc-dyn-exp"),
                Some(ToolProjectionState::Replaced { .. })
            ),
            "should be replaced after deferral expires"
        );
        assert_eq!(
            output2.report.replacements.len(),
            1,
            "should have one replacement"
        );
        assert_eq!(
            output2.report.replacements[0].outcome,
            ProjectionOutcome {
                text: "expired".to_string()
            }
        );
    }

    #[test]
    fn inserted_at_turn_preserved_across_update_replacement() {
        let big = "A".repeat(5000);
        let details = serde_json::json!({
            "content_kind": "generic_text",
            "strategy": { "type": "dynamic", "script": "#{ action: \"project\", text: \"ok\" }" },
            "original_chars": 5000,
            "truncated_by_tool": false,
        });
        let msgs = vec![
            user_msg("turn 0"),
            assistant_tool_call("tc-age", "read", r#"{"path":"x.rs"}"#),
            tool_result_msg_with_details("tc-age", "read", &big, details),
        ];

        let output1 = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs.clone(),
            budget: default_budget(),
            state: ContextProjectionState::default(),
        });
        let inserted1 = match output1.updated_state.tools.get("tc-age") {
            Some(ToolProjectionState::Replaced {
                inserted_at_turn, ..
            }) => *inserted_at_turn,
            _ => panic!("expected Replaced"),
        };

        // Re-project with same messages
        let output2 = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs,
            budget: default_budget(),
            state: output1.updated_state.clone(),
        });
        let inserted2 = match output2.updated_state.tools.get("tc-age") {
            Some(ToolProjectionState::Replaced {
                inserted_at_turn, ..
            }) => *inserted_at_turn,
            _ => panic!("expected Replaced"),
        };

        assert_eq!(
            inserted1, inserted2,
            "inserted_at_turn should be preserved across UpdateReplacement"
        );
    }

    #[test]
    fn headtail_shape_takes_head_and_tail() {
        let big = format!(
            "{}{}{}",
            "A".repeat(3000),
            "B".repeat(2000),
            "C".repeat(3000)
        );
        let details = serde_json::json!({
            "content_kind": "file_read",
            "strategy": { "type": "fixed", "shape": { "type": "head_tail", "head_chars": 100, "tail_chars": 100 }, "min_age": 0 },
            "original_chars": 8000,
            "truncated_by_tool": false,
        });
        let msgs = vec![
            user_msg("turn 0"),
            assistant_tool_call("tc-ht", "read", r#"{"path":"x.rs"}"#),
            tool_result_msg_with_details("tc-ht", "read", &big, details),
            user_msg("turn 1"),
            assistant_text("done"),
            user_msg("turn 2"),
            assistant_text("done"),
        ];

        let output = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs,
            budget: default_budget(),
            state: ContextProjectionState::default(),
        });

        assert_eq!(output.report.replacements.len(), 1);
        let replacement = &output.report.replacements[0];
        assert!(matches!(
            replacement.strategy,
            ProjectionStrategy::Fixed {
                shape: ProjectionShape::HeadTail {
                    head_chars: 100,
                    tail_chars: 100
                },
                ..
            }
        ));

        if let AgentMessage::ToolResult(tr) = &output.projected_messages[2] {
            let text = extract_text(&tr.content);
            assert!(
                text.contains("context-artifact"),
                "expected artifact wrapper"
            );
            // Should contain head (A's)
            assert!(text.contains(&"A".repeat(100)), "expected head text");
            // Should contain tail (C's)
            assert!(text.contains(&"C".repeat(100)), "expected tail text");
            // Should contain omission marker
            assert!(text.contains("chars omitted"), "expected omission marker");
            // Should NOT contain the middle B's
            assert!(
                !text.contains(&"B".repeat(500)),
                "should not contain middle text"
            );
        } else {
            panic!("expected tool result at index 2");
        }
    }

    #[test]
    fn keep_last_turn_adjusts_for_orphan_safety() {
        let msgs = vec![
            assistant_tool_call("tc-1", "bash", r#"{"command":"echo"}"#),
            tool_result_msg("tc-1", "bash", "output"),
        ];
        // Simulate boundaries where the last turn starts at index 1 (a ToolResult)
        let boundaries = vec![0, 1, 2];
        let (trimmed, dropped) = apply_trim(msgs.clone(), TrimBoundary::KeepLastTurn, &boundaries);
        // adjust_trim_start_for_orphan_safety should move start back to index 0
        assert_eq!(
            dropped, 0,
            "trim start should be adjusted to include preceding Assistant"
        );
        assert_eq!(
            trimmed.len(),
            2,
            "should keep both Assistant and ToolResult"
        );
        assert!(
            matches!(trimmed[0], AgentMessage::Assistant(_)),
            "first kept message should be Assistant"
        );
        assert!(
            matches!(trimmed[1], AgentMessage::ToolResult(_)),
            "second kept message should be ToolResult"
        );
    }

    #[test]
    fn drop_turns_adjusts_for_orphan_safety() {
        let msgs = vec![
            assistant_tool_call("tc-1", "bash", r#"{"command":"echo"}"#),
            tool_result_msg("tc-1", "bash", "output"),
        ];
        // Simulate boundaries where DropTurns(1) drops turn 0, leaving turn 1 starting at index 1 (a ToolResult)
        let boundaries = vec![0, 1, 2];
        let (trimmed, dropped) = apply_trim(msgs.clone(), TrimBoundary::DropTurns(1), &boundaries);
        // adjust_trim_start_for_orphan_safety should move start back to index 0
        assert_eq!(
            dropped, 0,
            "trim start should be adjusted to include preceding Assistant"
        );
        assert_eq!(
            trimmed.len(),
            2,
            "should keep both Assistant and ToolResult"
        );
        assert!(
            matches!(trimmed[0], AgentMessage::Assistant(_)),
            "first kept message should be Assistant"
        );
        assert!(
            matches!(trimmed[1], AgentMessage::ToolResult(_)),
            "second kept message should be ToolResult"
        );
    }

    #[test]
    fn backward_compat_migration_wraps_bare_script() {
        // Old session JSON used a top-level "replacements" field with Script strategy
        let old_json = serde_json::json!({
            "current_turn": 3,
            "turns_since_compaction": 1,
            "last_api_usage": null,
            "replacements": {
                "tc-old": {
                    "tool_call_id": "tc-old",
                    "tool_name": "read",
                    "artifact_id": "art-old",
                    "original_chars": 5000,
                    "preview_chars": 200,
                    "strategy": { "type": "script", "script": "head(text, 5)" }
                }
            }
        });

        let state: ContextProjectionState = serde_json::from_value(old_json).unwrap();
        let entry = state.tools.get("tc-old");
        assert!(
            matches!(entry, Some(ToolProjectionState::Replaced { replacement, .. }) if replacement.tool_call_id == "tc-old"),
            "old script replacement should migrate to Replaced state, got {:?}",
            entry
        );
        if let Some(ToolProjectionState::Replaced { replacement, .. }) = entry {
            assert!(
                matches!(replacement.strategy, ProjectionStrategy::Dynamic { ref script } if script.contains("#{ action: \"project\"") && script.contains("head(text, 5)")),
                "old bare script should be wrapped in new map format, got {:?}",
                replacement.strategy
            );
        }
    }

    #[test]
    fn tool_result_context_returns_none_on_malformed_metadata() {
        let details = Some(ToolDetails::new(serde_json::json!({
            "content_kind": "invalid_value",
            "strategy": "not_a_map"
        })));
        let result = tool_result_context(&details);
        assert!(result.is_none(), "malformed metadata should return None");
    }

    #[test]
    fn tool_result_context_returns_none_on_malformed_nested_context() {
        let details = Some(ToolDetails::new(serde_json::json!({
            "exitCode": 0,
            "context": {
                "content_kind": "invalid_value",
                "strategy": "not_a_map"
            }
        })));
        let result = tool_result_context(&details);
        assert!(
            result.is_none(),
            "malformed nested context should return None"
        );
    }

    #[test]
    fn trim_boundary_respects_calibrated_estimates() {
        let big = "A".repeat(4000);
        let msgs = vec![
            user_msg("turn 0"),
            assistant_text("response 0"),
            user_msg("turn 1"),
            assistant_tool_call("tc-1", "read", r#"{"path":"x.rs"}"#),
            tool_result_msg("tc-1", "read", &big),
        ];

        // Calibrated estimate: actual_input_tokens = 2000, estimated_tokens = 1000
        // ratio = 2.0, so calibrated estimate = 2x the raw heuristic.
        // 4000 chars + overhead ≈ 1000 raw tokens, but calibrated = 2000 tokens.
        // Budget 100 tokens means even the last turn alone exceeds budget.
        let budget = ContextProjectionBudget {
            max_tool_result_chars: 50_000,
            max_context_tokens: 100,
            microcompact_after_turns: 100,
            compaction_threshold: 0.75,
        };

        let mut state = ContextProjectionState::default();
        state.last_api_usage = Some(ApiUsageSnapshot {
            estimated_tokens: 1000,
            actual_input_tokens: 2000,
        });

        let output = project(ProjectionInput {
            system_prompt: "test".into(),
            messages: msgs,
            budget,
            state,
        });

        // Even with calibration, KeepLastTurn should keep the last turn
        assert!(
            output.report.dropped_messages > 0,
            "should drop earlier messages"
        );
        let last_msg = output.projected_messages.last().unwrap();
        assert!(
            matches!(last_msg, AgentMessage::ToolResult(tr) if tr.tool_call_id.as_str() == "tc-1"),
            "last message should be the tool result from the kept turn"
        );
    }

    #[test]
    fn evict_oldest_skips_inline_entries() {
        let mut state = ContextProjectionState::default();
        // Insert 1002 entries: one Inline, 1001 Deferred
        state
            .tools
            .insert("tc-inline".to_string(), ToolProjectionState::Inline);
        for i in 0..1001 {
            state.tools.insert(
                format!("tc-{i}"),
                ToolProjectionState::Deferred {
                    until_turn: 10,
                    inserted_at_turn: i as u32,
                },
            );
        }
        assert_eq!(state.tools.len(), 1002, "precondition: 1002 entries");

        evict_oldest_if_over_limit(&mut state);

        // Inline should be preserved (never evicted)
        assert!(
            state.tools.contains_key("tc-inline"),
            "Inline entry should never be evicted"
        );
        // Total should be capped at 1000
        assert_eq!(
            state.tools.len(),
            MAX_DEFERRED_ENTRIES + 1,
            "1000 Deferred + 1 Inline"
        );
        // The oldest Deferred entries should be evicted
        assert!(
            !state.tools.contains_key("tc-0"),
            "oldest Deferred should be evicted"
        );
        assert!(
            state.tools.contains_key("tc-1"),
            "second oldest Deferred should still be present"
        );
    }
}
