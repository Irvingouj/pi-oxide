//! Markdown → ratatui Text renderer.
//!
//! Uses pulldown-cmark to parse markdown and produces styled ratatui Text.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};

pub fn render(input: &str, _width: u16) -> Text<'static> {
    let mut parser = pulldown_cmark::Parser::new(input);
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut in_code_block = false;
    let mut code_lang = String::new();
    let mut code_lines: Vec<String> = Vec::new();
    let mut current_spans: Vec<Span<'static>> = Vec::new();
    let mut list_depth: usize = 0;

    while let Some(event) = parser.next() {
        match event {
            pulldown_cmark::Event::Start(pulldown_cmark::Tag::CodeBlock(kind)) => {
                flush_spans(&mut current_spans, &mut lines);
                in_code_block = true;
                code_lang = match kind {
                    pulldown_cmark::CodeBlockKind::Fenced(lang) => lang.to_string(),
                    _ => String::new(),
                };
                code_lines.clear();
            }
            pulldown_cmark::Event::End(pulldown_cmark::TagEnd::CodeBlock) => {
                in_code_block = false;
                render_code_block(&code_lang, &code_lines, &mut lines);
            }
            pulldown_cmark::Event::Start(pulldown_cmark::Tag::List(_)) => {
                list_depth += 1;
            }
            pulldown_cmark::Event::End(pulldown_cmark::TagEnd::List(_)) => {
                list_depth = list_depth.saturating_sub(1);
            }
            pulldown_cmark::Event::Start(pulldown_cmark::Tag::Item) => {
                flush_spans(&mut current_spans, &mut lines);
                let indent = "  ".repeat(list_depth.saturating_sub(1));
                current_spans.push(Span::styled(
                    format!("{indent}• "),
                    Style::default().fg(Color::DarkGray),
                ));
            }
            pulldown_cmark::Event::End(pulldown_cmark::TagEnd::Item) => {
                flush_spans(&mut current_spans, &mut lines);
            }
            pulldown_cmark::Event::Start(pulldown_cmark::Tag::Heading { level, .. }) => {
                flush_spans(&mut current_spans, &mut lines);
                let prefix = "#".repeat(level as usize) + " ";
                current_spans.push(Span::styled(
                    prefix,
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ));
            }
            pulldown_cmark::Event::End(pulldown_cmark::TagEnd::Heading(_)) => {
                flush_spans(&mut current_spans, &mut lines);
                lines.push(Line::raw(""));
            }
            pulldown_cmark::Event::Start(pulldown_cmark::Tag::Paragraph) => {}
            pulldown_cmark::Event::End(pulldown_cmark::TagEnd::Paragraph) => {
                flush_spans(&mut current_spans, &mut lines);
                lines.push(Line::raw(""));
            }
            pulldown_cmark::Event::Start(pulldown_cmark::Tag::Strong) => {}
            pulldown_cmark::Event::End(pulldown_cmark::TagEnd::Strong) => {}
            pulldown_cmark::Event::Start(pulldown_cmark::Tag::Emphasis) => {}
            pulldown_cmark::Event::End(pulldown_cmark::TagEnd::Emphasis) => {}
            pulldown_cmark::Event::Code(code) => {
                current_spans.push(Span::styled(
                    code.to_string(),
                    Style::default().fg(Color::Yellow),
                ));
            }
            pulldown_cmark::Event::Start(pulldown_cmark::Tag::Link { dest_url, .. }) => {
                let url = dest_url.to_string();
                current_spans.push(Span::raw("["));
                // Store URL for the End event
                let _ = url;
            }
            pulldown_cmark::Event::End(pulldown_cmark::TagEnd::Link) => {
                current_spans.push(Span::raw("]"));
            }
            pulldown_cmark::Event::Start(pulldown_cmark::Tag::BlockQuote(_)) => {
                flush_spans(&mut current_spans, &mut lines);
                current_spans.push(Span::styled("│ ", Style::default().fg(Color::DarkGray)));
            }
            pulldown_cmark::Event::End(pulldown_cmark::TagEnd::BlockQuote(_)) => {
                flush_spans(&mut current_spans, &mut lines);
            }
            pulldown_cmark::Event::Text(text) => {
                if in_code_block {
                    code_lines.push(text.to_string());
                } else {
                    current_spans.push(Span::raw(text.to_string()));
                }
            }
            pulldown_cmark::Event::SoftBreak | pulldown_cmark::Event::HardBreak => {
                flush_spans(&mut current_spans, &mut lines);
            }
            _ => {}
        }
    }

    flush_spans(&mut current_spans, &mut lines);
    Text::from(lines)
}

fn flush_spans(spans: &mut Vec<Span<'static>>, lines: &mut Vec<Line<'static>>) {
    if !spans.is_empty() {
        lines.push(Line::from(spans.drain(..).collect::<Vec<_>>()));
    }
}

fn render_code_block(_lang: &str, code_lines: &[String], lines: &mut Vec<Line<'static>>) {
    let code_text = code_lines.join("\n");
    let trimmed = code_text.trim();

    if trimmed.is_empty() {
        return;
    }

    for line in trimmed.lines() {
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default().fg(Color::DarkGray)),
            Span::styled(line.to_string(), Style::default().fg(Color::White)),
        ]));
    }
    lines.push(Line::raw(""));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_paragraph() {
        let text = render("Hello world", 80);
        assert!(text.lines.len() > 0);
        // Should have the text line + empty line after paragraph
        let first_line = &text.lines[0];
        assert!(first_line.spans.len() > 0);
    }

    #[test]
    fn test_render_heading() {
        let text = render("# Title\nSome text", 80);
        assert!(text.lines.len() >= 2);
        // First line should be heading with cyan style
    }

    #[test]
    fn test_render_code_block() {
        let text = render("```rust\nfn main() {}\n```", 80);
        assert!(text.lines.len() >= 2);
        // Code line should be indented
        let code_line = &text.lines[0];
        assert!(code_line.spans[0].content.starts_with("  "));
    }

    #[test]
    fn test_render_inline_code() {
        let text = render("Use `cargo build` to compile", 80);
        assert!(text.lines.len() > 0);
        // Should contain yellow-styled code span
        let has_yellow = text
            .lines
            .iter()
            .any(|l| l.spans.iter().any(|s| s.style.fg == Some(Color::Yellow)));
        assert!(has_yellow);
    }

    #[test]
    fn test_render_list() {
        let text = render("- item 1\n- item 2\n- item 3", 80);
        // Should have bullet markers
        let bullet_count = text
            .lines
            .iter()
            .filter(|l| l.spans.iter().any(|s| s.content.contains("•")))
            .count();
        assert_eq!(bullet_count, 3);
    }

    #[test]
    fn test_render_empty() {
        let text = render("", 80);
        assert!(text.lines.is_empty());
    }
}
