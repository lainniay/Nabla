use std::collections::HashMap;

use crate::state::TranscriptItem;

use view_model::SceneViewModel;

use super::{
    layout::{LayoutEngine, LayoutRequest},
    store::UiState,
    text::{GraphemeIndex, cursor_geometry, display_width, truncate, wrap_text},
    types::{
        CellStyle, Color, ComponentId, CursorPosition, HitRegion, HitTarget, MainLayout,
        PanelFrame, Rect, RowRange, StyledCell, SurfaceKind, VisualFrame, VisualRow,
    },
};

pub(crate) const COMPOSER_CHROME_HEIGHT: u16 = 2;
pub(crate) const MAX_COMPOSER_CONTENT_HEIGHT: u16 = 6;

#[derive(Debug, Clone, Copy, Default)]
pub struct SceneBuilder;

pub fn animation_active(view: &SceneViewModel) -> bool {
    view.run_state.is_busy()
        || view.transcript.iter().any(|item| {
            matches!(
                item,
                TranscriptItem::Tool(tool)
                    if matches!(
                        tool.status,
                        crate::state::ToolStatus::Running
                            | crate::state::ToolStatus::WaitingApproval
                    )
            )
        })
}

impl SceneBuilder {
    pub fn build(self, view: &SceneViewModel, ui: &UiState, surface: SurfaceKind) -> VisualFrame {
        match surface {
            SurfaceKind::Primary => self.build_primary(view, ui, None),
            SurfaceKind::Alternate => self.build_alternate(view, ui),
        }
    }

    pub fn build_with_projection(
        self,
        view: &SceneViewModel,
        ui: &UiState,
        surface: SurfaceKind,
        projection: &super::types::PrimaryTranscriptProjection,
    ) -> VisualFrame {
        match surface {
            SurfaceKind::Primary => self.build_primary(view, ui, Some(projection)),
            SurfaceKind::Alternate => self.build_alternate(view, ui),
        }
    }

    fn build_primary(
        self,
        view: &SceneViewModel,
        ui: &UiState,
        projection: Option<&super::types::PrimaryTranscriptProjection>,
    ) -> VisualFrame {
        let size = ui.terminal.size;
        let composer_width = composer_content_width(size.width);
        let (cursor_row, _) = cursor_geometry(
            view.editor.text(),
            GraphemeIndex(view.editor.cursor()),
            composer_width,
        );
        let editor_height = wrap_text(
            "composer-measure",
            view.editor.text(),
            composer_width,
            CellStyle::default(),
        )
        .len()
        .max(cursor_row.saturating_add(1));
        let input_height = u16::try_from(editor_height)
            .unwrap_or(MAX_COMPOSER_CONTENT_HEIGHT)
            .clamp(1, MAX_COMPOSER_CONTENT_HEIGHT)
            .saturating_add(COMPOSER_CHROME_HEIGHT);
        let panel_request = primary_panel_request(view, size.width);
        let mut layout = LayoutEngine.layout(
            size,
            LayoutRequest {
                composer_height: input_height.saturating_add(1),
                status_height: 1,
                panel_height: panel_request.as_ref().map(|panel| panel.height),
            },
        );
        // Keep one owned blank row between transcript output and the composer
        // without letting it enter native scrollback. A panel consumes that
        // row so it sits flush against the composer without moving the input.
        if layout.composer.height > 1 {
            layout.composer.y = layout.composer.y.saturating_add(1);
            layout.composer.height = layout.composer.height.saturating_sub(1);
            if let Some(panel) = layout.panel.as_mut() {
                panel.y = panel.y.saturating_add(1);
            }
        }

        let mut canvas = Canvas::new(ui.revision, size, layout);
        let owned_projection;
        let projection = if let Some(projection) = projection {
            projection
        } else {
            owned_projection = ui.transcript.project_primary(
                layout.history_window.width,
                usize::from(layout.history_window.height),
                ui.revision,
                usize::MAX,
                usize::MAX,
                ui.animation_frame,
            );
            &owned_projection
        };
        debug_assert_eq!(
            projection.resident_capacity,
            usize::from(layout.history_window.height)
        );
        canvas.place_tail(layout.history_window, &projection.resident_rows);

        canvas.panel = layout
            .panel
            .zip(panel_request)
            .map(|(area, request)| request.render(area));

        let composer = composer_rows(
            view,
            layout.composer.width,
            layout.composer.height,
            cursor_row,
        );
        canvas.place(layout.composer, &composer.rows);
        canvas.hit_regions.push(HitRegion {
            area: layout.composer,
            target: HitTarget::Composer,
        });
        let (editor_row, editor_column) = cursor_geometry(
            view.editor.text(),
            GraphemeIndex(view.editor.cursor()),
            composer_content_width(layout.composer.width),
        );
        let visible_cursor_row = editor_row.saturating_sub(composer.first_content_row);
        canvas.cursor = Some(CursorPosition {
            column: u16::try_from(editor_column)
                .unwrap_or(u16::MAX)
                .saturating_add(composer.content_column)
                .min(size.width.saturating_sub(1)),
            row: layout
                .composer
                .y
                .saturating_add(composer.content_row)
                .saturating_add(u16::try_from(visible_cursor_row).unwrap_or(0))
                .min(layout.composer.bottom().saturating_sub(1))
                .min(size.height.saturating_sub(1)),
        });
        canvas.place(
            layout.status,
            &[status_row(view, size.width, ui.animation_frame)],
        );
        canvas.viewport = layout.owned_surface;
        canvas.finish()
    }

