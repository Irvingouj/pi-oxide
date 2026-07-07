use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
};
use ratatui::Frame;

use crate::app::{App, ChatEntry};
use crate::theme::Theme;

/// Pure scroll-position computation. Returns the row offset to pass to
/// `Paragraph::scroll((offset, 0))`.
pub(crate) fn compute_scroll(
    total_lines: u16,
    visible: u16,
    auto_scroll: bool,
    scroll_offset: u16,
) -> u16 {
    if total_lines <= visible {
        return 0;
    }
    if auto_scroll {
        total_lines - visible
    } else {
        scroll_offset.min(total_lines - visible)
    }
}

/// Returns `(start, end)` — half-open row range to materialize for the
/// current frame. `end - start <= visible`.
pub(crate) fn visible_range(total_lines: u16, visible: u16, scroll: u16) -> (u16, u16) {
    if total_lines <= visible {
        return (0, total_lines);
    }
    let scroll = scroll.min(total_lines);
    let end = (scroll + visible).min(total_lines);
    (scroll, end)
}
/// Count how many wrapped lines a single ChatEntry produces.
fn count_entry_lines(entry: &ChatEntry, wrap_width: usize) -> u16 {
    entry.line_count(wrap_width)
}

/// Emit all logical lines of a ChatEntry into `lines`.
/// The caller is responsible for only calling this when the entry overlaps the visible range.
fn emit_entry(entry: &ChatEntry, lines: &mut Vec<Line<'static>>) {
    match entry {
        ChatEntry::User(text) => {
            lines.push(Line::from(vec![
                Span::styled("\u{258c} ", Style::default().fg(Theme::ACCENT)),
                Span::styled(
                    "You",
                    Style::default()
                        .fg(Theme::ACCENT)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(": "),
            ]));
            for line in text.lines() {
                lines.push(Line::from(vec![
                    Span::styled("\u{258c} ", Style::default().fg(Theme::ACCENT)),
                    Span::styled(line.to_string(), Style::default().fg(Theme::TEXT_LIGHT)),
                ]));
            }
            lines.push(Line::raw(""));
        }
        ChatEntry::Assistant(text) => {
            for line in text.lines.iter().cloned() {
                // Skip accent bar on blank lines (markdown trailing separators)
                let is_blank =
                    line.spans.is_empty() || line.spans.iter().all(|s| s.content.is_empty());
                if is_blank {
                    lines.push(Line::raw(""));
                } else {
                    let mut spans = vec![Span::styled(
                        "\u{258c} ",
                        Style::default().fg(Theme::ACCENT),
                    )];
                    spans.extend(line.spans);
                    lines.push(Line::from(spans));
                }
            }
            lines.push(Line::raw(""));
        }
        ChatEntry::ToolStart { name, args_summary } => {
            lines.push(Line::from(vec![
                Span::styled(" \u{256d}\u{2500} ", Style::default().fg(Theme::DARK_GRAY)),
                Span::styled(
                    name.clone(),
                    Style::default()
                        .fg(Theme::BLUE)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" {}", args_summary),
                    Style::default().fg(Theme::DIM_GRAY),
                ),
            ]));
        }
        ChatEntry::ToolResult {
            output, is_error, ..
        } => {
            let color = Theme::tool_border_color(false, *is_error);
            let footer_icon = if *is_error { "\u{2717}" } else { "\u{2713}" };
            for line in output.lines() {
                lines.push(Line::from(vec![
                    Span::styled(" \u{2502} ", Style::default().fg(color)),
                    Span::styled(line.to_string(), Style::default().fg(Theme::TEXT_MID)),
                ]));
            }
            lines.push(Line::styled(
                format!(" \u{2570}\u{2500}{}", footer_icon),
                Style::default().fg(color),
            ));
            lines.push(Line::raw(""));
        }
        ChatEntry::System(text) => {
            lines.push(Line::from(vec![
                Span::styled("\u{25c7} ", Style::default().fg(Theme::DIM_GRAY)),
                Span::styled(text.to_string(), Style::default().fg(Theme::GRAY)),
            ]));
            lines.push(Line::raw(""));
        }
    }
}

// Format a token count for display, matching the web-ui formatTokens logic.
fn format_tokens(count: u32) -> String {
    if count < 1000 {
        count.to_string()
    } else if count < 10_000 {
        format!("{:.1}k", count as f64 / 1000.0)
    } else if count < 1_000_000 {
        format!("{}k", (count as f64 / 1000.0).round() as u32)
    } else if count < 10_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else {
        format!("{}M", (count as f64 / 1_000_000.0).round() as u32)
    }
}

