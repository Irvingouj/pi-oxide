use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::App;

impl App {
    pub(crate) fn render_input(&mut self, frame: &mut Frame, area: Rect) {
        let style = if self.running {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().fg(Color::White)
        };

        let input = Paragraph::new(self.input.as_str())
            .style(style)
            .block(
                Block::new()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(if self.running {
                        Color::DarkGray
                    } else if self.show_suggestions {
                        Color::Yellow
                    } else {
                        Color::Cyan
                    }))
                    .title(if self.running {
                        " thinking... "
                    } else if self.show_suggestions {
                        " commands "
                    } else {
                        " > "
                    })
                    .title_style(
                        Style::default().fg(if self.running || self.show_suggestions {
                            Color::Yellow
                        } else {
                            Color::Cyan
                        }),
                    ),
            )
            .wrap(Wrap { trim: false });

        frame.render_widget(input, area);

        if !self.running {
            let cursor_x = area.x + 1 + (self.cursor_pos as u16).min(area.width.saturating_sub(3));
            let cursor_y = area.y + 1;
            frame.set_cursor_position((cursor_x, cursor_y));
        }

        // Model picker popup
        if let Some(ref picker) = self.model_picker {
            self.render_model_picker(frame, area, picker);
            return;
        }

        // Suggestion popup
        if self.show_suggestions && !self.suggestions.is_empty() {
            let max_visible = 5u16;
            let list_height = (self.suggestions.len() as u16).min(max_visible);
            let popup_height = list_height + 2;

            let popup_area = Rect {
                x: area.x,
                y: area.y.saturating_sub(popup_height),
                width: area.width,
                height: popup_height,
            };

            frame.render_widget(Clear, popup_area);

            let items: Vec<ListItem> = self
                .suggestions
                .iter()
                .map(|s| ListItem::new(s.as_str()))
                .collect();

            let list = List::new(items)
                .block(Block::bordered().title(" commands "))
                .highlight_style(
                    Style::new()
                        .add_modifier(Modifier::REVERSED)
                        .fg(Color::Cyan),
                )
                .highlight_symbol("> ");

            frame.render_stateful_widget(list, popup_area, &mut self.suggestion_state);
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
                let prefix = if *m == current { "★ " } else { "  " };
                ListItem::new(format!("{}{}", prefix, m))
            })
            .collect();

        let list = List::new(items)
            .block(Block::bordered().title(format!(
                " models ({} of {})",
                filtered.len(),
                picker.total_count()
            )))
            .highlight_style(
                Style::new()
                    .add_modifier(Modifier::REVERSED)
                    .fg(Color::Cyan),
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
        let filter = Paragraph::new(filter_text).style(Style::default().fg(Color::Yellow));
        frame.render_widget(filter, filter_area);
    }
}