    fn build_alternate(self, view: &SceneViewModel, ui: &UiState) -> VisualFrame {
        let size = ui.terminal.size;
        let status_height = u16::from(size.height > 0);
        let composer_height = 3.min(size.height.saturating_sub(status_height));
        let transcript_height = size
            .height
            .saturating_sub(status_height)
            .saturating_sub(composer_height)
            .saturating_sub(u16::from(size.height > status_height + composer_height));
        let composer_y = transcript_height
            .saturating_add(u16::from(size.height > status_height + composer_height));
        let layout = MainLayout {
            transcript: Rect::new(0, 0, size.width, transcript_height),
            history_window: Rect::new(0, 0, size.width, transcript_height),
            owned_surface: Rect::new(0, 0, size.width, size.height),
            panel: None,
            composer: Rect::new(0, composer_y, size.width, composer_height),
            status: Rect::new(0, size.height.saturating_sub(1), size.width, 1),
        };
        let mut canvas = Canvas::new(ui.revision, size, layout);
        let rows = alternate_rows(view, ui, size.width, layout.transcript.height);
        canvas.place(layout.transcript, &rows);
        let input = alternate_input_model(view);
        let cursor_text = input.display_text();
        let (cursor_row, cursor_column) = cursor_geometry(
            &cursor_text,
            GraphemeIndex(input.cursor),
            composer_content_width(layout.composer.width),
        );
        let composer = alternate_composer_rows(
            &input,
            layout.composer.width,
            layout.composer.height,
            cursor_row,
        );
        canvas.place(layout.composer, &composer.rows);
        if input.focused && layout.composer.height > 0 {
            let visible_cursor_row = cursor_row.saturating_sub(composer.first_content_row);
            canvas.cursor = Some(CursorPosition {
                column: u16::try_from(cursor_column)
                    .unwrap_or(u16::MAX)
                    .saturating_add(composer.content_column)
                    .min(size.width.saturating_sub(1)),
                row: layout
                    .composer
                    .y
                    .saturating_add(composer.content_row)
                    .saturating_add(u16::try_from(visible_cursor_row).unwrap_or(0))
                    .min(layout.composer.bottom().saturating_sub(1))
                    .min(size.height.saturating_sub(1)),
            });
        }
        let status = alternate_status(view, input.focused);
        canvas.place(
            layout.status,
            &[text_row(
                "alternate-status",
                status,
                CellStyle::foreground(Color::Gray).dim(),
                size.width,
            )],
        );
        canvas.finish()
    }
}

