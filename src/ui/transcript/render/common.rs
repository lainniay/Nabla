use crate::state::TurnSeparator;
use crate::ui::{
    palette,
    text::{display_width, truncate, wrap_text},
    types::{CellStyle, StyledCell, VisualRow},
};

pub(crate) fn styled_cells(text: &str, style: CellStyle) -> Vec<StyledCell> {
    wrap_text("inline", text, u16::MAX, style)
        .into_iter()
        .next()
        .map(|row| row.cells)
        .unwrap_or_default()
}

pub(crate) fn cells_width(cells: &[StyledCell]) -> u16 {
    cells
        .iter()
        .fold(0u16, |width, cell| width.saturating_add(cell.width))
}

pub(crate) fn clip_cells(cells: Vec<StyledCell>, width: u16) -> Vec<StyledCell> {
    let mut used = 0u16;
    cells
        .into_iter()
        .take_while(|cell| {
            let fits = used.saturating_add(cell.width) <= width;
            if fits {
                used = used.saturating_add(cell.width);
            }
            fits
        })
        .collect()
}

pub(crate) fn row_from_cells(id: &str, cells: Vec<StyledCell>, width: u16) -> VisualRow {
    VisualRow {
        component_id: id.to_owned(),
        logical_line: 0,
        wrap_index: 0,
        cells: clip_cells(cells, width.max(1)),
    }
}

pub(crate) fn indent_styled_rows(
    rows: Vec<VisualRow>,
    prefix: &str,
    style: CellStyle,
) -> Vec<VisualRow> {
    rows.into_iter()
        .map(|mut row| {
            let mut cells = styled_cells(prefix, style);
            cells.extend(row.cells);
            row.cells = cells;
            row
        })
        .collect()
}
pub(crate) fn single_line_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}
pub(crate) fn single_line_row(id: &str, text: &str, style: CellStyle, width: u16) -> VisualRow {
    wrap_text(
        id,
        &truncate(&single_line_text(text), usize::from(width.max(1))),
        width.max(1),
        style,
    )
    .into_iter()
    .next()
    .unwrap_or_else(|| VisualRow::blank(id))
}

pub(crate) fn render_turn_separator(
    id: &str,
    separator: &TurnSeparator,
    width: u16,
) -> Vec<VisualRow> {
    let approximate = if separator.estimated { "~" } else { "" };
    let label = format!(
        " Worked for {approximate}{} ─",
        format_turn_duration(separator.duration_ms)
    );
    let available = usize::from(width.max(1));
    let text = if display_width(&label) >= available {
        truncate(label.trim_start(), available)
    } else {
        format!(
            "{}{label}",
            "─".repeat(available.saturating_sub(display_width(&label)))
        )
    };
    vec![single_line_row(
        id,
        &text,
        CellStyle::foreground(palette::GRAY_FAINT),
        width,
    )]
}

pub(crate) fn format_turn_duration(duration_ms: u64) -> String {
    if duration_ms < 1_000 {
        return "<1s".to_owned();
    }
    let seconds = duration_ms / 1_000;
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m {:02}s", seconds % 60);
    }
    format!("{}h {:02}m", minutes / 60, minutes % 60)
}
