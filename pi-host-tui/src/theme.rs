use ratatui::style::{Color, Modifier, Style};

use pi_core::ThinkingLevel;

/// Stateless color palette and style helpers for the TUI.
///
/// All values are compile-time constants derived from pi's dark theme.
/// No runtime configuration or environment variables.
pub struct Theme;

impl Theme {
    // ── Palette constants ────────────────────────────────────────────

    pub const ACCENT: Color = Color::Rgb(138, 190, 183); // muted teal
    pub const BLUE: Color = Color::Rgb(95, 135, 255); // soft blue (tool names)
    pub const GREEN: Color = Color::Rgb(181, 189, 104); // muted green (success)
    pub const RED: Color = Color::Rgb(204, 102, 102); // soft red (error)
    pub const YELLOW: Color = Color::Rgb(240, 198, 116); // warm gold (headings)
    pub const GRAY: Color = Color::Rgb(128, 128, 128); // mid gray (muted text)
    pub const DIM_GRAY: Color = Color::Rgb(102, 102, 102); // dim text
    pub const DARK_GRAY: Color = Color::Rgb(80, 80, 80); // borders / chrome
    pub const TEXT_LIGHT: Color = Color::Rgb(220, 220, 220); // body text
    pub const TEXT_MID: Color = Color::Rgb(180, 180, 180); // secondary text
    pub const BG_STATUS: Color = Color::Rgb(20, 20, 25); // status bar bg
    #[allow(dead_code)]
    pub const BG_TOOL_PENDING: Color = Color::Rgb(40, 40, 50); // tool pending bg
    #[allow(dead_code)]
    pub const BG_TOOL_SUCCESS: Color = Color::Rgb(40, 50, 40); // tool success bg
    #[allow(dead_code)]
    pub const BG_TOOL_ERROR: Color = Color::Rgb(60, 40, 40); // tool error bg

    // ── Thinking-level colors ────────────────────────────────────────

    /// Returns the border/indicator color for the given thinking level.
    pub fn thinking_color(level: ThinkingLevel) -> Color {
        match level {
            ThinkingLevel::Off => Self::DARK_GRAY,
            ThinkingLevel::Minimal => Color::Rgb(110, 110, 110),
            ThinkingLevel::Low => Color::Rgb(95, 135, 175),
            ThinkingLevel::Medium => Color::Rgb(129, 162, 190),
            ThinkingLevel::High => Color::Rgb(178, 148, 187),
            ThinkingLevel::Xhigh => Color::Rgb(209, 131, 232),
        }
    }

    // ── Tool border helpers ──────────────────────────────────────────

    /// Returns the border color for a tool result block.
    ///
    /// - Pending → BLUE
    /// - Completed with error → RED
    /// - Completed success → GREEN
    pub fn tool_border_color(pending: bool, is_error: bool) -> Color {
        if pending {
            Self::BLUE
        } else if is_error {
            Self::RED
        } else {
            Self::GREEN
        }
    }

    // ── Input border helpers ─────────────────────────────────────────

    /// Returns the border style for the input area.
    ///
    /// - Running (agent active) → thinking-level color, bold
    /// - Idle → ACCENT, no modifier
    pub fn input_border_style(running: bool, thinking_level: ThinkingLevel) -> Style {
        if running {
            Style::default()
                .fg(Self::thinking_color(thinking_level))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Self::ACCENT)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_thinking_color_off_returns_dark_gray() {
        let color = Theme::thinking_color(ThinkingLevel::Off);
        assert_eq!(color, Theme::DARK_GRAY);
    }

    #[test]
    fn test_thinking_color_high_returns_purple() {
        let color = Theme::thinking_color(ThinkingLevel::High);
        assert_eq!(color, Color::Rgb(178, 148, 187));
    }

    #[test]
    fn test_thinking_color_xhigh_returns_pink() {
        let color = Theme::thinking_color(ThinkingLevel::Xhigh);
        assert_eq!(color, Color::Rgb(209, 131, 232));
    }

    #[test]
    fn test_tool_border_color_pending_returns_blue() {
        let color = Theme::tool_border_color(true, false);
        assert_eq!(color, Theme::BLUE);
    }

    #[test]
    fn test_tool_border_color_success_returns_green() {
        let color = Theme::tool_border_color(false, false);
        assert_eq!(color, Theme::GREEN);
    }

    #[test]
    fn test_tool_border_color_error_returns_red() {
        let color = Theme::tool_border_color(false, true);
        assert_eq!(color, Theme::RED);
    }

    #[test]
    fn test_input_border_style_running_returns_thinking_color() {
        let style = Theme::input_border_style(true, ThinkingLevel::High);
        assert_eq!(style.fg, Some(Theme::thinking_color(ThinkingLevel::High)));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn test_input_border_style_idle_returns_accent() {
        let style = Theme::input_border_style(false, ThinkingLevel::High);
        assert_eq!(style.fg, Some(Theme::ACCENT));
        assert!(!style.add_modifier.contains(Modifier::BOLD));
    }
}
