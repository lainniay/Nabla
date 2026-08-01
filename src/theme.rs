use ratatui::style::Color;

/// The single semantic color system used by every Nabla surface.
///
/// Values are the official Catppuccin Mocha palette. Keeping the palette here
/// makes status meaning consistent and gives ANSI-only terminals one place to
/// select a conservative fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiTheme {
    pub text: Color,
    pub subtext: Color,
    pub muted: Color,
    pub border: Color,
    pub surface0: Color,
    pub surface1: Color,
    pub primary: Color,
    pub user: Color,
    pub assistant: Color,
    pub goal: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
}

impl UiTheme {
    pub const MOCHA: Self = Self {
        text: Color::Rgb(205, 214, 244),
        subtext: Color::Rgb(166, 173, 200),
        muted: Color::Rgb(127, 132, 156),
        border: Color::Rgb(88, 91, 112),
        surface0: Color::Rgb(49, 50, 68),
        surface1: Color::Rgb(69, 71, 90),
        primary: Color::Rgb(203, 166, 247),
        user: Color::Rgb(137, 180, 250),
        assistant: Color::Rgb(203, 166, 247),
        goal: Color::Rgb(148, 226, 213),
        success: Color::Rgb(166, 227, 161),
        warning: Color::Rgb(250, 179, 135),
        error: Color::Rgb(243, 139, 168),
    };

    pub const ANSI16: Self = Self {
        text: Color::White,
        subtext: Color::Gray,
        muted: Color::DarkGray,
        border: Color::DarkGray,
        surface0: Color::Black,
        surface1: Color::DarkGray,
        primary: Color::Magenta,
        user: Color::Blue,
        assistant: Color::Magenta,
        goal: Color::Cyan,
        success: Color::Green,
        warning: Color::Yellow,
        error: Color::Red,
    };
}

pub const THEME: UiTheme = UiTheme::MOCHA;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mocha_tokens_match_the_official_palette() {
        assert_eq!(THEME.text, Color::Rgb(205, 214, 244));
        assert_eq!(THEME.subtext, Color::Rgb(166, 173, 200));
        assert_eq!(THEME.muted, Color::Rgb(127, 132, 156));
        assert_eq!(THEME.border, Color::Rgb(88, 91, 112));
        assert_eq!(THEME.surface0, Color::Rgb(49, 50, 68));
        assert_eq!(THEME.surface1, Color::Rgb(69, 71, 90));
        assert_eq!(THEME.primary, Color::Rgb(203, 166, 247));
        assert_eq!(THEME.user, Color::Rgb(137, 180, 250));
        assert_eq!(THEME.goal, Color::Rgb(148, 226, 213));
        assert_eq!(THEME.success, Color::Rgb(166, 227, 161));
        assert_eq!(THEME.warning, Color::Rgb(250, 179, 135));
        assert_eq!(THEME.error, Color::Rgb(243, 139, 168));
    }
}
