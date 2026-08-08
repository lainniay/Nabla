use crate::state::TurnSeparator;
use crate::ui::{
    palette,
    text::{truncate, wrap_text},
    types::{CellStyle, StyledCell, VisualRow},
};

pub(crate) use crate::ui::text::{
    cells_width, clip_cells, display_width, styled_cells, take_graphemes_by_width,
};

pub(crate) fn row_from_cells(id: &str, cells: Vec<StyledCell>, width: u16) -> VisualRow {
    VisualRow {
        component_id: id.to_owned(),
        logical_line: 0,
        wrap_index: 0,
        cells: clip_cells(cells, width.max(1)),
    }
}

/// Like [`crate::ui::text::wrap_styled_lines`] but also breaks cells that are
/// wider than the target width so long unbroken tokens (paths, test names)
/// wrap instead of being clipped away.
pub(crate) fn wrap_styled_breaking(
    component_id: &str,
    logical_lines: &[Vec<StyledCell>],
    width: u16,
) -> Vec<VisualRow> {
    let width = usize::from(width.max(1));
    let mut rows = Vec::new();
    for (logical_line, line) in logical_lines.iter().enumerate() {
        if line.is_empty() {
            rows.push(VisualRow {
                component_id: component_id.to_owned(),
                logical_line,
                wrap_index: 0,
                cells: Vec::new(),
            });
            continue;
        }
        let mut current = Vec::new();
        let mut current_width = 0usize;
        let mut wrap_index = 0usize;
        for cell in line {
            let mut remaining = cell.symbol.as_str();
            while !remaining.is_empty() {
                if current_width >= width {
                    rows.push(VisualRow {
                        component_id: component_id.to_owned(),
                        logical_line,
                        wrap_index,
                        cells: std::mem::take(&mut current),
                    });
                    current_width = 0;
                    wrap_index += 1;
                }
                let available = width - current_width;
                let (take, rest) = take_graphemes_by_width(remaining, available);
                let take_width = display_width(take);
                current.push(StyledCell::new(take, take_width as u16, cell.style));
                current_width = current_width.saturating_add(take_width);
                remaining = rest;
            }
        }
        if !current.is_empty() {
            rows.push(VisualRow {
                component_id: component_id.to_owned(),
                logical_line,
                wrap_index,
                cells: current,
            });
        }
    }
    if rows.is_empty() {
        rows.push(VisualRow::blank(component_id));
    }
    rows
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
