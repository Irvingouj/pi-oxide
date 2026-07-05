use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
};
use ratatui::Frame;

use crate::app::{App, ChatEntry};
#[allow(unused_imports)]
use crate::llm::LlmProvider;

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
                Span::styled(
                    "You",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(": "),
            ]));
            for line in text.lines() {
                lines.push(Line::from(Span::styled(
                    line.to_string(),
                    Style::default().fg(Color::White),
                )));
            }
            lines.push(Line::raw(""));
        }
        ChatEntry::Assistant(text) => {
            for line in text.lines.iter().cloned() {
                lines.push(line);
            }
            lines.push(Line::raw(""));
        }
        ChatEntry::ToolStart { name, args_summary } => {
            lines.push(Line::from(vec![
                Span::styled(" ┌─ ", Style::default().fg(Color::Yellow)),
                Span::styled(
                    name.clone(),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" {args_summary}"),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }
        ChatEntry::ToolResult {
            output, is_error, ..
        } => {
            let color = if *is_error { Color::Red } else { Color::Green };
            let border = if *is_error { " ┃ " } else { " │ " };
            for line in output.lines() {
                lines.push(Line::from(vec![
                    Span::styled(border, Style::default().fg(color)),
                    Span::styled(line.to_string(), Style::default().fg(color)),
                ]));
            }
            lines.push(Line::styled(" └──", Style::default().fg(color)));
            lines.push(Line::raw(""));
        }
        ChatEntry::System(text) => {
            lines.push(Line::styled(
                format!("  {text}"),
                Style::default().fg(Color::DarkGray),
            ));
            lines.push(Line::raw(""));
        }
    }
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

        // Total wrapped lines
        let total_lines: u16 = entry_line_counts.iter().sum();

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
        let mut lines: Vec<Line<'static>> = Vec::new();
        let mut current_row: u16 = 0;
        let mut first_emitted_row: u16 = 0;

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

            // Track the first row emitted (for paragraph scroll adjustment)
            if lines.is_empty() {
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
                lines.push(Line::styled(
                    "  ● Thinking...",
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }
        let paragraph_scroll = scroll.saturating_sub(first_emitted_row);
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

    pub(crate) fn render_status(&self, frame: &mut Frame, area: Rect) {
        let model_name = self.llm_client.model_id();
        let parts = vec![Span::styled(
            format!(" {model_name}"),
            Style::default().fg(Color::DarkGray),
        )];

        let mut spans = parts;

        if let Some((input, output, _total)) = self.last_usage {
            let budget = &pi_core::ContextProjectionBudget::default();
            let ctx_pct = if budget.max_context_tokens > 0 {
                let est = (input as f64 / budget.max_context_tokens as f64 * 100.0) as u16;
                est.min(100)
            } else {
                0
            };
            let ctx_color = if ctx_pct > 90 {
                Color::Red
            } else if ctx_pct > 70 {
                Color::Yellow
            } else {
                Color::Green
            };
            let bar_full = ctx_pct / 10;
            let bar_empty = 10 - bar_full;
            let bar = format!(
                "{}{}",
                "█".repeat(bar_full as usize),
                "░".repeat(bar_empty as usize)
            );

            spans.push(Span::raw(" │ "));
            spans.push(Span::styled(
                format!("in:{:.1}k", input as f64 / 1000.0),
                Style::default().fg(Color::DarkGray),
            ));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                format!("out:{:.1}k", output as f64 / 1000.0),
                Style::default().fg(Color::DarkGray),
            ));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                format!("ctx:{ctx_pct}% {bar}"),
                Style::default().fg(ctx_color),
            ));
        }

        if !self.running_tasks.is_empty() {
            let count = self.running_tasks.len();
            spans.push(Span::styled(
                format!(" ● {count} tools"),
                Style::default().fg(Color::Yellow),
            ));
        } else if self.running {
            spans.push(Span::styled(" ●", Style::default().fg(Color::Yellow)));
        }

        let status = Paragraph::new(Line::from(spans)).style(Style::default().bg(Color::DarkGray));
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
}