// Format context-usage string and pick a color by percentage.
// Returns `(display_string, color)`.
fn format_ctx_usage(input_tokens: u32, context_window: u32, auto_compact: bool) -> (String, Color) {
    if context_window == 0 {
        return (
            format!("?/{}", format_tokens(context_window)),
            Color::DarkGray,
        );
    }

    let pct = input_tokens as f64 / context_window as f64 * 100.0;

    let color = if pct <= 70.0 {
        Color::Green
    } else if pct <= 90.0 {
        Color::Yellow
    } else {
        Color::Red
    };

    let suffix = if auto_compact { " (auto)" } else { "" };
    (
        format!("{:.1}%/{}{}", pct, format_tokens(context_window), suffix),
        color,
    )
}

impl App {
    /// Count the total number of wrapped lines across all entries.
    pub(crate) fn wrapped_line_count(&self, width: usize) -> u16 {
        let mut total: u16 = 0;
        for entry in &self.entries {
            total += count_entry_lines(entry, width);
        }
        if self.running && self.streaming_text.is_empty() && self.running_tasks.is_empty() {
            total += 1;
        }
        total
    }

    pub(crate) fn render_chat(&self, frame: &mut Frame, area: Rect) {
        let visible = area.height.saturating_sub(2);
        let width = area.width as usize;

        // Check if we need the streaming indicator
        let has_streaming =
            self.running && self.streaming_text.is_empty() && self.running_tasks.is_empty();

        // Pass 1: count approximate wrapped lines per entry
        let mut entry_line_counts: Vec<u16> =
            Vec::with_capacity(self.entries.len() + if has_streaming { 1 } else { 0 });
        for entry in &self.entries {
            let count = count_entry_lines(entry, width);
            entry_line_counts.push(count);
        }
        if has_streaming {
            entry_line_counts.push(1); // "Thinking..." is one line
        }

        // Total wrapped lines (+1 for top padding line)
        const TOP_PAD: u16 = 1;
        let total_lines: u16 = entry_line_counts.iter().sum::<u16>() + TOP_PAD;

        // Compute scroll position and visible range
        let scroll = compute_scroll(total_lines, visible, self.auto_scroll, self.scroll_offset);
        let (start, end) = visible_range(total_lines, visible, scroll);

        // If visible_range returns an empty or inverted range, render nothing but keep scrollbar
        if start >= end && total_lines > visible {
            // Still render scrollbar
            let mut scrollbar_state =
                ScrollbarState::new(total_lines as usize).position(scroll as usize);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight).thumb_symbol("█"),
                area,
                &mut scrollbar_state,
            );
            return;
        }

        // Pass 2: materialize only visible rows
        // Prepend a blank line for top padding
        let mut lines: Vec<Line<'static>> = vec![Line::raw("")];
        let mut current_row: u16 = TOP_PAD;
        let mut first_emitted_row: u16 = TOP_PAD;

        for (idx, entry) in self.entries.iter().enumerate() {
            let entry_len = entry_line_counts[idx];
            let entry_start = current_row;
            let entry_end = current_row + entry_len;

            // Skip if this entry is entirely before the visible range
            if entry_end <= start {
                current_row = entry_end;
                continue;
            }
            // Stop if this entry starts at or after the visible range end
            if entry_start >= end {
                break;
            }

            // Track the first content row emitted (for paragraph scroll adjustment)
            // lines[0] is the padding line, so check len == 1 (padding only)
            if lines.len() == 1 {
                first_emitted_row = entry_start;
            }

            // Emit this entry — caller already verified it overlaps [start, end)
            emit_entry(entry, &mut lines);
            current_row = entry_end;
        }

        // Handle streaming indicator
        if has_streaming {
            let stream_row = current_row;
            if stream_row < end && stream_row + 1 > start {
                let frame = self.get_spinner_frame();
                lines.push(Line::from(vec![
                    Span::styled(format!("  {} ", frame), Style::default().fg(Theme::YELLOW)),
                    Span::styled(
                        "Thinking",
                        Style::default()
                            .fg(Theme::DIM_GRAY)
                            .add_modifier(Modifier::ITALIC),
                    ),
                ]));
            }
        }
        // Paragraph includes padding line at index 0, so add 1 to skip it
        let paragraph_scroll = scroll.saturating_sub(first_emitted_row) + 1;
        let paragraph = Paragraph::new(ratatui::text::Text::from(lines))
            .scroll((paragraph_scroll, 0))
            .block(Block::new().borders(Borders::NONE))
            .wrap(Wrap { trim: false });

        frame.render_widget(paragraph, area);

        if total_lines > visible {
            let mut scrollbar_state =
                ScrollbarState::new(total_lines as usize).position(scroll as usize);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight).thumb_symbol("█"),
                area,
                &mut scrollbar_state,
            );
        }
    }

    /// Build the status-bar spans. Extracted so the formatting logic is testable.
    fn build_status_spans(&self) -> Vec<Span<'static>> {
        let model_name = self.llm_client.model_id();
        let mut spans = vec![
            Span::styled(
                " pio",
                Style::default()
                    .fg(Theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" · {}", model_name),
                Style::default().fg(Theme::GRAY),
            ),
        ];

        if let Some((input, output, _total)) = self.last_usage {
            let context_window = self.context_window;
            let auto_compact = self.budget.compaction_threshold > 0.0;
            let (ctx_text, ctx_color) = format_ctx_usage(input, context_window, auto_compact);

            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                format!("in:{:.1}k", input as f64 / 1000.0),
                Style::default().fg(Theme::DIM_GRAY),
            ));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                format!("out:{:.1}k", output as f64 / 1000.0),
                Style::default().fg(Theme::DIM_GRAY),
            ));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(ctx_text, Style::default().fg(ctx_color)));
        }

        if !self.running_tasks.is_empty() {
            let count = self.running_tasks.len();
            spans.push(Span::styled(
                format!("  ⠋ {} tools", count),
                Style::default().fg(Theme::BLUE),
            ));
        } else if self.running {
            spans.push(Span::styled("  ⠋", Style::default().fg(Theme::YELLOW)));
        }

        spans
    }

    pub(crate) fn render_status(&self, frame: &mut Frame, area: Rect) {
        let spans = self.build_status_spans();
        let status = Paragraph::new(Line::from(spans))
            .style(Style::default().bg(Theme::BG_STATUS).fg(Theme::GRAY));
        frame.render_widget(status, area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_scroll_empty_returns_zero() {
        assert_eq!(compute_scroll(0, 10, true, 0), 0);
    }

    #[test]
    fn compute_scroll_fits_returns_zero() {
        assert_eq!(compute_scroll(5, 10, true, 0), 0);
    }

    #[test]
    fn compute_scroll_auto_pins_to_bottom() {
        assert_eq!(compute_scroll(100, 10, true, 0), 90);
    }

    #[test]
    fn compute_scroll_manual_respects_offset() {
        assert_eq!(compute_scroll(100, 10, false, 30), 30);
    }

    #[test]
    fn compute_scroll_manual_clamps_overflow() {
        assert_eq!(compute_scroll(100, 10, false, 9999), 90);
    }

    #[test]
    fn visible_range_fits_full() {
        assert_eq!(visible_range(5, 10, 0), (0, 5));
    }

    #[test]
    fn visible_range_middle() {
        assert_eq!(visible_range(100, 10, 30), (30, 40));
    }

    #[test]
    fn visible_range_clamps_near_bottom() {
        assert_eq!(visible_range(95, 10, 90), (90, 95));
    }

    #[test]
    fn visible_range_scroll_beyond_end() {
        assert_eq!(visible_range(50, 10, 60), (50, 50));
    }

    // format_tokens tests

    #[test]
    fn format_tokens_zero() {
        assert_eq!(format_tokens(0), "0");
    }

    #[test]
    fn format_tokens_below_1k() {
        assert_eq!(format_tokens(42), "42");
        assert_eq!(format_tokens(999), "999");
    }

    #[test]
    fn format_tokens_at_1k_boundary() {
        assert_eq!(format_tokens(1000), "1.0k");
    }

    #[test]
    fn format_tokens_1k_to_10k() {
        assert_eq!(format_tokens(1500), "1.5k");
        assert_eq!(format_tokens(5000), "5.0k");
        assert_eq!(format_tokens(9999), "10.0k");
    }

    #[test]
    fn format_tokens_at_10k_boundary() {
        assert_eq!(format_tokens(10000), "10k");
    }

    #[test]
    fn format_tokens_10k_to_1m() {
        assert_eq!(format_tokens(15000), "15k");
        assert_eq!(format_tokens(500000), "500k");
        assert_eq!(format_tokens(999999), "1000k");
    }

    #[test]
    fn format_tokens_at_1m_boundary() {
        assert_eq!(format_tokens(1000000), "1.0M");
    }

    #[test]
    fn format_tokens_1m_to_10m() {
        assert_eq!(format_tokens(1500000), "1.5M");
        assert_eq!(format_tokens(5000000), "5.0M");
        assert_eq!(format_tokens(9999999), "10.0M");
    }

    #[test]
    fn format_tokens_at_10m_boundary() {
        assert_eq!(format_tokens(10000000), "10M");
    }

    #[test]
    fn format_tokens_above_10m() {
        assert_eq!(format_tokens(15000000), "15M");
        assert_eq!(format_tokens(50000000), "50M");
        assert_eq!(format_tokens(u32::MAX), "4295M");
    }

    // format_ctx_usage tests

    #[test]
    fn format_ctx_usage_normal_with_auto() {
        let (text, color) = format_ctx_usage(1280, 128000, true);
        assert_eq!(text, "1.0%/128k (auto)");
        assert_eq!(color, Color::Green);
    }

    #[test]
    fn format_ctx_usage_high_yellow() {
        let (text, color) = format_ctx_usage(75000, 100000, false);
        assert_eq!(color, Color::Yellow);
        assert_eq!(text, "75.0%/100k");
    }

    #[test]
    fn format_ctx_usage_critical_red() {
        let (text, color) = format_ctx_usage(95000, 100000, false);
        assert_eq!(color, Color::Red);
        assert_eq!(text, "95.0%/100k");
    }

    #[test]
    fn format_ctx_usage_auto_off() {
        let (text, color) = format_ctx_usage(1280, 128000, false);
        assert_eq!(text, "1.0%/128k");
        assert_eq!(color, Color::Green);
    }

    #[test]
    fn format_ctx_usage_zero_window() {
        let (text, color) = format_ctx_usage(100, 0, true);
        assert_eq!(text, "?/0");
        assert_eq!(color, Color::DarkGray);
    }

    #[test]
    fn format_ctx_usage_exact_70_percent_is_green() {
        let (_text, color) = format_ctx_usage(70000, 100000, false);
        assert_eq!(color, Color::Green);
    }

    #[test]
    fn format_ctx_usage_exact_90_percent_is_yellow() {
        let (_text, color) = format_ctx_usage(90000, 100000, false);
        assert_eq!(color, Color::Yellow);
    }

    #[test]
    fn format_ctx_usage_over_100_percent_is_red() {
        let (text, color) = format_ctx_usage(150_000, 128_000, true);
        assert!(text.contains("117.2%"));
        assert_eq!(color, Color::Red);
    }

    // build_status_spans tests

    fn dummy_model() -> pi_core::Model {
        pi_core::Model {
            id: pi_core::ModelId("test".to_string()),
            name: pi_core::ModelName("test".to_string()),
            api: pi_core::ApiName("openai".to_string()),
            provider: pi_core::ProviderName("openai".to_string()),
            base_url: None,
            reasoning: false,
            context_window: 128_000,
            max_tokens: 8192,
            capabilities: pi_core::ModelCapabilities::default(),
            cost: pi_core::ModelCost::default(),
        }
    }

    fn app_with_agent() -> App {
        let runtime = pi_core::AgentRuntime::new(pi_core::AgentOptions {
            system_prompt: "test".to_string(),
            model: dummy_model(),
            thinking_level: pi_core::events::ThinkingLevel::default(),
            steering_mode: pi_core::events::QueueMode::default(),
            follow_up_mode: pi_core::events::QueueMode::default(),
            tool_execution_mode: pi_core::tool::ExecutionMode::default(),
            session_id: None,
        });
        App {
            agent_host: crate::agent_host::AgentHost::new(runtime),
            entries: Vec::new(),
            editor: crate::editor::Editor::new(),
            scroll_offset: 0,
            auto_scroll: true,
            should_quit: false,
            running: false,
            streaming_text: String::new(),
            streaming_start: None,
            current_tools: Vec::new(),
            tool_definitions: Vec::new(),
            llm_client: crate::llm::LlmClient::new(
                "x",
                "x",
                "test-model",
                crate::llm::WireFormat::OpenAI,
            ),
            host_state: None,
            last_usage: None,
            session_id: None,
            session_backend: crate::session::FileSystemSessionBackend::new(),
            cwd: std::path::PathBuf::from("."),
            cancelled: false,
            pending_quit: false,
            model_picker: None,
            extensions: Vec::new(),
            running_tasks: Vec::new(),
            session_logger: None,
            budget: pi_core::ContextProjectionBudget::default(),
            context_window: 128_000,
            last_chat_area: Rect::ZERO,
            resolved_config: crate::config::ResolvedConfig {
                model: "test".into(),
                provider: "openai".into(),
                api_key: "***".into(),
                base_url: "x".into(),
                config_path: None,
            },
            thinking_level: pi_core::events::ThinkingLevel::Off,
            needs_render: true,
            last_spinner_frame: "",
        }
    }

    /// Extract the text content from a span for easy assertion.
    fn span_text<'a>(s: &'a Span<'static>) -> &'a str {
        s.content.as_ref()
    }

    #[test]
    fn build_status_spans_no_usage_shows_model_only() {
        let app = app_with_agent();
        let spans = app.build_status_spans();
        assert_eq!(spans.len(), 2);
        assert_eq!(span_text(&spans[0]), " pio");
        assert_eq!(span_text(&spans[1]), " · test-model");
    }

    #[test]
    fn build_status_spans_with_usage_shows_ctx_format() {
        let mut app = app_with_agent();
        app.last_usage = Some((1280, 500, 1780));
        let spans = app.build_status_spans();

        // Brand + model
        assert_eq!(span_text(&spans[0]), " pio");
        assert_eq!(span_text(&spans[1]), " · test-model");
        // Separator
        assert_eq!(span_text(&spans[2]), "  ");
        // in:1.3k
        assert_eq!(span_text(&spans[3]), "in:1.3k");
        // space
        assert_eq!(span_text(&spans[4]), " ");
        // out:0.5k
        assert_eq!(span_text(&spans[5]), "out:0.5k");
        // space
        assert_eq!(span_text(&spans[6]), " ");
        // ctx usage: "1.0%/128k (auto)" with Green
        assert_eq!(span_text(&spans[7]), "1.0%/128k (auto)");
        assert_eq!(spans[7].style.fg, Some(Color::Green));
    }

    #[test]
    fn build_status_spans_running_indicator() {
        let mut app = app_with_agent();
        app.running = true;
        let spans = app.build_status_spans();
        // Last span should be the running indicator
        assert_eq!(span_text(spans.last().unwrap()), "  ⠋");
        assert_eq!(spans.last().unwrap().style.fg, Some(Theme::YELLOW));
    }

    #[test]
    fn build_status_spans_running_tasks_indicator() {
        let mut app = app_with_agent();
        let (_tx, rx) = std::sync::mpsc::channel();
        app.running_tasks.push(crate::app::RunningTask {
            tool_call_id: pi_core::types::ToolCallId("tc1".to_string()),
            tool_name: "test_tool".to_string(),
            stream: Box::new(rx),
        });
        let spans = app.build_status_spans();
        // Last span should be the tools indicator
        assert_eq!(span_text(spans.last().unwrap()), "  ⠋ 1 tools");
        assert_eq!(spans.last().unwrap().style.fg, Some(Theme::BLUE));
    }

    #[test]
    fn build_status_spans_high_usage_is_yellow() {
        let mut app = app_with_agent();
        app.last_usage = Some((100_000, 500, 100_500));
        let spans = app.build_status_spans();
        // Find the ctx usage span (last before any running indicator)
        let ctx_span = spans
            .iter()
            .rev()
            .find(|s| s.content.contains('%'))
            .unwrap();
        assert!(ctx_span.content.contains("78.1%"));
        assert_eq!(ctx_span.style.fg, Some(Color::Yellow));
    }

    #[test]
    fn build_status_spans_critical_usage_is_red() {
        let mut app = app_with_agent();
        app.last_usage = Some((120_000, 500, 120_500));
        let spans = app.build_status_spans();
        let ctx_span = spans
            .iter()
            .rev()
            .find(|s| s.content.contains('%'))
            .unwrap();
        assert!(ctx_span.content.contains("93.8%"));
        assert_eq!(ctx_span.style.fg, Some(Color::Red));
    }

    #[test]
    fn build_status_spans_auto_compact_off_no_suffix() {
        let mut app = app_with_agent();
        app.last_usage = Some((1280, 500, 1780));
        app.budget.compaction_threshold = 0.0;
        let spans = app.build_status_spans();
        let ctx_span = spans
            .iter()
            .rev()
            .find(|s| s.content.contains('%'))
            .unwrap();
        assert_eq!(ctx_span.content, "1.0%/128k");
        assert!(!ctx_span.content.contains("auto"));
    }

    #[test]
    fn build_status_spans_running_with_tasks_shows_tools_not_dot() {
        let mut app = app_with_agent();
        app.running = true;
        let (_tx, rx) = std::sync::mpsc::channel();
        app.running_tasks.push(crate::app::RunningTask {
            tool_call_id: pi_core::types::ToolCallId("tc1".to_string()),
            tool_name: "test_tool".to_string(),
            stream: Box::new(rx),
        });
        let spans = app.build_status_spans();
        // Should show tools indicator, not bare running spinner
        assert_eq!(span_text(spans.last().unwrap()), "  ⠋ 1 tools");
        // Should NOT have a bare "  ⠋" running indicator
        assert!(!spans.iter().any(|s| s.content.as_ref() == "  ⠋"));
    }
}
