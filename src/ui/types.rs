use std::collections::HashMap;

pub type ComponentId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptSyncOutcome {
    Unchanged,
    AppendOnly,
    ProjectionInvalidated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssistantContentKind {
    Thinking,
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssistantSegmentPhase {
    Streaming,
    Stable,
    Sealed,
    Committed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantSegment {
    pub message_id: u64,
    pub session_epoch: u64,
    pub segment_index: usize,
    pub first_in_message: bool,
    pub content_kind: AssistantContentKind,
    pub byte_start: usize,
    pub byte_end: usize,
    pub content_revision: u64,
    pub phase: AssistantSegmentPhase,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TerminalSize {
    pub width: u16,
    pub height: u16,
}

impl TerminalSize {
    pub const fn new(width: u16, height: u16) -> Self {
        Self {
            width: if width == 0 { 1 } else { width },
            height: if height == 0 { 1 } else { height },
        }
    }
}

impl From<(u16, u16)> for TerminalSize {
    fn from((width, height): (u16, u16)) -> Self {
        Self::new(width, height)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl Rect {
    pub const fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub const fn right(self) -> u16 {
        self.x.saturating_add(self.width)
    }

    pub const fn bottom(self) -> u16 {
        self.y.saturating_add(self.height)
    }

    pub const fn contains(self, column: u16, row: u16) -> bool {
        column >= self.x && column < self.right() && row >= self.y && row < self.bottom()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MainLayout {
    pub transcript: Rect,
    pub panel: Option<Rect>,
    pub composer: Rect,
    pub status: Rect,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Color {
    #[default]
    Default,
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    Gray,
    Rgb(u8, u8, u8),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CellStyle {
    pub foreground: Color,
    pub background: Color,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underlined: bool,
    pub crossed_out: bool,
    pub reversed: bool,
}

impl CellStyle {
    pub const fn foreground(foreground: Color) -> Self {
        Self {
            foreground,
            background: Color::Default,
            bold: false,
            dim: false,
            italic: false,
            underlined: false,
            crossed_out: false,
            reversed: false,
        }
    }

    pub const fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    pub const fn dim(mut self) -> Self {
        self.dim = true;
        self
    }

    pub const fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    pub const fn underlined(mut self) -> Self {
        self.underlined = true;
        self
    }

    pub const fn crossed_out(mut self) -> Self {
        self.crossed_out = true;
        self
    }

    pub const fn reversed(mut self) -> Self {
        self.reversed = true;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledCell {
    /// One extended grapheme cluster. Continuation columns are represented by
    /// `width`, not by fake scalar cells.
    pub symbol: String,
    pub width: u16,
    pub style: CellStyle,
}

impl StyledCell {
    pub fn new(symbol: impl Into<String>, width: u16, style: CellStyle) -> Self {
        Self {
            symbol: symbol.into(),
            width,
            style,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualRow {
    pub component_id: ComponentId,
    pub logical_line: usize,
    pub wrap_index: usize,
    pub cells: Vec<StyledCell>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelFrame {
    pub area: Rect,
    /// Exactly `area.height` rows, already clipped to `area.width`.
    pub rows: Vec<VisualRow>,
}

impl VisualRow {
    pub fn blank(component_id: impl Into<ComponentId>) -> Self {
        Self {
            component_id: component_id.into(),
            logical_line: 0,
            wrap_index: 0,
            cells: Vec::new(),
        }
    }

    pub fn display_width(&self) -> u16 {
        self.cells
            .iter()
            .fold(0u16, |width, cell| width.saturating_add(cell.width))
    }

    pub fn plain_text(&self) -> String {
        self.cells.iter().map(|cell| cell.symbol.as_str()).collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowRange {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HitTarget {
    Command(usize),
    Choice(usize),
    ListItem(usize),
    Transcript(ComponentId),
    Composer,
    Panel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HitRegion {
    pub area: Rect,
    pub target: HitTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorPosition {
    pub column: u16,
    pub row: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualFrame {
    pub revision: u64,
    pub terminal_size: TerminalSize,
    /// Exactly `terminal_size.height` rows in terminal coordinates.
    pub rows: Vec<VisualRow>,
    /// A primary-screen overlay. It is composed after `rows` and never
    /// participates in viewport ownership or native-history geometry.
    pub panel: Option<PanelFrame>,
    /// Absolute terminal rows owned by the current live surface. Primary
    /// frames leave rows above this rectangle to native terminal scrollback.
    pub viewport: Rect,
    pub component_bounds: HashMap<ComponentId, RowRange>,
    pub hit_regions: Vec<HitRegion>,
    pub cursor: Option<CursorPosition>,
    pub main_layout: MainLayout,
}

impl VisualFrame {
    pub fn hit_test(&self, column: u16, row: u16) -> Option<&HitTarget> {
        self.hit_regions
            .iter()
            .rev()
            .find(|region| region.area.contains(column, row))
            .map(|region| &region.target)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SurfaceKind {
    #[default]
    Primary,
    Alternate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameUpdate {
    Full(VisualFrame),
    Rows {
        revision: u64,
        rows: Vec<(u16, VisualRow)>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedHistoryBlock {
    pub component_id: ComponentId,
    pub source_revision: u64,
    /// Offset into the component's rendered canonical rows.
    pub row_offset: usize,
    /// Total rendered rows for the component at the projected width.
    pub total_rows: usize,
    pub rows: Vec<VisualRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalReflowProjection {
    pub canonical_revision: u64,
    pub session_epoch: u64,
    pub source_revision: u64,
    pub width: u16,
    /// Canonical prefix omitted by the configured physical replay window.
    pub omitted_components: usize,
    /// Cursor after all replayed history components and before the active tail.
    pub history_end_cursor: usize,
    pub history_blocks: Vec<CommittedHistoryBlock>,
    pub active_rows: Vec<VisualRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalCommitPlan {
    pub revision: u64,
    pub surface: SurfaceKind,
    pub history_scroll_rows: usize,
    pub history_blocks: Vec<CommittedHistoryBlock>,
    pub frame_update: FrameUpdate,
    pub panel: Option<PanelFrame>,
    pub cursor: Option<CursorPosition>,
    pub full_redraw: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_testing_uses_last_region_as_the_topmost_region() {
        let frame = VisualFrame {
            revision: 1,
            terminal_size: TerminalSize::new(80, 24),
            rows: Vec::new(),
            panel: None,
            viewport: Rect::new(0, 20, 80, 4),
            component_bounds: HashMap::new(),
            hit_regions: vec![
                HitRegion {
                    area: Rect::new(0, 0, 20, 5),
                    target: HitTarget::Panel,
                },
                HitRegion {
                    area: Rect::new(0, 2, 20, 1),
                    target: HitTarget::Choice(2),
                },
            ],
            cursor: None,
            main_layout: MainLayout::default(),
        };

        assert_eq!(frame.hit_test(4, 2), Some(&HitTarget::Choice(2)));
        assert_eq!(frame.hit_test(4, 1), Some(&HitTarget::Panel));
        assert_eq!(frame.hit_test(30, 1), None);
    }
}
