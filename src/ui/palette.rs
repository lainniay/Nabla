use super::types::{CellStyle, Color};

/// Bump when semantic palette values or Markdown styling rules change.
pub const THEME_REVISION: u64 = 3;

// Catppuccin Mocha — https://catppuccin.com/palette/
//
// UI code should use these semantic colors rather than terminal named colors
// or one-off RGB values. This keeps true-color and ANSI fallback rendering
// visually coherent.
pub const ROSEWATER: Color = Color::Rgb(245, 224, 220);
pub const FLAMINGO: Color = Color::Rgb(242, 205, 205);
pub const PINK: Color = Color::Rgb(245, 194, 231);
pub const MAUVE: Color = Color::Rgb(203, 166, 247);
pub const RED: Color = Color::Rgb(243, 139, 168);
pub const MAROON: Color = Color::Rgb(235, 160, 172);
pub const PEACH: Color = Color::Rgb(250, 179, 135);
pub const YELLOW: Color = Color::Rgb(249, 226, 175);
pub const GREEN: Color = Color::Rgb(166, 227, 161);
pub const TEAL: Color = Color::Rgb(148, 226, 213);
pub const SKY: Color = Color::Rgb(137, 220, 235);
pub const SAPPHIRE: Color = Color::Rgb(116, 199, 236);
pub const BLUE: Color = Color::Rgb(137, 180, 250);
pub const LAVENDER: Color = Color::Rgb(180, 190, 254);
pub const TEXT: Color = Color::Rgb(205, 214, 244);
pub const SUBTEXT_1: Color = Color::Rgb(186, 194, 222);
pub const SUBTEXT_0: Color = Color::Rgb(166, 173, 200);
pub const OVERLAY_2: Color = Color::Rgb(147, 153, 178);
pub const OVERLAY_1: Color = Color::Rgb(127, 132, 156);
pub const OVERLAY_0: Color = Color::Rgb(108, 112, 134);
pub const SURFACE_2: Color = Color::Rgb(88, 91, 112);
pub const SURFACE_1: Color = Color::Rgb(69, 71, 90);
pub const SURFACE_0: Color = Color::Rgb(49, 50, 68);
pub const BASE: Color = Color::Rgb(30, 30, 46);
pub const MANTLE: Color = Color::Rgb(24, 24, 37);
pub const CRUST: Color = Color::Rgb(17, 17, 27);

// Semantic gray ramp drawn directly from Mocha. Keeping neutral hierarchy on
// the same hue family prevents the UI from drifting toward flat ANSI gray.
pub const GRAY_TEXT: Color = OVERLAY_2;
pub const GRAY_MUTED: Color = OVERLAY_0;
pub const GRAY_FAINT: Color = SURFACE_2;
pub const THINKING_TEXT: Color = OVERLAY_1;

pub const INPUT_ACCENT: Color = GRAY_TEXT;
pub const HISTORY_BORDER: Color = GRAY_FAINT;
pub const PANEL_BORDER: Color = MAUVE;
pub const ACTIVE_PATH: Color = TEAL;
pub const DIFF_ADDED_BACKGROUND: Color = Color::Rgb(42, 58, 52);
pub const DIFF_REMOVED_BACKGROUND: Color = Color::Rgb(64, 42, 54);

pub const fn input_border() -> CellStyle {
    CellStyle::foreground(INPUT_ACCENT)
}

pub const fn selected() -> CellStyle {
    CellStyle::foreground(MAUVE).bold()
}

pub const fn selected_muted() -> CellStyle {
    CellStyle::foreground(MAUVE).bold()
}
