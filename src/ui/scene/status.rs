use crate::state::ConnectionState;
use crate::ui::{
    text::{display_width, truncate},
    types::{CellStyle, Color, StyledCell, VisualRow},
};

use super::{append_text_cells, view_model::SceneViewModel};

pub(crate) fn status_row(view: &SceneViewModel, width: u16, animation_frame: u8) -> VisualRow {
    let context = view
        .context
        .actual_percent
        .map(|percent| format!("ctx {percent:.0}%"))
        .unwrap_or_else(|| "ctx —".to_owned());
    let left = format!(
        "{} · thinking {}",
        view.model_label(),
        view.session.thinking_level
    );
    let mut right_parts = Vec::new();
    if view.run_state.is_busy() {
        right_parts.push(spinner(animation_frame).to_owned());
    }
    if *view.connection_state == ConnectionState::Disconnected {
        right_parts.push("disconnected".to_owned());
    }
    right_parts.push(context);
    if *view.plan_mode_active {
        right_parts.push("PLAN".to_owned());
    }
    match view.sandbox_status.mode.as_str() {
        "enforced" => right_parts.push("sandbox".to_owned()),
        "degraded" => right_parts.push("sandbox:degraded".to_owned()),
        _ => right_parts.push("sandbox:off".to_owned()),
    }
    let right = right_parts.join(" · ");
    let left_width = display_width(&left);
    let right_width = display_width(&right);
    let margin = 1usize;
    let available = usize::from(width).saturating_sub(margin * 2);
    let muted = CellStyle::foreground(Color::Gray).dim();
    let mut cells = vec![StyledCell::new(" ".repeat(margin), margin as u16, muted)];
    if left_width.saturating_add(right_width).saturating_add(2) <= available {
        append_text_cells(&mut cells, &left, CellStyle::foreground(Color::Cyan));
        let padding = available.saturating_sub(left_width + right_width);
        cells.push(StyledCell::new(" ".repeat(padding), padding as u16, muted));
        append_text_cells(&mut cells, &right, muted);
    } else {
        let compact = truncate(&format!("{left} · {right}"), available);
        append_text_cells(&mut cells, &compact, muted);
        let padding = available.saturating_sub(display_width(&compact));
        if padding > 0 {
            cells.push(StyledCell::new(" ".repeat(padding), padding as u16, muted));
        }
    }
    cells.push(StyledCell::new(" ".repeat(margin), margin as u16, muted));
    VisualRow {
        component_id: "status".to_owned(),
        logical_line: 0,
        wrap_index: 0,
        cells,
    }
}

fn spinner(frame: u8) -> &'static str {
    const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    FRAMES[usize::from(frame) % FRAMES.len()]
}
