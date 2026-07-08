use ratatui::style::Color;

/// Stateless color palette for the TUI.
pub struct Theme;

impl Theme {
    pub const ACCENT: Color = Color::Rgb(138, 190, 183);
    pub const BLUE: Color = Color::Rgb(95, 135, 255);
    pub const YELLOW: Color = Color::Rgb(240, 198, 116);
    pub const DIM_GRAY: Color = Color::Rgb(102, 102, 102);
    pub const DARK_GRAY: Color = Color::Rgb(80, 80, 80);
}