struct Canvas {
    revision: u64,
    size: super::types::TerminalSize,
    rows: Vec<VisualRow>,
    bounds: HashMap<ComponentId, RowRange>,
    hit_regions: Vec<HitRegion>,
    cursor: Option<CursorPosition>,
    layout: MainLayout,
    viewport: Rect,
    panel: Option<PanelFrame>,
}

impl Canvas {
    fn new(revision: u64, size: super::types::TerminalSize, layout: MainLayout) -> Self {
        Self {
            revision,
            size,
            rows: (0..size.height)
                .map(|_| VisualRow::blank("surface"))
                .collect(),
            bounds: HashMap::new(),
            hit_regions: Vec::new(),
            cursor: None,
            layout,
            viewport: Rect::new(0, 0, size.width, size.height),
            panel: None,
        }
    }

    fn place(&mut self, area: Rect, source: &[VisualRow]) {
        for (offset, row) in source.iter().take(usize::from(area.height)).enumerate() {
            let terminal_row = usize::from(area.y).saturating_add(offset);
            if terminal_row >= self.rows.len() {
                break;
            }
            let row = clip_row(row, area.width);
            self.extend_bound(&row.component_id, terminal_row);
            self.rows[terminal_row] = row;
        }
    }

    fn place_tail(&mut self, area: Rect, source: &[VisualRow]) -> Option<Rect> {
        let visible_rows = source.len().min(usize::from(area.height));
        if visible_rows == 0 {
            return None;
        }
        let start = source.len().saturating_sub(usize::from(area.height));
        let height = u16::try_from(visible_rows).unwrap_or(area.height);
        let target = Rect::new(
            area.x,
            area.bottom().saturating_sub(height),
            area.width,
            height,
        );
        self.place(target, &source[start..]);
        Some(target)
    }

    fn extend_bound(&mut self, component_id: &str, row: usize) {
        self.bounds
            .entry(component_id.to_owned())
            .and_modify(|range| range.end = row.saturating_add(1))
            .or_insert(RowRange {
                start: row,
                end: row.saturating_add(1),
            });
    }

    fn finish(self) -> VisualFrame {
        VisualFrame {
            revision: self.revision,
            terminal_size: self.size,
            rows: self.rows,
            panel: self.panel,
            viewport: self.viewport,
            component_bounds: self.bounds,
            hit_regions: self.hit_regions,
            cursor: self.cursor,
            main_layout: self.layout,
        }
    }
}

fn clip_row(row: &VisualRow, width: u16) -> VisualRow {
    let mut result = row.clone();
    let mut used = 0u16;
    result.cells.retain(|cell| {
        if used.saturating_add(cell.width) > width {
            return false;
        }
        used = used.saturating_add(cell.width);
        true
    });
    result
}

pub(crate) fn text_row(id: &str, text: &str, style: CellStyle, width: u16) -> VisualRow {
    wrap_text(id, text, width.max(1), style)
        .into_iter()
        .next()
        .unwrap_or_else(|| VisualRow::blank(id))
}

pub mod composer;
pub mod panels;
pub mod view_model;

#[cfg(test)]
mod tests;

pub(crate) use composer::{
    alternate_composer_rows, alternate_input_model, alternate_status, composer_content_width,
    composer_rows,
};
pub(crate) use panels::{alternate_rows, primary_panel_request};
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

pub(crate) fn cells_width(cells: &[StyledCell]) -> u16 {
    cells
        .iter()
        .fold(0u16, |total, cell| total.saturating_add(cell.width))
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

fn status_row(view: &SceneViewModel, width: u16, animation_frame: u8) -> VisualRow {
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
    if *view.connection_state == crate::state::ConnectionState::Disconnected {
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
