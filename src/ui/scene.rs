use crate::state::{ToolStatus, TranscriptItem};

use view_model::SceneViewModel;

use super::{
    text::{cells_width, wrap_text},
    types::{CellStyle, StyledCell, VisualRow},
};

pub(crate) const COMPOSER_CHROME_HEIGHT: u16 = 2;
pub(crate) const MAX_COMPOSER_CONTENT_HEIGHT: u16 = 6;

pub use builder::SceneBuilder;

pub fn animation_active(view: &SceneViewModel) -> bool {
    view.run_state.is_busy()
        || view.transcript.iter().any(|item| {
            matches!(
                item,
                TranscriptItem::Tool(tool)
                    if matches!(
                        tool.status,
                        ToolStatus::Running
                            | ToolStatus::WaitingApproval
                    )
            )
        })
}

pub(crate) fn text_row(id: &str, text: &str, style: CellStyle, width: u16) -> VisualRow {
    wrap_text(id, text, width.max(1), style)
        .into_iter()
        .next()
        .unwrap_or_else(|| VisualRow::blank(id))
}

pub(crate) fn input_border_row(
    id: &str,
    left: &str,
    right: &str,
    width: u16,
    style: CellStyle,
) -> VisualRow {
    let mut row = composer_border_row(left, right, width, style);
    row.component_id = id.to_owned();
    row
}

pub(crate) fn composer_border_row(
    left: &str,
    right: &str,
    width: u16,
    style: CellStyle,
) -> VisualRow {
    let mut cells = vec![StyledCell::new(left, 1, style)];
    let fill = width.saturating_sub(2);
    if fill > 0 {
        cells.push(StyledCell::new("─".repeat(usize::from(fill)), fill, style));
    }
    if width > 1 {
        cells.push(StyledCell::new(right, 1, style));
    }
    VisualRow {
        component_id: "composer".to_owned(),
        logical_line: 0,
        wrap_index: 0,
        cells,
    }
}

pub(crate) fn append_text_cells(cells: &mut Vec<StyledCell>, text: &str, style: CellStyle) {
    if let Some(row) = wrap_text("inline", text, u16::MAX, style)
        .into_iter()
        .next()
    {
        cells.extend(row.cells);
    }
}

pub mod builder;
pub mod canvas;
pub mod composer;
pub mod panels;
pub mod status;
pub mod view_model;

#[cfg(test)]
mod tests;
