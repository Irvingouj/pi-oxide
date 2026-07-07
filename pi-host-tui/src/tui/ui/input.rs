use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::symbols;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::theme::Theme;

impl App {
    pub(crate) fn render_input(&mut self, frame: &mut Frame, area: Rect) {
        let border_style = if self.editor.show_suggestions {
            Style::default().fg(Theme::YELLOW)
        } else {
            Theme::input_border_style(self.running, self.thinking_level)
        };

        let title = if self.running {
            Span::styled(" \u{2809} thinking ", Style::default().fg(Theme::YELLOW))
        } else if self.editor.show_suggestions {
            Span::styled(" commands ", Style::default().fg(Theme::YELLOW))
        } else {
            Span::styled(" pio ", border_style)
        };

        let text_style = if self.running {
            Style::default().fg(Theme::DIM_GRAY)
        } else {
            Style::default().fg(Theme::TEXT_LIGHT)
        };

        // Build the full input line: prefix + typed text
        let prefix = "\u{258c} ";
        let input_spans: Vec<Span> = vec![
            Span::styled(prefix, Style::default().fg(Theme::ACCENT)),
            Span::styled(&self.editor.input, text_style),
        ];

        let input = Paragraph::new(Line::from(input_spans))
            .block(
                Block::default()
                    .border_set(symbols::border::ROUNDED)
                    .border_style(border_style)
                    .title(title)
                    .title_style(border_style),
            )
            .wrap(Wrap { trim: false });

        frame.render_widget(input, area);

        // Cursor position: computed from display width of prefix + text before cursor.
        // cursor_pos is a byte index — slice by bytes, then measure display width.
        if !self.running {
            let before = self.editor.input.get(..self.editor.cursor_pos).unwrap_or("");
            let cursor_x = (area.x
                + 1 // left border
                + prefix.width() as u16
                + before.width() as u16
                - 1)
            .max(area.x + 1)
            .min(area.x + area.width - 1);
            let cursor_y = area.y + 1;
            frame.set_cursor_position((cursor_x, cursor_y));
        }

        // Model picker popup
        if let Some(ref picker) = self.model_picker {
            self.render_model_picker(frame, area, picker);
            return;
        }

        // Suggestion popup
        if self.editor.show_suggestions && !self.editor.suggestions.is_empty() {
            let max_visible = 5u16;
            let list_height = (self.editor.suggestions.len() as u16).min(max_visible);
            let popup_height = list_height + 2;

            let popup_area = Rect {
                x: area.x,
                y: area.y.saturating_sub(popup_height),
                width: area.width,
                height: popup_height,
            };

            frame.render_widget(Clear, popup_area);

            let items: Vec<ListItem> = self
                .editor
                .suggestions
                .iter()
                .map(|s| ListItem::new(s.as_str()))
                .collect();

            let list = List::new(items)
                .block(
                    Block::bordered()
                        .border_set(symbols::border::ROUNDED)
                        .title(" commands "),
                )
                .highlight_style(
                    Style::new()
                        .add_modifier(Modifier::REVERSED)
                        .fg(Theme::ACCENT),
                )
                .highlight_symbol("> ");

            frame.render_stateful_widget(list, popup_area, &mut self.editor.suggestion_state);
        }
    }

    fn render_model_picker(
        &self,
        frame: &mut Frame,
        area: Rect,
        picker: &crate::model_picker::ModelPicker,
    ) {
        let filtered = picker.filtered();
        let current = picker.current_model();
        let selected = picker.selected();
        let filter_text = picker.filter_text();

        let max_visible = 8u16;
        let list_height = (filtered.len() as u16).max(1).min(max_visible);
        let popup_height = list_height + 3; // title + list + filter

        let popup_area = Rect {
            x: area.x,
            y: area.y.saturating_sub(popup_height),
            width: area.width,
            height: popup_height,
        };

        frame.render_widget(Clear, popup_area);

        // Build list items
        let items: Vec<ListItem> = filtered
            .iter()
            .map(|m| {
                if *m == current {
                    let line = Line::from(vec![
                        Span::styled("★ ", Style::default().fg(Theme::ACCENT)),
                        Span::raw(*m),
                    ]);
                    ListItem::new(line)
                } else {
                    ListItem::new(format!("  {}", m))
                }
            })
            .collect();

        let list = List::new(items)
            .block(
                Block::bordered()
                    .border_set(symbols::border::ROUNDED)
                    .title(format!(
                        " models ({} of {})",
                        filtered.len(),
                        picker.total_count()
                    )),
            )
            .highlight_style(
                Style::new()
                    .add_modifier(Modifier::REVERSED)
                    .fg(Theme::ACCENT),
            );

        // Render list
        let mut state = ratatui::widgets::ListState::default();
        // Find the index of the selected item in the filtered list
        if let Some(sel) = selected {
            if let Some(idx) = filtered.iter().position(|&m| m == sel) {
                state.select(Some(idx));
            }
        } else if !filtered.is_empty() {
            state.select(Some(0));
        }
        frame.render_stateful_widget(list, popup_area, &mut state);

        // Render filter line at bottom
        let filter_area = Rect {
            x: area.x,
            y: area.y.saturating_sub(1),
            width: area.width,
            height: 1,
        };
        let filter_text = format!("filter: {}", filter_text);
        let filter = Paragraph::new(filter_text).style(Style::default().fg(Theme::YELLOW));
        frame.render_widget(filter, filter_area);
    }
}
