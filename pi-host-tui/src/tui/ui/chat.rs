use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
};
use ratatui::Frame;

use crate::app::{App, ChatEntry};

impl App {
    pub(crate) fn render_chat(&self, frame: &mut Frame, area: Rect) {
        let mut lines: Vec<Line> = Vec::new();

        for entry in &self.entries {
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
                            name.as_str(),
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
                    name: _,
                    output,
                    is_error,
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

        // Streaming indicator (only when LLM is actually streaming, not waiting for async tools)
        if self.running && self.streaming_text.is_empty() && self.running_tasks.is_empty() {
            lines.push(Line::styled(
                "  ● Thinking...",
                Style::default().fg(Color::DarkGray),
            ));
        }

        let total_lines = lines.len() as u16;
        let visible = area.height.saturating_sub(2);

        let scroll = if total_lines > visible {
            if self.auto_scroll {
                total_lines - visible
            } else {
                self.scroll_offset.min(total_lines - visible)
            }
        } else {
            0
        };

        let paragraph = Paragraph::new(ratatui::text::Text::from(lines))
            .scroll((scroll, 0))
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
            let ctx_pct = if self.budget.max_context_tokens > 0 {
                let est = (input as f64 / self.budget.max_context_tokens as f64 * 100.0) as u16;
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
