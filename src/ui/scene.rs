use std::{
    collections::HashMap,
    path::{Component, Path},
};
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    command::COMMAND_MENU_VISIBLE_ROWS,
    host::ApprovalDecision,
    state::{
        AppState, AuthPromptKind, AuthState, GrantProposal, TranscriptItem, TranscriptViewMode,
        TreeItem, TreePhase, UiModalKind, matching_auth_choice_indices,
    },
};

use super::{
    layout::{LayoutEngine, LayoutRequest},
    palette,
    panel::PanelRequest,
    selector::VirtualList,
    store::UiState,
    text::{
        GraphemeIndex, cursor_geometry, display_width, truncate, wrap_file_references, wrap_text,
    },
    transcript::{render_viewer_item, tool_operation_summary},
    types::{
        CellStyle, Color, ComponentId, CursorPosition, HitRegion, HitTarget, MainLayout,
        PanelFrame, Rect, RowRange, StyledCell, SurfaceKind, VisualFrame, VisualRow,
    },
};

const COMPOSER_CHROME_HEIGHT: u16 = 2;
const MAX_COMPOSER_CONTENT_HEIGHT: u16 = 6;

#[derive(Debug, Clone, Copy, Default)]
pub struct SceneBuilder;

pub fn animation_active(state: &AppState) -> bool {
    state.run_state.is_busy()
        || state.transcript.iter().any(|item| {
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
    pub fn build(self, domain: &AppState, ui: &UiState, surface: SurfaceKind) -> VisualFrame {
        match surface {
            SurfaceKind::Primary => self.build_primary(domain, ui, None),
            SurfaceKind::Alternate => self.build_alternate(domain, ui),
        }
    }

    pub fn build_with_projection(
        self,
        domain: &AppState,
        ui: &UiState,
        surface: SurfaceKind,
        projection: &super::types::PrimaryTranscriptProjection,
    ) -> VisualFrame {
        match surface {
            SurfaceKind::Primary => self.build_primary(domain, ui, Some(projection)),
            SurfaceKind::Alternate => self.build_alternate(domain, ui),
        }
    }

    fn build_primary(
        self,
        domain: &AppState,
        ui: &UiState,
        projection: Option<&super::types::PrimaryTranscriptProjection>,
    ) -> VisualFrame {
        let size = ui.terminal.size;
        let composer_width = composer_content_width(size.width);
        let (cursor_row, _) = cursor_geometry(
            domain.editor.text(),
            GraphemeIndex(domain.editor.cursor()),
            composer_width,
        );
        let editor_height = wrap_text(
            "composer-measure",
            domain.editor.text(),
            composer_width,
            CellStyle::default(),
        )
        .len()
        .max(cursor_row.saturating_add(1));
        let input_height = u16::try_from(editor_height)
            .unwrap_or(MAX_COMPOSER_CONTENT_HEIGHT)
            .clamp(1, MAX_COMPOSER_CONTENT_HEIGHT)
            .saturating_add(COMPOSER_CHROME_HEIGHT);
        let panel_request = primary_panel_request(domain, size.width);
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
            domain,
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
            domain.editor.text(),
            GraphemeIndex(domain.editor.cursor()),
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
            &[status_row(domain, size.width, ui.animation_frame)],
        );
        canvas.viewport = layout.owned_surface;
        canvas.finish()
    }

    fn build_alternate(self, domain: &AppState, ui: &UiState) -> VisualFrame {
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
        let rows = alternate_rows(domain, ui, size.width, layout.transcript.height);
        canvas.place(layout.transcript, &rows);
        let input = alternate_input_model(domain);
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
        let status = alternate_status(domain, input.focused);
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

fn text_row(id: &str, text: &str, style: CellStyle, width: u16) -> VisualRow {
    wrap_text(id, text, width.max(1), style)
        .into_iter()
        .next()
        .unwrap_or_else(|| VisualRow::blank(id))
}

struct ComposerRender {
    rows: Vec<VisualRow>,
    first_content_row: usize,
    content_column: u16,
    content_row: u16,
}

fn composer_content_width(width: u16) -> u16 {
    width.saturating_sub(4).max(1)
}

fn composer_rows(state: &AppState, width: u16, height: u16, cursor_row: usize) -> ComposerRender {
    let accent = if state.plan_mode_active {
        CellStyle::foreground(Color::Magenta)
    } else {
        palette::input_border()
    };
    if height < 3 || width < 4 {
        let mut rows = wrap_file_references(
            "composer",
            state.editor.text(),
            width.saturating_sub(2).max(1),
            CellStyle::foreground(palette::TEXT),
        );
        rows.truncate(usize::from(height.max(1)));
        for row in &mut rows {
            let mut cells = vec![
                StyledCell::new("›", 1, accent.bold()),
                StyledCell::new(" ", 1, accent),
            ];
            cells.extend(std::mem::take(&mut row.cells));
            row.cells = cells;
        }
        return ComposerRender {
            rows,
            first_content_row: 0,
            content_column: 2,
            content_row: 0,
        };
    }

    let border = if state.plan_mode_active {
        CellStyle::foreground(Color::Magenta).bold()
    } else {
        palette::input_border()
    };
    let text_style = CellStyle::foreground(palette::TEXT);
    let content_width = composer_content_width(width);
    let mut content =
        wrap_file_references("composer", state.editor.text(), content_width, text_style);
    while content.len() <= cursor_row {
        content.push(VisualRow::blank("composer"));
    }
    let content_capacity = usize::from(height.saturating_sub(COMPOSER_CHROME_HEIGHT).max(1));
    let maximum_start = content.len().saturating_sub(content_capacity);
    let first_content_row = cursor_row
        .saturating_sub(content_capacity.saturating_sub(1))
        .min(maximum_start);
    let visible_content =
        &content[first_content_row..content.len().min(first_content_row + content_capacity)];

    let mut rows = Vec::with_capacity(visible_content.len().saturating_add(2));
    rows.push(composer_border_row("╭", "╮", width, border));
    for (visible_index, row) in visible_content.iter().enumerate() {
        let mut cells = vec![StyledCell::new("│", 1, border)];
        if first_content_row + visible_index == 0 {
            cells.push(StyledCell::new("›", 1, accent.bold()));
            cells.push(StyledCell::new(" ", 1, accent));
        } else {
            cells.push(StyledCell::new("  ", 2, text_style));
        }

        if state.editor.text().is_empty() && first_content_row + visible_index == 0 {
            let placeholder = if state.plan_mode_active {
                "Describe the plan you want Nabla to make"
            } else {
                "Ask Nabla to work on your code"
            };
            append_text_cells(
                &mut cells,
                &truncate(placeholder, usize::from(content_width)),
                CellStyle::foreground(palette::GRAY_MUTED),
            );
        } else {
            cells.extend(row.cells.clone());
        }
        let used = cells
            .iter()
            .fold(0u16, |total, cell| total.saturating_add(cell.width));
        let padding = width.saturating_sub(used).saturating_sub(1);
        if padding > 0 {
            cells.push(StyledCell::new(
                " ".repeat(usize::from(padding)),
                padding,
                text_style,
            ));
        }
        cells.push(StyledCell::new("│", 1, border));
        rows.push(VisualRow {
            component_id: "composer".to_owned(),
            logical_line: first_content_row + visible_index,
            wrap_index: row.wrap_index,
            cells,
        });
    }
    rows.push(composer_border_row("╰", "╯", width, border));
    ComposerRender {
        rows,
        first_content_row,
        content_column: 3,
        content_row: 1,
    }
}

struct AlternateInputModel {
    text: String,
    cursor: usize,
    placeholder: String,
    focused: bool,
    secret: bool,
}

impl AlternateInputModel {
    fn display_text(&self) -> String {
        if self.secret {
            "•".repeat(self.text.graphemes(true).count())
        } else {
            self.text.clone()
        }
    }
}

fn alternate_input_model(state: &AppState) -> AlternateInputModel {
    match state.active_modal_kind() {
        Some(UiModalKind::SessionBrowser) => state.session_browser.as_ref().map_or_else(
            || alternate_placeholder("Search sessions", false),
            |browser| AlternateInputModel {
                text: browser.query.text().to_owned(),
                cursor: browser.query.cursor(),
                placeholder: "Search sessions".to_owned(),
                focused: browser.search_active,
                secret: false,
            },
        ),
        Some(UiModalKind::TreeBrowser) => state.tree_browser.as_ref().map_or_else(
            || alternate_placeholder("Search tree", false),
            |browser| match &browser.phase {
                TreePhase::EditLabel { editor, .. } => AlternateInputModel {
                    text: editor.text().to_owned(),
                    cursor: editor.cursor(),
                    placeholder: "Edit branch label".to_owned(),
                    focused: true,
                    secret: false,
                },
                TreePhase::CustomSummary { editor, .. } => AlternateInputModel {
                    text: editor.text().to_owned(),
                    cursor: editor.cursor(),
                    placeholder: "Describe the branch summary".to_owned(),
                    focused: true,
                    secret: false,
                },
                _ => AlternateInputModel {
                    text: browser.query.text().to_owned(),
                    cursor: browser.query.cursor(),
                    placeholder: "Search tree".to_owned(),
                    focused: browser.search_active,
                    secret: false,
                },
            },
        ),
        Some(UiModalKind::Transcript) => state.transcript_viewer.as_ref().map_or_else(
            || alternate_placeholder("Search transcript", false),
            |viewer| AlternateInputModel {
                text: viewer.search_query.text().to_owned(),
                cursor: viewer.search_query.cursor(),
                placeholder: "Search transcript".to_owned(),
                focused: viewer.search_active,
                secret: false,
            },
        ),
        Some(UiModalKind::Auth) => match &state.auth_state {
            AuthState::Selecting {
                filter,
                search_active,
                ..
            } => AlternateInputModel {
                text: filter.text().to_owned(),
                cursor: filter.cursor(),
                placeholder: "Search login providers".to_owned(),
                focused: *search_active,
                secret: false,
            },
            AuthState::Running(flow) => flow.prompt.as_ref().map_or_else(
                || alternate_placeholder(&flow.status, false),
                |prompt| {
                    if prompt.kind == AuthPromptKind::Select {
                        alternate_placeholder("Select an authentication option", false)
                    } else {
                        AlternateInputModel {
                            text: prompt.editor.text().to_owned(),
                            cursor: prompt.editor.cursor(),
                            placeholder: prompt
                                .placeholder
                                .clone()
                                .unwrap_or_else(|| prompt.message.clone()),
                            focused: true,
                            secret: prompt.kind == AuthPromptKind::Secret,
                        }
                    }
                },
            ),
            AuthState::LoadingProviders => alternate_placeholder("Loading providers…", false),
            AuthState::Inactive => alternate_placeholder("Authentication", false),
        },
        _ => alternate_placeholder("Search", false),
    }
}

fn alternate_placeholder(placeholder: &str, focused: bool) -> AlternateInputModel {
    AlternateInputModel {
        text: String::new(),
        cursor: 0,
        placeholder: placeholder.to_owned(),
        focused,
        secret: false,
    }
}

fn alternate_status(state: &AppState, input_focused: bool) -> &'static str {
    if let Some(browser) = state.tree_browser.as_ref() {
        return match &browser.phase {
            TreePhase::EditLabel { .. } => "Esc cancel · Enter save label · ←→ edit",
            TreePhase::CustomSummary { .. } => "Esc back · Enter navigate · ←→ edit",
            TreePhase::Navigating {
                summarizing: true,
                aborting: false,
                ..
            } => "Esc cancel navigation",
            TreePhase::Navigating { .. } => "Navigation in progress",
            _ if input_focused => "Esc clear · Enter return to tree · ←→ edit",
            _ => "/ search · Esc close · Tab/⇧Tab/Ctrl+N/P select · Enter",
        };
    }
    if let AuthState::Running(flow) = &state.auth_state
        && flow
            .prompt
            .as_ref()
            .is_some_and(|prompt| prompt.kind != AuthPromptKind::Select)
    {
        return "Esc cancel login · Enter submit · ←→ edit";
    }
    if input_focused {
        return "Esc clear · Enter return to list · ←→ edit";
    }
    if state.transcript_viewer.is_some() {
        "/ search · Esc close · Tab/⇧Tab tools · Enter expand · ↑↓/PgUp/PgDn scroll"
    } else {
        "/ search · Esc close · Tab/⇧Tab/Ctrl+N/P select · Enter"
    }
}

fn alternate_composer_rows(
    input: &AlternateInputModel,
    width: u16,
    height: u16,
    cursor_row: usize,
) -> ComposerRender {
    let display_text = input.display_text();
    let border = palette::input_border();
    if height < 3 || width < 4 {
        let visible = if display_text.is_empty() {
            input.placeholder.as_str()
        } else {
            display_text.as_str()
        };
        let mut row = text_row(
            "alternate-input",
            visible,
            if display_text.is_empty() {
                CellStyle::foreground(palette::GRAY_MUTED)
            } else {
                CellStyle::foreground(palette::TEXT)
            },
            width.saturating_sub(2).max(1),
        );
        let mut cells = vec![
            StyledCell::new("›", 1, border),
            StyledCell::new(" ", 1, border),
        ];
        cells.extend(row.cells);
        row.cells = cells;
        return ComposerRender {
            rows: vec![row],
            first_content_row: 0,
            content_column: 2,
            content_row: 0,
        };
    }

    let content_width = composer_content_width(width);
    let text_style = CellStyle::foreground(palette::TEXT);
    let mut content = wrap_text("alternate-input", &display_text, content_width, text_style);
    while content.len() <= cursor_row {
        content.push(VisualRow::blank("alternate-input"));
    }
    let capacity = usize::from(height.saturating_sub(COMPOSER_CHROME_HEIGHT).max(1));
    let maximum_start = content.len().saturating_sub(capacity);
    let first_content_row = cursor_row
        .saturating_sub(capacity.saturating_sub(1))
        .min(maximum_start);
    let visible = &content[first_content_row..content.len().min(first_content_row + capacity)];
    let mut rows = vec![input_border_row("alternate-input", "╭", "╮", width, border)];
    for (visible_index, row) in visible.iter().enumerate() {
        let mut cells = vec![
            StyledCell::new("│", 1, border),
            StyledCell::new(
                if first_content_row + visible_index == 0 {
                    "›"
                } else {
                    " "
                },
                1,
                border,
            ),
            StyledCell::new(" ", 1, border),
        ];
        if display_text.is_empty() && first_content_row + visible_index == 0 {
            append_text_cells(
                &mut cells,
                &truncate(&input.placeholder, usize::from(content_width)),
                CellStyle::foreground(palette::GRAY_MUTED),
            );
        } else {
            cells.extend(row.cells.clone());
        }
        let used = cells_width(&cells);
        let padding = width.saturating_sub(used).saturating_sub(1);
        if padding > 0 {
            cells.push(StyledCell::new(
                " ".repeat(usize::from(padding)),
                padding,
                text_style,
            ));
        }
        cells.push(StyledCell::new("│", 1, border));
        rows.push(VisualRow {
            component_id: "alternate-input".to_owned(),
            logical_line: first_content_row + visible_index,
            wrap_index: row.wrap_index,
            cells,
        });
    }
    rows.push(input_border_row("alternate-input", "╰", "╯", width, border));
    ComposerRender {
        rows,
        first_content_row,
        content_column: 3,
        content_row: 1,
    }
}

fn input_border_row(id: &str, left: &str, right: &str, width: u16, style: CellStyle) -> VisualRow {
    let mut row = composer_border_row(left, right, width, style);
    row.component_id = id.to_owned();
    row
}

fn cells_width(cells: &[StyledCell]) -> u16 {
    cells
        .iter()
        .fold(0u16, |total, cell| total.saturating_add(cell.width))
}

fn composer_border_row(left: &str, right: &str, width: u16, style: CellStyle) -> VisualRow {
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

fn append_text_cells(cells: &mut Vec<StyledCell>, text: &str, style: CellStyle) {
    if let Some(row) = wrap_text("inline", text, u16::MAX, style)
        .into_iter()
        .next()
    {
        cells.extend(row.cells);
    }
}

fn status_row(state: &AppState, width: u16, animation_frame: u8) -> VisualRow {
    let context = state
        .context
        .actual_percent
        .map(|percent| format!("ctx {percent:.0}%"))
        .unwrap_or_else(|| "ctx —".to_owned());
    let left = format!(
        "{} · thinking {}",
        state.model_label(),
        state.session.thinking_level
    );
    let mut right_parts = Vec::new();
    if state.run_state.is_busy() {
        right_parts.push(spinner(animation_frame).to_owned());
    }
    if state.connection_state == crate::state::ConnectionState::Disconnected {
        right_parts.push("disconnected".to_owned());
    }
    right_parts.push(context);
    if state.plan_mode_active {
        right_parts.push("PLAN".to_owned());
    }
    let right = right_parts.join(" · ");
    let left_width = display_width(&left);
    let right_width = display_width(&right);
    let available = usize::from(width);
    let muted = CellStyle::foreground(Color::Gray).dim();
    let mut cells = Vec::new();
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

fn primary_panel_request(state: &AppState, width: u16) -> Option<PanelRequest> {
    let width = width.saturating_sub(2).max(1);
    match state.active_modal_kind() {
        None => {
            if let Some(completion) = state.file_completion.as_ref() {
                let rows = if let Some(error) = completion.error.as_ref() {
                    vec![text_row(
                        "file-panel",
                        error,
                        CellStyle::foreground(Color::Red),
                        width,
                    )]
                } else if completion.loading && completion.candidates.is_empty() {
                    vec![text_row(
                        "file-panel",
                        "Searching files…",
                        CellStyle::foreground(Color::Gray).dim(),
                        width,
                    )]
                } else {
                    completion
                        .candidates
                        .iter()
                        .enumerate()
                        .map(|(index, candidate)| {
                            panel_choice_row(
                                "file-panel",
                                &candidate.basename,
                                &candidate.parent,
                                index == completion.selected,
                                true,
                                width,
                            )
                        })
                        .collect()
                };
                let height = rows.len().min(COMMAND_MENU_VISIBLE_ROWS);
                return PanelRequest::new(rows, Some(completion.selected), height);
            }
            let rows = state
                .command_candidates()
                .iter()
                .enumerate()
                .map(|(index, command)| {
                    panel_choice_row(
                        "command-panel",
                        &format!("/{}", command.name),
                        &command.description,
                        index == state.command_menu_selected(),
                        true,
                        width,
                    )
                })
                .collect::<Vec<_>>();
            let height = rows.len().min(COMMAND_MENU_VISIBLE_ROWS);
            PanelRequest::new(rows, Some(state.command_menu_selected()), height)
        }
        Some(UiModalKind::Approval) => approval_panel_request(state.approval.as_ref()?, width),
        Some(UiModalKind::Permissions) => {
            let manager = state.permission_manager.as_ref()?;
            let mut rows = vec![
                text_row(
                    "permissions",
                    "Persistent Approvals",
                    CellStyle::foreground(palette::LAVENDER).bold(),
                    width,
                ),
                text_row(
                    "permissions",
                    "Current project · [D] revoke · [C] clear · Esc close",
                    CellStyle::foreground(Color::Gray).dim(),
                    width,
                ),
            ];
            if manager.snapshot.grants.is_empty() {
                rows.push(text_row(
                    "permissions",
                    "No persistent approvals",
                    CellStyle::foreground(palette::SUBTEXT_0),
                    width,
                ));
            } else {
                rows.extend(
                    manager
                        .snapshot
                        .grants
                        .iter()
                        .enumerate()
                        .map(|(index, grant)| {
                            panel_choice_row(
                                "permissions",
                                &grant.proposal.scope,
                                &grant_proposal_summary(&grant.proposal),
                                index == manager.selected,
                                true,
                                width,
                            )
                        }),
                );
            }
            let selected =
                (!manager.snapshot.grants.is_empty()).then_some(manager.selected.saturating_add(2));
            let height = rows.len().min(state.selection_page_size.saturating_add(2));
            PanelRequest::new(rows, selected, height)
        }
        Some(UiModalKind::Question) => {
            let flow = state.question.as_ref()?;
            let question = flow.current_question()?;
            let mut rows = vec![text_row(
                "question",
                &question.prompt,
                CellStyle::foreground(Color::Cyan).bold(),
                width,
            )];
            rows.extend(question.options.iter().enumerate().map(|(index, option)| {
                panel_choice_row(
                    "question",
                    &option.label,
                    option.description.as_deref().unwrap_or_default(),
                    index == flow.selected,
                    true,
                    width,
                )
            }));
            rows.push(panel_choice_row(
                "question",
                "Custom answer",
                "Type a different response",
                flow.selected == question.options.len(),
                true,
                width,
            ));
            if flow.custom_answer {
                rows.extend(wrap_text(
                    "question-input",
                    flow.editor.text(),
                    width,
                    CellStyle::foreground(Color::White),
                ));
            }
            let height = rows.len().min(state.selection_page_size.saturating_add(2));
            PanelRequest::new(rows, Some(flow.selected.saturating_add(1)), height)
        }
        Some(UiModalKind::Selection) => state.selection_panel.as_ref().and_then(|panel| {
            let mut rows = vec![text_row(
                "selection-panel",
                &panel.title,
                CellStyle::foreground(Color::Cyan).bold(),
                width,
            )];
            if panel.loading {
                rows.push(text_row(
                    "selection-panel",
                    "Loading…",
                    CellStyle::foreground(Color::Gray).dim(),
                    width,
                ));
            } else if panel.options.is_empty() {
                rows.push(text_row(
                    "selection-panel",
                    "No options available",
                    CellStyle::foreground(Color::Gray).dim(),
                    width,
                ));
            } else {
                rows.extend(panel.options.iter().enumerate().map(|(index, option)| {
                    panel_choice_row(
                        "selection-panel",
                        &option.label,
                        &option.description,
                        index == panel.selected,
                        true,
                        width,
                    )
                }));
            }
            let height = rows.len().min(state.selection_page_size.saturating_add(1));
            PanelRequest::new(rows, Some(panel.selected.saturating_add(1)), height)
        }),
        Some(UiModalKind::AgentPicker) => state.agent_picker.as_ref().and_then(|picker| {
            let rows = picker
                .profiles
                .iter()
                .enumerate()
                .map(|(index, profile)| {
                    panel_choice_row(
                        "agent-picker",
                        &profile.name,
                        &profile.description,
                        index == picker.selected,
                        true,
                        width,
                    )
                })
                .collect::<Vec<_>>();
            let height = rows.len().min(state.selection_page_size);
            PanelRequest::new(rows, Some(picker.selected), height)
        }),
        Some(UiModalKind::Integration) => state.integration_prompt.as_ref().and_then(|prompt| {
            let mut rows = vec![text_row(
                "integration",
                &format!("Integrate changes from {}?", prompt.agent.profile),
                CellStyle::foreground(Color::Yellow).bold(),
                width,
            )];
            for (index, (label, description, enabled)) in [
                ("Apply", "Apply changes automatically", true),
                (
                    "Resolve",
                    "Resolve conflicts interactively",
                    prompt.integration.resolver_available,
                ),
                ("Keep worktree", "Leave changes isolated", true),
                ("Discard", "Discard isolated changes", true),
            ]
            .iter()
            .enumerate()
            {
                rows.push(panel_choice_row(
                    "integration",
                    label,
                    description,
                    index == prompt.selected,
                    *enabled,
                    width,
                ));
            }
            let height = rows.len();
            PanelRequest::new(rows, Some(prompt.selected.saturating_add(1)), height)
        }),
        Some(UiModalKind::PlanReview) => state.plan_review.as_ref().and_then(|review| {
            let labels = ["Execute", "Fresh execute", "Close"];
            let descriptions = [
                "Continue in this conversation",
                "Start a new session with the Plan and handoff",
                "Keep the Plan without executing",
            ];
            let mut rows = vec![text_row(
                "plan-review",
                &state.context.remaining_percent().map_or_else(
                    || "Current context remaining: unknown".to_owned(),
                    |remaining| {
                        format!(
                            "Current context remaining: {:.0}% ({})",
                            remaining,
                            state.context.usage_state.label()
                        )
                    },
                ),
                CellStyle::foreground(Color::Gray),
                width,
            )];
            rows.extend(labels.iter().enumerate().map(|(index, label)| {
                panel_choice_row(
                    "plan-review",
                    label,
                    descriptions[index],
                    index == review.selected,
                    true,
                    width,
                )
            }));
            let height = rows.len();
            PanelRequest::new(rows, Some(review.selected.saturating_add(1)), height)
        }),
        _ => None,
    }
}

fn approval_panel_request(
    approval: &crate::state::ApprovalState,
    width: u16,
) -> Option<PanelRequest> {
    let risk = approval.risk.as_deref().unwrap_or("normal");
    let mut rows = vec![text_row(
        "approval",
        "Ask for Approval",
        CellStyle::foreground(palette::LAVENDER).bold(),
        width,
    )];
    rows.push(text_row(
        "approval-summary",
        approval_summary(approval),
        match risk {
            "high" | "credential" | "outside_workspace" => CellStyle::foreground(Color::Red),
            "elevated" => CellStyle::foreground(Color::Yellow),
            _ => CellStyle::foreground(palette::SUBTEXT_0),
        },
        width,
    ));
    if risk != "normal"
        && !approval.summary.is_empty()
        && approval.summary != approval_summary(approval)
    {
        rows.push(text_row(
            "approval-summary-detail",
            &approval.summary,
            CellStyle::foreground(palette::SUBTEXT_0),
            width,
        ));
    }
    rows.push(approval_operation_row(approval, width));

    if approval
        .available_decisions
        .contains(&ApprovalDecision::AllowSession)
        && let Some(proposal) = approval.session_grant.as_ref()
    {
        rows.push(text_row(
            "approval-session-grant",
            &format!("Session saves: {}", grant_proposal_summary(proposal)),
            CellStyle::foreground(palette::SUBTEXT_0).dim(),
            width,
        ));
    }
    if approval
        .available_decisions
        .contains(&ApprovalDecision::AllowWorkspace)
        && let Some(proposal) = approval.workspace_grant.as_ref()
    {
        rows.push(text_row(
            "approval-workspace-grant",
            &format!("Workspace saves: {}", grant_proposal_summary(proposal)),
            CellStyle::foreground(palette::SUBTEXT_0).dim(),
            width,
        ));
    }

    let actions = approval
        .available_decisions
        .iter()
        .map(|decision| match decision {
            ApprovalDecision::AllowOnce => (
                "[Y] Allow once".to_owned(),
                "Approve only this request".to_owned(),
            ),
            ApprovalDecision::AllowSession => (
                "[S] Allow for Session".to_owned(),
                approval.session_grant.as_ref().map_or_else(
                    || "No session grant was proposed".to_owned(),
                    grant_proposal_summary,
                ),
            ),
            ApprovalDecision::AllowWorkspace => (
                "[A] Allow for Workspace".to_owned(),
                approval.workspace_grant.as_ref().map_or_else(
                    || "No workspace grant was proposed".to_owned(),
                    grant_proposal_summary,
                ),
            ),
            ApprovalDecision::Deny => {
                ("[N] Deny".to_owned(), "Reject this tool request".to_owned())
            }
        })
        .collect::<Vec<_>>();
    let action_offset = rows.len();
    for (index, (label, description)) in actions.iter().enumerate() {
        rows.push(panel_choice_row(
            "approval",
            label,
            description,
            index == approval.selected,
            true,
            width,
        ));
    }
    let height = rows.len();
    PanelRequest::new(
        rows,
        Some(approval.selected.saturating_add(action_offset)),
        height,
    )
}

fn grant_proposal_summary(proposal: &GrantProposal) -> String {
    let mut parts = proposal
        .matchers
        .iter()
        .map(grant_matcher_summary)
        .collect::<Vec<_>>();
    parts.extend(proposal.invalidation_keys.iter().map(|key| {
        let kind = key
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("invalidation");
        let path = key
            .get("path")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if path.is_empty() {
            format!("{kind}={}", key["value"].as_str().unwrap_or("?"))
        } else {
            format!("{kind} {path}={}", key["value"].as_str().unwrap_or("?"))
        }
    }));
    parts.join("; ")
}

fn grant_matcher_summary(matcher: &serde_json::Value) -> String {
    match matcher.get("kind").and_then(serde_json::Value::as_str) {
        Some("exec") => {
            let executable = matcher["executable"].as_str().unwrap_or("?");
            let argv = matcher["argv"]
                .as_array()
                .map(|values| {
                    values
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            let cwd = matcher["cwd"].as_str().unwrap_or("?");
            format!("exec {executable} {argv} @ {cwd}")
        }
        Some("file") => format!(
            "{} {}",
            matcher["operation"].as_str().unwrap_or("file"),
            matcher["path"].as_str().unwrap_or("?")
        ),
        Some("opaque_code") => format!(
            "exact opaque {}:{}",
            matcher["runtime"].as_str().unwrap_or("?"),
            matcher["digest"].as_str().unwrap_or("?")
        ),
        Some(kind) => format!("{kind} {}", matcher),
        None => matcher.to_string(),
    }
}

fn approval_summary(approval: &crate::state::ApprovalState) -> &str {
    match approval.risk.as_deref().unwrap_or("normal") {
        "outside_workspace" => "Outside trusted project scope",
        "credential" => "May access sensitive credentials",
        "high" => "High-risk command",
        "elevated" => "Elevated operation",
        _ if approval.summary.is_empty() => "This action requires approval",
        _ => &approval.summary,
    }
}

fn approval_operation_row(approval: &crate::state::ApprovalState, width: u16) -> VisualRow {
    let normalized_name = approval.tool_name.to_ascii_lowercase();
    let operation = if let Some(command) = input_string(&approval.input, &["command", "cmd"]) {
        command.replace(['\r', '\n'], " ")
    } else if is_file_tool(&normalized_name) {
        let label = tool_operation_summary(&approval.tool_name, &approval.input)
            .split(" · ")
            .next()
            .unwrap_or("File")
            .to_owned();
        input_paths(&approval.input).first().map_or_else(
            || label.clone(),
            |path| format!("{label} {}", normalize_display_path(path)),
        )
    } else {
        tool_operation_summary(&approval.tool_name, &approval.input)
    };
    let available = usize::from(width.saturating_sub(4));
    text_row(
        "approval-input",
        &format!("    {}", truncate(&operation, available)),
        CellStyle::foreground(Color::Gray).dim(),
        width,
    )
}

fn input_string<'a>(input: &'a serde_json::Value, keys: &[&str]) -> Option<&'a str> {
    let object = input.as_object()?;
    keys.iter().find_map(|key| object.get(*key)?.as_str())
}

fn input_paths(input: &serde_json::Value) -> Vec<String> {
    let Some(object) = input.as_object() else {
        return Vec::new();
    };
    for key in ["path", "filePath", "file", "target"] {
        if let Some(path) = object.get(key).and_then(serde_json::Value::as_str) {
            return vec![path.to_owned()];
        }
    }
    object
        .get("paths")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .collect()
}

fn is_file_tool(name: &str) -> bool {
    ["read", "write", "edit", "patch", "file", "delete", "remove"]
        .iter()
        .any(|operation| name.contains(operation))
}

fn normalize_display_path(value: &str) -> String {
    let path = Path::new(value);
    let absolute = path.is_absolute();
    let mut components = Vec::<String>::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => {
                components.push(prefix.as_os_str().to_string_lossy().into_owned());
            }
            Component::RootDir | Component::CurDir => {}
            Component::ParentDir => {
                if components.last().is_some_and(|part| part != "..") {
                    components.pop();
                } else if !absolute {
                    components.push("..".to_owned());
                }
            }
            Component::Normal(part) => components.push(part.to_string_lossy().into_owned()),
        }
    }
    let body = components.join("/");
    if absolute {
        format!("/{body}")
    } else if body.is_empty() {
        ".".to_owned()
    } else {
        body
    }
}

fn panel_choice_row(
    id: &str,
    label: &str,
    description: &str,
    selected: bool,
    enabled: bool,
    width: u16,
) -> VisualRow {
    let label_style = if selected {
        palette::selected()
    } else if enabled {
        CellStyle::foreground(Color::White)
    } else {
        CellStyle::foreground(Color::Gray).dim()
    };
    let description_style = if selected {
        palette::selected_muted()
    } else {
        CellStyle::foreground(Color::Gray).dim()
    };
    aligned_panel_row(
        id,
        label,
        description,
        label_style,
        description_style,
        width,
    )
}

fn aligned_panel_row(
    id: &str,
    label: &str,
    description: &str,
    label_style: CellStyle,
    description_style: CellStyle,
    width: u16,
) -> VisualRow {
    let available = usize::from(width);
    let label = truncate(label, available);
    let label_width = display_width(&label);
    let description_budget = available.saturating_sub(label_width.saturating_add(1));
    let description = if description_budget >= 4 {
        truncate(description, description_budget)
    } else {
        String::new()
    };
    let description_width = display_width(&description);
    let padding = available.saturating_sub(label_width.saturating_add(description_width));
    let mut cells = Vec::new();
    append_text_cells(&mut cells, &label, label_style);
    if padding > 0 {
        cells.push(StyledCell::new(
            " ".repeat(padding),
            u16::try_from(padding).unwrap_or(width),
            label_style,
        ));
    }
    append_text_cells(&mut cells, &description, description_style);
    VisualRow {
        component_id: id.to_owned(),
        logical_line: 0,
        wrap_index: 0,
        cells,
    }
}

fn choice_row(id: &str, label: &str, description: &str, selected: bool, width: u16) -> VisualRow {
    aligned_panel_row(
        id,
        label,
        description,
        if selected {
            palette::selected()
        } else {
            CellStyle::foreground(Color::White)
        },
        if selected {
            palette::selected_muted()
        } else {
            CellStyle::foreground(Color::Gray).dim()
        },
        width,
    )
}

fn alternate_rows(state: &AppState, _ui: &UiState, width: u16, height: u16) -> Vec<VisualRow> {
    let mut rows = Vec::new();
    let title_style = CellStyle::foreground(Color::Magenta).bold();
    match state.active_modal_kind() {
        Some(UiModalKind::SessionBrowser) => {
            rows.push(text_row(
                "session-browser",
                "Resume session",
                title_style,
                width,
            ));
            if let Some(browser) = state.session_browser.as_ref() {
                rows.push(text_row(
                    "session-browser",
                    &format!(
                        "{} · {} results · {}",
                        browser.sort_mode.label(),
                        browser.total,
                        match browser.scope {
                            crate::state::SessionScope::Current => "current workspace",
                            crate::state::SessionScope::All => "all workspaces",
                        }
                    ),
                    CellStyle::foreground(Color::Gray).dim(),
                    width,
                ));
                let choices = browser
                    .sessions
                    .iter()
                    .enumerate()
                    .map(|(index, session)| {
                        let indent = if session.depth > 4 {
                            format!("… {}", "  ".repeat(2))
                        } else {
                            "  ".repeat(session.depth)
                        };
                        let description = if session.current {
                            format!("current · {} messages", session.message_count)
                        } else {
                            format!("{} messages", session.message_count)
                        };
                        choice_row(
                            "session-browser",
                            &format!("{indent}{}", session.label()),
                            &description,
                            index == browser.selected,
                            width,
                        )
                    })
                    .collect();
                append_choice_window(&mut rows, choices, browser.selected, height);
            }
        }
        Some(UiModalKind::TreeBrowser) => {
            rows.push(text_row(
                "tree-browser",
                "Session tree",
                CellStyle::foreground(palette::TEXT).bold(),
                width,
            ));
            if let Some(browser) = state.tree_browser.as_ref() {
                rows.push(text_row(
                    "tree-browser",
                    &format!(
                        "{} filter · {} entries",
                        browser.filter_mode.label(),
                        browser.items.len()
                    ),
                    CellStyle::foreground(Color::Gray).dim(),
                    width,
                ));
                match &browser.phase {
                    TreePhase::ChooseSummary { selected, .. } => {
                        rows.push(text_row(
                            "tree-browser",
                            "How should Nabla preserve the abandoned branch?",
                            CellStyle::foreground(Color::Yellow).bold(),
                            width,
                        ));
                        let choices = [
                            ("Navigate directly", "Do not create a branch summary"),
                            ("Generate summary", "Summarize the abandoned branch"),
                            ("Custom summary", "Provide summary instructions"),
                        ]
                        .iter()
                        .enumerate()
                        .map(|(index, (label, description))| {
                            choice_row(
                                "tree-browser",
                                label,
                                description,
                                index == *selected,
                                width,
                            )
                        })
                        .collect();
                        append_choice_window(&mut rows, choices, *selected, height);
                    }
                    TreePhase::Navigating {
                        summarizing,
                        aborting,
                        ..
                    } => rows.push(text_row(
                        "tree-browser",
                        if *aborting {
                            "Cancelling tree navigation…"
                        } else if *summarizing {
                            "Summarizing branch before navigation…"
                        } else {
                            "Navigating session tree…"
                        },
                        CellStyle::foreground(Color::Cyan),
                        width,
                    )),
                    _ => {
                        let choices = browser
                            .items
                            .iter()
                            .enumerate()
                            .map(|(index, item)| {
                                tree_choice_rows(item, index == browser.selected, width)
                            })
                            .collect();
                        append_tree_choice_window(&mut rows, choices, browser.selected, height);
                    }
                }
            }
        }
        Some(UiModalKind::Transcript) => {
            rows.push(text_row(
                "transcript-viewer",
                "Transcript viewer",
                title_style,
                width,
            ));
            if let Some(viewer) = state.transcript_viewer.as_ref() {
                rows.push(text_row(
                    "transcript-viewer",
                    &format!(
                        "{} · {} matches",
                        viewer.mode.label(),
                        viewer.search_matches.len()
                    ),
                    CellStyle::foreground(Color::Gray).dim(),
                    width,
                ));
                let mut body = Vec::new();
                let mut ranges = HashMap::<usize, RowRange>::new();
                for (index, item) in state.transcript.iter().enumerate() {
                    if viewer.mode != TranscriptViewMode::Summary
                        && (index == 0
                            || viewer_item_group(&state.transcript[index - 1])
                                != viewer_item_group(item))
                    {
                        body.push(VisualRow::blank("transcript-viewer-spacing"));
                    }
                    let start = body.len();
                    let expanded = match item {
                        TranscriptItem::Tool(tool) => viewer
                            .tool_expansion_overrides
                            .get(&tool.id)
                            .copied()
                            .unwrap_or(viewer.mode == TranscriptViewMode::Verbose),
                        _ => false,
                    };
                    body.extend(render_viewer_item(
                        &format!("viewer:{index}"),
                        item,
                        width,
                        viewer.mode,
                        expanded,
                        viewer.selected_item == Some(index),
                    ));
                    ranges.insert(
                        index,
                        RowRange {
                            start,
                            end: body.len(),
                        },
                    );
                }

                let body_height = usize::from(height).saturating_sub(rows.len());
                let maximum_start = body.len().saturating_sub(body_height);
                let start = if viewer.scroll_to_selected {
                    viewer
                        .selected_item
                        .and_then(|selected| ranges.get(&selected))
                        .map(|range| {
                            range
                                .start
                                .saturating_sub(body_height.saturating_sub(1) / 2)
                                .min(maximum_start)
                        })
                        .unwrap_or(maximum_start)
                } else if viewer.follow_tail {
                    maximum_start
                } else {
                    maximum_start.saturating_sub(viewer.scroll_from_bottom)
                };
                rows.extend(body.into_iter().skip(start).take(body_height));
            }
        }
        Some(UiModalKind::Auth) => {
            rows.push(text_row("auth", "Authentication", title_style, width));
            match &state.auth_state {
                AuthState::Inactive => {}
                AuthState::LoadingProviders => rows.push(text_row(
                    "auth",
                    "Loading providers…",
                    CellStyle::foreground(Color::Gray),
                    width,
                )),
                AuthState::Selecting {
                    choices,
                    selected,
                    filter,
                    ..
                } => {
                    rows.push(text_row(
                        "auth",
                        &format!(
                            "{} providers",
                            matching_auth_choice_indices(choices, filter.text()).len()
                        ),
                        CellStyle::foreground(Color::Gray).dim(),
                        width,
                    ));
                    let visible_choices = matching_auth_choice_indices(choices, filter.text());
                    let choice_rows = visible_choices
                        .into_iter()
                        .enumerate()
                        .map(|(visible_index, choice_index)| {
                            let choice = &choices[choice_index];
                            let description = format!(
                                "{} · {}{}",
                                choice.label,
                                choice.auth_type,
                                if choice.configured {
                                    " · configured"
                                } else {
                                    ""
                                }
                            );
                            choice_row(
                                "auth",
                                &choice.provider_name,
                                &description,
                                visible_index == *selected,
                                width,
                            )
                        })
                        .collect();
                    append_choice_window(&mut rows, choice_rows, *selected, height);
                }
                AuthState::Running(flow) => {
                    rows.push(text_row(
                        "auth",
                        &format!("{} · {}", flow.provider_name, flow.status),
                        CellStyle::foreground(Color::Cyan),
                        width,
                    ));
                    if let Some(code) = flow.device_code.as_ref() {
                        rows.push(text_row(
                            "auth",
                            &format!("Code: {code}"),
                            CellStyle::foreground(Color::Yellow).bold(),
                            width,
                        ));
                    }
                    if let Some(prompt) = flow.prompt.as_ref() {
                        rows.push(text_row(
                            "auth",
                            &prompt.message,
                            CellStyle::foreground(Color::White),
                            width,
                        ));
                        if prompt.kind == AuthPromptKind::Select {
                            let choice_rows = prompt
                                .options
                                .iter()
                                .enumerate()
                                .map(|(index, option)| {
                                    choice_row(
                                        "auth",
                                        &option.label,
                                        option.description.as_deref().unwrap_or_default(),
                                        index == prompt.selected,
                                        width,
                                    )
                                })
                                .collect();
                            append_choice_window(&mut rows, choice_rows, prompt.selected, height);
                        }
                    }
                }
            }
        }
        _ => {
            rows.push(text_row("alternate", "Nabla", title_style, width));
            rows.push(text_row(
                "alternate",
                "No alternate-screen route is active.",
                CellStyle::foreground(Color::Gray),
                width,
            ));
        }
    }
    rows
}

fn append_choice_window(
    rows: &mut Vec<VisualRow>,
    choices: Vec<VisualRow>,
    selected: usize,
    height: u16,
) {
    let visible_rows = usize::from(height).saturating_sub(rows.len());
    let range = VirtualList {
        total: choices.len(),
        selected,
        visible_rows,
    }
    .visible_range();
    rows.extend(choices[range].iter().cloned());
}

fn append_tree_choice_window(
    rows: &mut Vec<VisualRow>,
    choices: Vec<Vec<VisualRow>>,
    selected: usize,
    height: u16,
) {
    let visible_rows = usize::from(height).saturating_sub(rows.len());
    let visible_items = (visible_rows / 2).max(1);
    let range = VirtualList {
        total: choices.len(),
        selected,
        visible_rows: visible_items,
    }
    .visible_range();
    rows.extend(choices[range].iter().flatten().take(visible_rows).cloned());
}

fn tree_choice_rows(item: &TreeItem, selected: bool, width: u16) -> Vec<VisualRow> {
    let subject = tree_subject(item);
    let mut metadata = Vec::<String>::new();
    if let Some(label) = item.label.as_deref() {
        metadata.push(label.to_owned());
    }
    if item.is_active_path {
        metadata.push("active".to_owned());
    }
    if item.is_leaf {
        metadata.push("leaf".to_owned());
    }
    if item.foldable {
        metadata.push(if item.folded {
            "folded".to_owned()
        } else {
            "expanded".to_owned()
        });
    }
    let identity_style = if selected {
        palette::selected()
    } else if item.is_active_path {
        CellStyle::foreground(palette::ACTIVE_PATH).bold()
    } else {
        CellStyle::foreground(tree_identity_color(item)).bold()
    };
    let content_style = if selected {
        palette::selected()
    } else {
        CellStyle::foreground(palette::TEXT)
    };
    let description_style = if selected {
        palette::selected_muted()
    } else {
        CellStyle::foreground(palette::GRAY_MUTED)
    };
    let identity = tree_identity_label(item);
    let heading = aligned_panel_row(
        "tree-browser",
        &format!("• {identity}"),
        &metadata.join(" · "),
        identity_style,
        description_style,
        width,
    );

    let indent = truncate("  └ ", usize::from(width));
    let mut cells = styled_tree_cells(
        &indent,
        if selected {
            palette::selected()
        } else {
            CellStyle::foreground(palette::GRAY_FAINT)
        },
    );
    let branch = truncate(
        &tree_prefix(item),
        usize::from(width.saturating_sub(cells_width(&cells))),
    );
    cells.extend(styled_tree_cells(&branch, identity_style));
    let used = cells_width(&cells);
    let subject = truncate(&subject, usize::from(width.saturating_sub(used)));
    append_text_cells(&mut cells, &subject, content_style);
    vec![
        heading,
        VisualRow {
            component_id: "tree-browser".to_owned(),
            logical_line: 1,
            wrap_index: 0,
            cells,
        },
    ]
}

fn tree_subject(item: &TreeItem) -> String {
    let mut preview = item.preview.trim();
    if let Some(label) = item.label.as_deref() {
        let label_prefix = format!("[{label}]");
        if preview
            .get(..label_prefix.len())
            .is_some_and(|prefix| prefix == label_prefix)
        {
            preview = preview[label_prefix.len()..].trim_start();
        }
    }
    let identity = item.role.as_deref().unwrap_or(&item.kind);
    let prefix_length = identity.len().saturating_add(1);
    if preview.len() >= prefix_length
        && preview
            .get(..identity.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(identity))
        && preview.as_bytes().get(identity.len()) == Some(&b':')
    {
        return preview[prefix_length..].trim_start().to_owned();
    }
    preview.to_owned()
}

fn tree_identity_label(item: &TreeItem) -> String {
    match item.role.as_deref().unwrap_or(&item.kind) {
        "toolResult" | "tool_result" => "Tool result".to_owned(),
        "toolCall" | "tool_call" => "Tool call".to_owned(),
        "branch_summary" => "Branch summary".to_owned(),
        "custom_message" => "Custom message".to_owned(),
        "model_change" => "Model change".to_owned(),
        "thinking_level_change" => "Thinking level".to_owned(),
        "session_info" => "Session info".to_owned(),
        identity => identity
            .split(['_', '-'])
            .map(|part| {
                let mut chars = part.chars();
                chars.next().map_or_else(String::new, |first| {
                    first.to_uppercase().collect::<String>() + chars.as_str()
                })
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

fn styled_tree_cells(text: &str, style: CellStyle) -> Vec<StyledCell> {
    let mut cells = Vec::new();
    append_text_cells(&mut cells, text, style);
    cells
}

fn tree_identity_color(item: &TreeItem) -> Color {
    match item.role.as_deref().unwrap_or(&item.kind) {
        "user" => palette::BLUE,
        "assistant" | "agent" => palette::MAUVE,
        "tool" | "toolCall" | "tool_call" => palette::TEAL,
        "toolResult" | "tool_result" => palette::PEACH,
        "system" => palette::YELLOW,
        "custom" | "custom_message" => palette::PINK,
        "compaction" => palette::RED,
        "branch_summary" => palette::GREEN,
        "label" => palette::ROSEWATER,
        "model_change" => palette::SAPPHIRE,
        "thinking_level_change" => palette::LAVENDER,
        "session_info" => palette::SKY,
        _ => palette::SAPPHIRE,
    }
}

fn tree_prefix(item: &TreeItem) -> String {
    let depth = item.visual_depth;
    let ancestor_count = depth.saturating_sub(1);
    let mut prefix = String::new();
    let start = if depth > 4 {
        prefix.push_str("… ");
        ancestor_count.saturating_sub(2)
    } else {
        0
    };
    for position in start..ancestor_count {
        prefix.push_str(if item.gutter_positions.contains(&position) {
            "│ "
        } else {
            "  "
        });
    }
    if depth > 0 {
        prefix.push_str(if item.show_connector {
            if item.is_last { "└─" } else { "├─" }
        } else {
            "  "
        });
    }
    prefix.push_str(if item.foldable {
        if item.folded { "▸ " } else { "▾ " }
    } else {
        "· "
    });
    prefix
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewerItemGroup {
    User,
    Assistant,
    Tool,
    Turn,
    Other,
}

fn viewer_item_group(item: &TranscriptItem) -> ViewerItemGroup {
    match item {
        TranscriptItem::User(_) => ViewerItemGroup::User,
        TranscriptItem::Assistant(_) => ViewerItemGroup::Assistant,
        TranscriptItem::Tool(_) => ViewerItemGroup::Tool,
        TranscriptItem::TurnSeparator(_) => ViewerItemGroup::Turn,
        _ => ViewerItemGroup::Other,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{
        rpc::PiState,
        state::{
            ApprovalState, AssistantMessage, AuthFlowState, AuthPromptState, EditorState, RunState,
            SessionBrowserState, ToolExecution, ToolStatus, TranscriptItem, TranscriptViewerState,
            TreeBrowserState, UserMessage, UserMessageStatus,
        },
        ui::{SurfaceManager, store::UiStore},
    };

    use super::*;

    fn state() -> AppState {
        AppState::new(PiState {
            model: Some(json!({"provider": "test", "id": "model"})),
            thinking_level: "off".to_owned(),
            is_streaming: false,
            is_compacting: false,
            steering_mode: "one-at-a-time".to_owned(),
            follow_up_mode: "one-at-a-time".to_owned(),
            session_file: None,
            session_id: "session".to_owned(),
            session_name: None,
            auto_compaction_enabled: true,
            message_count: 0,
            pending_message_count: 0,
        })
    }

    #[test]
    fn frame_rows_layout_bounds_and_cursor_share_one_revision() {
        let mut domain = state();
        domain.transcript = vec![
            TranscriptItem::User(UserMessage {
                text: "hello".to_owned(),
                status: UserMessageStatus::Accepted,
            }),
            TranscriptItem::Assistant(AssistantMessage {
                text: "streaming".to_owned(),
                complete: false,
                ..AssistantMessage::default()
            }),
        ];
        domain.editor.insert_text("你👩🏽‍💻");
        let mut store = UiStore::new(super::super::types::TerminalSize::new(20, 8));
        store.synchronize(&domain);
        let frame = SceneBuilder.build(&domain, store.state(), SurfaceKind::Primary);

        assert_eq!(frame.revision, store.state().revision);
        assert_eq!(frame.rows.len(), 8);
        assert_eq!(frame.main_layout.status.y, 7);
        assert!(
            frame
                .component_bounds
                .keys()
                .any(|id| id.starts_with("assistant:0:2:text:segment:"))
        );
        assert!(
            frame
                .cursor
                .is_some_and(|cursor| cursor.row < 8 && cursor.column < 20)
        );
    }

    #[test]
    fn stable_history_remains_resident_until_it_leaves_the_fixed_window() {
        let mut domain = state();
        domain.transcript = vec![
            TranscriptItem::Notice("sealed".to_owned()),
            TranscriptItem::Assistant(AssistantMessage {
                text: "live".to_owned(),
                complete: false,
                ..AssistantMessage::default()
            }),
        ];
        let mut store = UiStore::new(super::super::types::TerminalSize::new(30, 8));
        store.synchronize(&domain);
        let frame = SceneBuilder.build(&domain, store.state(), SurfaceKind::Primary);
        let visible = frame
            .rows
            .iter()
            .map(VisualRow::plain_text)
            .collect::<String>();
        assert!(visible.contains("sealed"));
        assert!(visible.contains("live"));
    }

    #[test]
    fn empty_primary_surface_owns_the_full_screen_with_fixed_history_geometry() {
        let domain = state();
        let size = super::super::types::TerminalSize::new(40, 12);
        let mut store = UiStore::new(size);
        store.synchronize(&domain);

        let frame = SceneBuilder.build(&domain, store.state(), SurfaceKind::Primary);

        assert_eq!(frame.viewport, Rect::new(0, 0, size.width, size.height));
        assert_eq!(frame.main_layout.owned_surface, frame.viewport);
        assert_eq!(
            frame.main_layout.history_window,
            frame.main_layout.transcript
        );
        assert_eq!(frame.main_layout.history_window.y, 0);
        assert_eq!(
            frame.main_layout.history_window.bottom().saturating_add(1),
            frame.main_layout.composer.y
        );
        assert!(
            frame.rows[..usize::from(frame.main_layout.history_window.bottom())]
                .iter()
                .all(|row| row.plain_text().is_empty())
        );
    }

    #[test]
    fn bootstrap_blank_rows_move_up_as_transcript_grows() {
        let mut domain = state();
        let size = super::super::types::TerminalSize::new(40, 12);
        let mut store = UiStore::new(size);
        store.synchronize(&domain);
        let empty = SceneBuilder.build(&domain, store.state(), SurfaceKind::Primary);

        domain
            .transcript
            .push(TranscriptItem::Assistant(AssistantMessage {
                text: "first\n\nsecond\n\nthird".to_owned(),
                complete: false,
                ..AssistantMessage::default()
            }));
        store.synchronize(&domain);
        let grown = SceneBuilder.build(&domain, store.state(), SurfaceKind::Primary);

        assert_eq!(empty.viewport, Rect::new(0, 0, size.width, size.height));
        assert_eq!(grown.viewport, empty.viewport);
        let first_content = grown
            .rows
            .iter()
            .position(|row| !row.plain_text().is_empty())
            .expect("resident transcript content");
        assert!(first_content > 0);
        assert!(
            grown.rows[..first_content]
                .iter()
                .all(|row| row.plain_text().is_empty())
        );
    }

    #[test]
    fn claimed_primary_surface_never_exposes_shell_rows() {
        let domain = state();
        let size = super::super::types::TerminalSize::new(40, 12);
        let mut store = UiStore::new(size);
        store.synchronize(&domain);
        let frame = SceneBuilder.build(&domain, store.state(), SurfaceKind::Primary);

        assert_eq!(frame.viewport, Rect::new(0, 0, size.width, size.height));
        assert_eq!(frame.main_layout.transcript.y, 0);
        assert_eq!(frame.main_layout.status.bottom(), size.height);
    }

    #[test]
    fn message_completion_preserves_visible_row_positions() {
        let mut domain = state();
        domain
            .transcript
            .push(TranscriptItem::Assistant(AssistantMessage {
                id: 1,
                text: "```text\none\ntwo\nthree\nfour\nfive".to_owned(),
                complete: false,
                ..AssistantMessage::default()
            }));
        let size = super::super::types::TerminalSize::new(48, 14);
        let mut store = UiStore::new(size);
        store.synchronize(&domain);
        let streaming = SceneBuilder.build(&domain, store.state(), SurfaceKind::Primary);
        let row_before = streaming
            .rows
            .iter()
            .position(|row| row.plain_text().contains("three"))
            .expect("streaming row");

        let TranscriptItem::Assistant(message) = &mut domain.transcript[0] else {
            unreachable!()
        };
        message.complete = true;
        store.synchronize(&domain);
        let projection = store.state().transcript.project_primary(
            size.width,
            usize::from(streaming.main_layout.history_window.height),
            store.state().revision,
            100,
            usize::MAX,
            store.state().animation_frame,
        );
        let completed = SceneBuilder.build_with_projection(
            &domain,
            store.state(),
            SurfaceKind::Primary,
            &projection,
        );
        let row_after = completed
            .rows
            .iter()
            .position(|row| row.plain_text().contains("three"))
            .expect("completed row remains resident");

        assert_eq!(row_after, row_before);
    }

    #[test]
    fn completed_assistant_does_not_leave_screen_height_gap() {
        let mut domain = state();
        domain.transcript = vec![
            TranscriptItem::Assistant(AssistantMessage {
                id: 7,
                text: "```text\none\ntwo\nthree\nfour\nfive".to_owned(),
                complete: true,
                ..AssistantMessage::default()
            }),
            TranscriptItem::TurnSeparator(crate::state::TurnSeparator {
                turn_id: "turn-gap".to_owned(),
                started_at: "start".to_owned(),
                ended_at: "end".to_owned(),
                duration_ms: 1_000,
                estimated: false,
            }),
        ];
        let size = super::super::types::TerminalSize::new(48, 14);
        let mut store = UiStore::new(size);
        store.synchronize(&domain);
        let frame = SceneBuilder.build(&domain, store.state(), SurfaceKind::Primary);
        let last_history_row = frame.rows[..usize::from(frame.main_layout.composer.y)]
            .iter()
            .rposition(|row| !row.plain_text().is_empty())
            .expect("completed transcript remains visible");
        assert_eq!(
            last_history_row.saturating_add(2),
            usize::from(frame.main_layout.composer.y)
        );
    }

    #[test]
    fn turn_separator_remains_adjacent_to_visible_history() {
        let mut domain = state();
        domain.transcript = vec![
            TranscriptItem::Assistant(AssistantMessage {
                id: 1,
                text: "```text\nvisible tail".to_owned(),
                complete: true,
                ..AssistantMessage::default()
            }),
            TranscriptItem::TurnSeparator(crate::state::TurnSeparator {
                turn_id: "turn-adjacent".to_owned(),
                started_at: "start".to_owned(),
                ended_at: "end".to_owned(),
                duration_ms: 1_000,
                estimated: false,
            }),
        ];
        let size = super::super::types::TerminalSize::new(48, 14);
        let mut store = UiStore::new(size);
        store.synchronize(&domain);
        let frame = SceneBuilder.build(&domain, store.state(), SurfaceKind::Primary);
        let visible = frame
            .rows
            .iter()
            .map(VisualRow::plain_text)
            .collect::<Vec<_>>();
        let tail = visible
            .iter()
            .position(|row| row.contains("visible tail"))
            .expect("visible assistant tail");
        let separator = visible
            .iter()
            .position(|row| row.contains("Worked for"))
            .expect("visible turn separator");
        assert!(separator > tail && separator.saturating_sub(tail) <= 2);
    }

    #[test]
    fn opening_and_closing_panel_restores_owned_primary_rows() {
        let mut domain = state();
        domain
            .transcript
            .push(TranscriptItem::Assistant(AssistantMessage {
                text: "owned transcript row".to_owned(),
                complete: false,
                ..AssistantMessage::default()
            }));
        let size = super::super::types::TerminalSize::new(48, 14);
        let mut store = UiStore::new(size);
        store.synchronize(&domain);
        let baseline = SceneBuilder.build(&domain, store.state(), SurfaceKind::Primary);

        domain.editor.insert_text("/");
        store.synchronize(&domain);
        let opened = SceneBuilder.build(&domain, store.state(), SurfaceKind::Primary);
        assert!(opened.panel.is_some());
        domain.editor.clear();
        store.synchronize(&domain);
        let restored = SceneBuilder.build(&domain, store.state(), SurfaceKind::Primary);

        assert_eq!(baseline.viewport, Rect::new(0, 0, size.width, size.height));
        assert_eq!(restored.viewport, baseline.viewport);
        assert_eq!(
            restored
                .rows
                .iter()
                .map(VisualRow::plain_text)
                .collect::<Vec<_>>(),
            baseline
                .rows
                .iter()
                .map(VisualRow::plain_text)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn active_transcript_keeps_one_blank_row_above_the_composer() {
        let mut domain = state();
        domain.transcript.push(TranscriptItem::User(UserMessage {
            text: "hello from the bottom".to_owned(),
            status: UserMessageStatus::Pending,
        }));
        let size = super::super::types::TerminalSize::new(40, 12);
        let mut store = UiStore::new(size);
        store.synchronize(&domain);

        let frame = SceneBuilder.build(&domain, store.state(), SurfaceKind::Primary);
        let transcript = frame
            .component_bounds
            .get("transcript:0")
            .expect("active transcript bounds");

        assert_eq!(
            transcript.end.saturating_add(1),
            usize::from(frame.main_layout.composer.y)
        );
        assert!(frame.rows[transcript.end].plain_text().is_empty());
        assert_eq!(frame.viewport.y, 0);
        assert_eq!(frame.main_layout.history_window.y, 0);
        assert!(
            frame.rows[transcript.start..transcript.end]
                .iter()
                .map(VisualRow::plain_text)
                .collect::<Vec<_>>()
                .join("\n")
                .contains("hello from the bottom")
        );
        assert!(
            frame.rows[transcript.start..transcript.end]
                .iter()
                .any(|row| row.plain_text().starts_with('╭'))
        );
    }

    #[test]
    fn resident_turn_separator_keeps_an_owned_blank_row_above_the_composer() {
        let mut domain = state();
        domain
            .transcript
            .push(TranscriptItem::TurnSeparator(crate::state::TurnSeparator {
                turn_id: "turn-1".to_owned(),
                started_at: "start".to_owned(),
                ended_at: "end".to_owned(),
                duration_ms: 1_000,
                estimated: false,
            }));
        let size = super::super::types::TerminalSize::new(48, 10);
        let mut store = UiStore::new(size);
        store.synchronize(&domain);

        let frame = SceneBuilder.build(&domain, store.state(), SurfaceKind::Primary);
        let gap = frame.main_layout.composer.y.saturating_sub(1);
        assert_eq!(frame.viewport.y, 0);
        assert_eq!(frame.main_layout.history_window.y, 0);
        assert!(frame.rows[usize::from(gap)].plain_text().is_empty());
        assert!(
            frame.rows[..usize::from(gap)]
                .iter()
                .any(|row| row.plain_text().contains("Worked for"))
        );
        assert!(
            frame.rows[usize::from(frame.main_layout.composer.y)]
                .plain_text()
                .starts_with('╭')
        );
    }

    #[test]
    fn animation_changes_only_the_live_frame_and_never_history_or_domain_state() {
        let mut domain = state();
        domain.transcript.push(TranscriptItem::Tool(ToolExecution {
            id: "running-tool".to_owned(),
            name: "bash".to_owned(),
            args: json!({"command": "cargo test"}),
            output: String::new(),
            diff: None,
            status: ToolStatus::Running,
        }));
        let original_transcript = domain.transcript.clone();
        let size = super::super::types::TerminalSize::new(48, 10);
        let mut store = UiStore::new(size);
        store.synchronize(&domain);

        assert!(animation_active(&domain));
        let first = SceneBuilder.build(&domain, store.state(), SurfaceKind::Primary);
        store.state_mut().animation_frame = 1;
        let second = SceneBuilder.build(&domain, store.state(), SurfaceKind::Primary);
        let first_text = first
            .rows
            .iter()
            .map(VisualRow::plain_text)
            .collect::<Vec<_>>()
            .join("\n");
        let second_text = second
            .rows
            .iter()
            .map(VisualRow::plain_text)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(first_text.contains("⠋ Ran"));
        assert!(second_text.contains("⠙ Ran"));
        assert_eq!(domain.transcript, original_transcript);
        assert!(
            store
                .state()
                .transcript
                .project_primary(size.width, 100, 1, 100, usize::MAX, 0)
                .overflow_blocks
                .is_empty()
        );
    }

    #[test]
    fn panel_open_and_close_restores_the_occluded_transcript_exactly() {
        let mut domain = state();
        domain
            .transcript
            .push(TranscriptItem::Assistant(AssistantMessage {
                text: "persistent live row".to_owned(),
                complete: false,
                ..AssistantMessage::default()
            }));
        let mut store = UiStore::new(super::super::types::TerminalSize::new(80, 24));
        store.synchronize(&domain);
        let baseline = SceneBuilder.build(&domain, store.state(), SurfaceKind::Primary);

        domain.editor.insert_text("/");
        store.reduce(super::super::store::UiEvent::DomainChanged);
        let opened = SceneBuilder.build(&domain, store.state(), SurfaceKind::Primary);
        assert!(opened.main_layout.panel.is_some());
        assert_eq!(
            opened.main_layout.transcript,
            baseline.main_layout.transcript
        );
        assert_eq!(opened.main_layout.composer, baseline.main_layout.composer);
        assert_eq!(opened.main_layout.status, baseline.main_layout.status);
        assert_eq!(opened.viewport, baseline.viewport);
        let panel = opened.panel.as_ref().expect("floating panel");
        assert_eq!(panel.area.x, 0);
        assert_eq!(panel.area.width, opened.terminal_size.width);
        assert_eq!(
            panel.area.height,
            u16::try_from(COMMAND_MENU_VISIBLE_ROWS + 2).unwrap()
        );
        assert_eq!(panel.rows.len(), usize::from(panel.area.height));
        assert!(
            opened
                .hit_regions
                .iter()
                .all(|region| !matches!(region.target, HitTarget::Panel | HitTarget::Command(_)))
        );

        domain.editor.clear();
        store.reduce(super::super::store::UiEvent::DomainChanged);
        let restored = SceneBuilder.build(&domain, store.state(), SurfaceKind::Primary);
        assert!(restored.main_layout.panel.is_none());
        assert!(restored.panel.is_none());
        assert_eq!(
            baseline
                .rows
                .iter()
                .map(VisualRow::plain_text)
                .collect::<Vec<_>>(),
            restored
                .rows
                .iter()
                .map(VisualRow::plain_text)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn approval_panel_only_renders_actions_valid_for_the_request() {
        let mut domain = state();
        domain.approval = Some(ApprovalState {
            approval_id: "approval".to_owned(),
            tool_call_id: "tool".to_owned(),
            tool_name: "bash".to_owned(),
            input: json!({"command": "cargo test"}),
            agent_id: None,
            agent_profile: None,
            model: None,
            reason: Some("run tests".to_owned()),
            risk: Some("normal".to_owned()),
            summary: "run tests".to_owned(),
            selected: 0,
            replying: false,
            available_decisions: vec![
                ApprovalDecision::AllowOnce,
                ApprovalDecision::AllowSession,
                ApprovalDecision::AllowWorkspace,
                ApprovalDecision::Deny,
            ],
            session_grant: Some(GrantProposal {
                scope: "session".to_owned(),
                workspace_id: "workspace-1".to_owned(),
                session_id: Some("session-1".to_owned()),
                matchers: vec![json!({
                    "kind": "exec",
                    "executable": "cargo",
                    "argv": ["test"],
                    "cwd": "/workspace",
                    "environment": {}
                })],
                invalidation_keys: Vec::new(),
            }),
            workspace_grant: Some(GrantProposal {
                scope: "workspace".to_owned(),
                workspace_id: "workspace-1".to_owned(),
                session_id: None,
                matchers: vec![json!({
                    "kind": "exec",
                    "executable": "cargo",
                    "argv": ["test"],
                    "cwd": "/workspace",
                    "environment": {}
                })],
                invalidation_keys: vec![json!({
                    "kind": "file_digest",
                    "path": "/workspace/Cargo.toml",
                    "value": "manifest-digest"
                })],
            }),
            ..ApprovalState::default()
        });
        let size = super::super::types::TerminalSize::new(120, 16);
        let mut store = UiStore::new(size);
        store.synchronize(&domain);

        let frame = SceneBuilder.build(&domain, store.state(), SurfaceKind::Primary);
        let panel = frame.panel.expect("approval panel");
        let text = panel
            .rows
            .iter()
            .map(VisualRow::plain_text)
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(panel.area.width, size.width);
        assert!(panel.area.height >= 4);
        assert!(text.contains("Ask for Approval"));
        assert!(text.contains("run tests"));
        assert!(text.contains("cargo test"));
        let command_row = panel
            .rows
            .iter()
            .find(|row| row.plain_text().contains("cargo test"))
            .expect("indented command");
        assert!(command_row.plain_text().contains("    cargo test"));
        assert!(
            command_row
                .cells
                .iter()
                .filter(|cell| !cell.symbol.trim().is_empty() && cell.symbol != "│")
                .all(|cell| cell.style.foreground == Color::Gray && cell.style.dim)
        );
        assert!(text.contains("Allow once"));
        assert!(text.contains("Allow for Session"));
        assert!(text.contains("Allow for Workspace"));
        assert!(text.contains("Session saves: exec cargo test @ /workspace"));
        assert!(text.contains("Workspace saves: exec cargo test @ /workspace"));
        assert!(text.contains("file_digest /workspace/Cargo.toml=manifest-digest"));
        assert!(text.contains("Deny"));
        assert!(
            panel
                .rows
                .iter()
                .filter(|row| {
                    let text = row.plain_text();
                    text.contains("Allow once") || text.contains("Deny")
                })
                .all(|row| row.display_width() == size.width)
        );
        assert!(
            panel
                .rows
                .iter()
                .any(|row| row.plain_text().contains("Approve only this request"))
        );
        assert!(
            panel
                .rows
                .iter()
                .any(|row| row.plain_text().contains("Reject this tool request"))
        );
    }

    #[test]
    fn high_risk_approval_uses_the_same_full_width_floating_panel() {
        let mut domain = state();
        domain.selection_page_size = 12;
        domain.approval = Some(ApprovalState {
            approval_id: "approval".to_owned(),
            tool_call_id: "tool".to_owned(),
            tool_name: "bash".to_owned(),
            input: json!({"command": "printenv SECRET_TOKEN"}),
            agent_id: None,
            agent_profile: None,
            model: None,
            reason: Some("inspect credentials".to_owned()),
            risk: Some("credential".to_owned()),
            selected: 1,
            replying: false,
            available_decisions: vec![ApprovalDecision::AllowOnce, ApprovalDecision::Deny],
            ..ApprovalState::default()
        });
        let size = super::super::types::TerminalSize::new(72, 16);
        let mut store = UiStore::new(size);
        store.synchronize(&domain);

        assert_eq!(SurfaceManager.route(&domain), SurfaceKind::Primary);
        let frame = SceneBuilder.build(&domain, store.state(), SurfaceKind::Primary);
        let panel = frame.panel.expect("high-risk approval panel");
        let text = panel
            .rows
            .iter()
            .map(VisualRow::plain_text)
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(panel.area.width, size.width);
        assert_eq!(panel.rows.len(), 7);
        assert!(text.contains("Ask for Approval"));
        assert!(text.contains("May access sensitive credentials"));
        assert!(text.contains("printenv SECRET_TOKEN"));
        assert!(!text.contains("Credential risk"));
        assert!(!text.contains("inspect credentials"));
        assert!(!text.contains("Allow for Session"));
        assert!(!text.contains("Always allow matching"));
        assert!(
            panel
                .rows
                .iter()
                .filter(|row| {
                    let text = row.plain_text();
                    text.contains("Allow once") || text.contains("Deny")
                })
                .all(|row| row.display_width() == size.width)
        );
        assert!(
            panel
                .rows
                .iter()
                .any(|row| row.plain_text().contains("Approve only this request"))
        );
    }

    #[test]
    fn approval_panel_normalizes_paths_truncates_details_and_keeps_actions_visible() {
        let mut domain = state();
        domain.selection_page_size = 8;
        domain.approval = Some(ApprovalState {
            approval_id: "approval".to_owned(),
            tool_call_id: "tool".to_owned(),
            tool_name: "edit_file".to_owned(),
            input: json!({
                "path": "src/./nested/../lib.rs",
                "replacement": {
                    "deep": {"value": ["你好", "世界", {"more": true}]}
                }
            }),
            agent_id: Some("agent-1".to_owned()),
            agent_profile: Some("worker".to_owned()),
            model: Some("provider/model".to_owned()),
            reason: Some("Apply a requested change".to_owned()),
            risk: Some("outside_workspace".to_owned()),
            selected: 2,
            replying: false,
            available_decisions: vec![ApprovalDecision::AllowOnce, ApprovalDecision::Deny],
            ..ApprovalState::default()
        });

        let request = primary_panel_request(&domain, 36).expect("approval panel request");
        assert!(request.height <= 10);
        let all_text = request
            .rows
            .iter()
            .map(VisualRow::plain_text)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(all_text.contains("Ask for Approval"));
        assert!(all_text.contains("Outside trusted project scope"));
        assert!(all_text.contains("Edit src/lib.rs"));
        assert!(!all_text.contains("Workspace boundary"));
        assert!(!all_text.contains("Apply a requested change"));
        assert!(!all_text.contains("replacement"));
        assert!(all_text.contains("[N] Deny"));
        assert!(!all_text.contains("agent-1"));
        assert!(!all_text.contains("provider/model"));

        let selected = request.clone().render(Rect::new(0, 0, 36, 3));
        assert!(
            selected
                .rows
                .iter()
                .any(|row| row.plain_text().contains("[N] Deny"))
        );
    }

    #[test]
    fn narrow_panel_rows_preserve_action_and_shortcut_before_description() {
        let row = panel_choice_row(
            "approval",
            "[Y] Allow once",
            "Approve only this request",
            true,
            true,
            12,
        );
        assert_eq!(row.display_width(), 12);
        assert!(row.plain_text().contains("[Y]"));
        assert!(!row.plain_text().contains("Approve"));
    }

    #[test]
    fn alternate_transcript_uses_modes_expansion_selection_and_full_output() {
        let mut domain = state();
        domain
            .transcript
            .push(TranscriptItem::Tool(crate::state::ToolExecution {
                id: "tool-1".to_owned(),
                name: "read_file".to_owned(),
                args: json!({"path": "src/lib.rs", "line": 7}),
                output: "FULL OUTPUT\nsecond line".to_owned(),
                diff: None,
                status: crate::state::ToolStatus::Succeeded,
            }));
        domain.transcript_viewer = Some(TranscriptViewerState::new(
            TranscriptViewMode::Normal,
            &domain.transcript,
        ));
        let size = super::super::types::TerminalSize::new(48, 20);
        let mut store = UiStore::new(size);
        store.synchronize(&domain);

        let normal = SceneBuilder.build(&domain, store.state(), SurfaceKind::Alternate);
        let normal_text = normal
            .rows
            .iter()
            .map(VisualRow::plain_text)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!normal_text.contains("FULL OUTPUT"));
        assert_eq!(normal.main_layout.composer.height, 3);
        assert!(
            normal.rows[usize::from(normal.main_layout.composer.y.saturating_sub(1))]
                .plain_text()
                .is_empty()
        );
        assert!(
            normal.rows[usize::from(normal.main_layout.composer.y)]
                .plain_text()
                .starts_with('╭')
        );

        domain.transcript_viewer.as_mut().unwrap().mode = TranscriptViewMode::Verbose;
        let verbose = SceneBuilder.build(&domain, store.state(), SurfaceKind::Alternate);
        let verbose_text = verbose
            .rows
            .iter()
            .map(VisualRow::plain_text)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(verbose_text.contains("Arguments"));
        assert!(verbose_text.contains("\"line\": 7"));
        assert!(verbose_text.contains("FULL OUTPUT"));
        assert!(
            verbose
                .rows
                .iter()
                .flat_map(|row| &row.cells)
                .any(|cell| { cell.style.background == palette::SURFACE_0 })
        );

        domain.transcript_viewer.as_mut().unwrap().mode = TranscriptViewMode::Summary;
        let summary = SceneBuilder.build(&domain, store.state(), SurfaceKind::Alternate);
        let summary_text = summary
            .rows
            .iter()
            .map(VisualRow::plain_text)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!summary_text.contains("Arguments"));
        assert!(!summary_text.contains("FULL OUTPUT"));
    }

    #[test]
    fn common_panel_rows_right_align_descriptions() {
        let width = 48;
        let normal = panel_choice_row("panel", "Option", "Description", false, true, width);
        let selected = panel_choice_row("panel", "Selected", "Right aligned", true, true, width);

        for (row, description) in [(&normal, "Description"), (&selected, "Right aligned")] {
            assert_eq!(row.display_width(), width);
            assert!(row.plain_text().ends_with(description));
            assert!(!row.plain_text().contains('○'));
            assert!(!row.plain_text().contains('●'));
        }
        assert!(
            selected
                .cells
                .iter()
                .all(|cell| cell.style.background == Color::Default)
        );
        assert!(
            selected
                .cells
                .iter()
                .all(|cell| { cell.style.foreground == palette::LAVENDER && cell.style.bold })
        );
    }

    #[test]
    fn alternate_search_box_tracks_editor_focus_cursor_and_escape_visual_state() {
        let mut domain = state();
        domain
            .transcript
            .push(TranscriptItem::Notice("你好 result".to_owned()));
        domain.transcript_viewer = Some(TranscriptViewerState::new(
            TranscriptViewMode::Normal,
            &domain.transcript,
        ));
        let viewer = domain.transcript_viewer.as_mut().unwrap();
        viewer.search_active = true;
        viewer.search_query.insert_text("你好");
        let size = super::super::types::TerminalSize::new(36, 12);
        let mut store = UiStore::new(size);
        store.synchronize(&domain);

        let frame = SceneBuilder.build(&domain, store.state(), SurfaceKind::Alternate);
        let composer = frame.main_layout.composer;
        let text = frame.rows[usize::from(composer.y)..usize::from(composer.bottom())]
            .iter()
            .map(VisualRow::plain_text)
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(composer.height, 3);
        assert!(text.contains("你好"));
        assert!(frame.cursor.is_some());
        assert!(
            frame.rows[usize::from(composer.y)]
                .cells
                .iter()
                .all(|cell| cell.style.foreground == palette::INPUT_ACCENT)
        );
    }

    #[test]
    fn alternate_input_projection_covers_session_tree_and_secret_auth_editors() {
        let mut domain = state();
        domain.session_browser = Some(SessionBrowserState::loading());
        let session = alternate_input_model(&domain);
        assert_eq!(session.placeholder, "Search sessions");
        assert!(!session.focused);

        domain.session_browser = None;
        let mut tree = TreeBrowserState::loading();
        let mut label = EditorState::default();
        label.insert_text("branch label");
        tree.phase = TreePhase::EditLabel {
            entry_id: "entry".to_owned(),
            editor: label,
        };
        domain.tree_browser = Some(tree);
        let tree = alternate_input_model(&domain);
        assert_eq!(tree.text, "branch label");
        assert!(tree.focused);

        domain.tree_browser = None;
        let mut secret = EditorState::default();
        secret.insert_text("密钥");
        domain.auth_state = AuthState::Running(Box::new(AuthFlowState {
            id: "flow".to_owned(),
            provider_name: "Provider".to_owned(),
            status: "Waiting".to_owned(),
            url: None,
            device_code: None,
            prompt: Some(AuthPromptState {
                id: "prompt".to_owned(),
                kind: AuthPromptKind::Secret,
                message: "Enter token".to_owned(),
                placeholder: None,
                options: Vec::new(),
                selected: 0,
                editor: secret,
            }),
        }));
        let auth = alternate_input_model(&domain);
        assert!(auth.focused);
        assert!(auth.secret);
        assert_eq!(auth.display_text(), "••");
    }

    #[test]
    fn deep_tree_prefix_caps_gutter_and_keeps_recent_connectors() {
        let item = TreeItem {
            entry_id: "entry".to_owned(),
            parent_id: Some("parent".to_owned()),
            kind: "message".to_owned(),
            role: Some("assistant".to_owned()),
            preview: "assistant: deep node".to_owned(),
            label: None,
            label_timestamp: None,
            visual_depth: 9,
            show_connector: true,
            gutter_positions: vec![0, 2, 5, 6, 7],
            is_last: true,
            is_active_path: true,
            is_leaf: false,
            foldable: true,
            folded: true,
        };
        let prefix = tree_prefix(&item);
        assert!(prefix.starts_with("… "));
        assert!(prefix.contains('│'));
        assert!(prefix.contains("└─"));
        assert!(prefix.ends_with("▸ "));
        assert!(display_width(&prefix) <= 10);

        let mut identity_item = item;
        identity_item.is_active_path = false;
        let rows = tree_choice_rows(&identity_item, false, 48);
        assert_eq!(rows.len(), 2);
        assert!(rows[0].plain_text().starts_with("• Assistant"));
        assert!(rows[1].plain_text().starts_with("  └ "));
        assert!(!rows[1].plain_text().contains("assistant:"));
        assert!(
            rows[1]
                .cells
                .iter()
                .any(|cell| { cell.symbol == "d" && cell.style.foreground == palette::TEXT })
        );
        assert!(
            rows[1]
                .cells
                .iter()
                .any(|cell| { cell.symbol == "…" && cell.style.foreground == palette::MAUVE })
        );

        let selected = tree_choice_rows(&identity_item, true, 48);
        assert!(
            selected
                .iter()
                .flat_map(|row| &row.cells)
                .all(|cell| cell.style.background == Color::Default)
        );
        assert!(
            selected
                .iter()
                .flat_map(|row| &row.cells)
                .all(|cell| cell.style.bold)
        );
        for width in 1..=12 {
            assert!(
                tree_choice_rows(&identity_item, false, width)
                    .iter()
                    .all(|row| row.display_width() <= width)
            );
        }

        identity_item.label = Some("checkpoint".to_owned());
        identity_item.preview = "[checkpoint] assistant: deep node".to_owned();
        let labeled = tree_choice_rows(&identity_item, false, 48);
        assert!(labeled[0].plain_text().contains("checkpoint"));
        assert!(labeled[1].plain_text().contains("deep node"));
        assert!(!labeled[1].plain_text().contains("checkpoint"));
        assert!(!labeled[1].plain_text().contains("assistant:"));

        for (role, kind, color) in [
            (Some("user"), "message", palette::BLUE),
            (Some("assistant"), "message", palette::MAUVE),
            (Some("toolCall"), "message", palette::TEAL),
            (Some("toolResult"), "message", palette::PEACH),
            (None, "compaction", palette::RED),
            (None, "branch_summary", palette::GREEN),
            (None, "custom_message", palette::PINK),
        ] {
            identity_item.role = role.map(ToOwned::to_owned);
            identity_item.kind = kind.to_owned();
            assert_eq!(tree_identity_color(&identity_item), color);
        }
    }

    #[test]
    fn resize_rebuilds_every_row_at_the_new_width_and_height() {
        let mut domain = state();
        domain
            .transcript
            .push(TranscriptItem::Assistant(AssistantMessage {
                text: "你👩🏽‍💻 mixed-width content ".repeat(20),
                complete: false,
                ..AssistantMessage::default()
            }));
        let mut store = UiStore::new(super::super::types::TerminalSize::new(120, 40));
        store.synchronize(&domain);

        for size in [
            super::super::types::TerminalSize::new(120, 40),
            super::super::types::TerminalSize::new(80, 24),
            super::super::types::TerminalSize::new(200, 60),
        ] {
            store.reduce(super::super::store::UiEvent::Resize(size));
            let frame = SceneBuilder.build(&domain, store.state(), SurfaceKind::Primary);
            assert_eq!(frame.terminal_size, size);
            assert_eq!(frame.rows.len(), usize::from(size.height));
            assert_eq!(frame.viewport.bottom(), size.height);
            assert!(frame.viewport.height <= size.height);
            assert!(
                frame
                    .rows
                    .iter()
                    .all(|row| row.display_width() <= size.width)
            );
        }
    }

    #[test]
    fn busy_to_idle_converges_in_the_same_frame_without_layout_holes() {
        let mut domain = state();
        let mut store = UiStore::new(super::super::types::TerminalSize::new(80, 24));
        store.synchronize(&domain);
        domain.run_state = RunState::Running;
        store.reduce(super::super::store::UiEvent::DomainChanged);
        let busy = SceneBuilder.build(&domain, store.state(), SurfaceKind::Primary);

        domain.run_state = RunState::Idle;
        store.reduce(super::super::store::UiEvent::DomainChanged);
        let idle = SceneBuilder.build(&domain, store.state(), SurfaceKind::Primary);
        assert_eq!(busy.main_layout, idle.main_layout);
        assert_eq!(busy.rows.len(), idle.rows.len());
        let busy_status = busy.rows.last().unwrap().plain_text();
        let idle_status = idle.rows.last().unwrap().plain_text();
        assert!(busy_status.starts_with("model · thinking off"));
        assert!(busy_status.contains("⠋ · ctx"));
        assert!(!busy_status.contains("running"));
        assert!(!busy_status.contains("connected"));
        assert!(idle_status.starts_with("model · thinking off"));
        assert!(!idle_status.contains('⠋'));
        assert!(!idle_status.contains("idle"));
        assert!(!idle_status.contains("connected"));
    }

    #[test]
    fn composer_has_a_full_width_border_and_cursor_stays_inside_it() {
        let mut domain = state();
        domain.editor.insert_text("hello");
        let size = super::super::types::TerminalSize::new(32, 10);
        let mut store = UiStore::new(size);
        store.synchronize(&domain);

        let frame = SceneBuilder.build(&domain, store.state(), SurfaceKind::Primary);
        let composer = frame.main_layout.composer;
        let top = &frame.rows[usize::from(composer.y)];
        let content = &frame.rows[usize::from(composer.y.saturating_add(1))];
        let bottom = &frame.rows[usize::from(composer.bottom().saturating_sub(1))];

        assert_eq!(top.display_width(), size.width);
        assert_eq!(content.display_width(), size.width);
        assert_eq!(bottom.display_width(), size.width);
        assert!(top.plain_text().starts_with('╭'));
        assert!(top.plain_text().ends_with('╮'));
        assert!(content.plain_text().starts_with("│› "));
        assert!(content.plain_text().ends_with('│'));
        assert!(
            content
                .cells
                .iter()
                .filter(|cell| {
                    !cell.symbol.trim().is_empty() && cell.symbol != "│" && cell.symbol != "›"
                })
                .all(|cell| !cell.style.bold)
        );
        assert!(bottom.plain_text().starts_with('╰'));
        assert!(bottom.plain_text().ends_with('╯'));
        assert!(frame.cursor.is_some_and(|cursor| {
            cursor.row > composer.y
                && cursor.row < composer.bottom().saturating_sub(1)
                && cursor.column < size.width
        }));
    }

    #[test]
    fn status_line_fills_the_width_without_a_background() {
        let domain = state();
        let size = super::super::types::TerminalSize::new(48, 10);
        let mut store = UiStore::new(size);
        store.synchronize(&domain);

        let frame = SceneBuilder.build(&domain, store.state(), SurfaceKind::Primary);
        let status = frame.rows.last().expect("status row");
        assert_eq!(status.display_width(), size.width);
        assert!(
            status
                .cells
                .iter()
                .all(|cell| cell.style.background == Color::Default && !cell.style.reversed)
        );
    }

    #[test]
    fn plan_review_panel_shows_only_execute_fresh_execute_close() {
        let mut domain = state();
        domain.plan_review = Some(crate::state::PlanReviewState {
            selected: 0,
            submitting: false,
        });
        domain.context = crate::state::ContextSnapshot {
            usage_state: crate::state::ContextUsageState::Estimated,
            actual_tokens: Some(40_000),
            actual_percent: Some(40.0),
            context_window: Some(100_000),
            ..crate::state::ContextSnapshot::default()
        };

        let panel = primary_panel_request(&domain, 48).expect("plan review panel");
        let text = panel
            .rows
            .iter()
            .map(|row| row.plain_text())
            .collect::<Vec<_>>()
            .join(" ");

        assert!(text.contains("Current context remaining: 60% (estimated)"));
        for label in ["Execute", "Fresh execute", "Close"] {
            assert!(text.contains(label), "missing {label}: {text}");
        }
        assert!(!text.contains("Confirm"));
        assert!(!text.contains("Execute in current context"));
        assert_eq!(panel.selected_row, Some(1));
    }

    #[test]
    fn plan_review_panel_shows_unknown_when_context_is_unavailable() {
        let mut domain = state();
        domain.plan_review = Some(crate::state::PlanReviewState {
            selected: 2,
            submitting: false,
        });

        let panel = primary_panel_request(&domain, 48).expect("plan review panel");
        let text = panel
            .rows
            .iter()
            .map(|row| row.plain_text())
            .collect::<Vec<_>>()
            .join(" ");

        assert!(text.contains("Current context remaining: unknown"));
        assert_eq!(panel.selected_row, Some(3));
    }

    #[test]
    fn open_panel_sits_flush_against_the_composer() {
        let size = super::super::types::TerminalSize::new(48, 14);
        let mut store = UiStore::new(size);
        let baseline_domain = state();
        store.synchronize(&baseline_domain);
        let baseline = SceneBuilder.build(&baseline_domain, store.state(), SurfaceKind::Primary);

        let mut domain = state();
        domain.plan_review = Some(crate::state::PlanReviewState {
            selected: 0,
            submitting: false,
        });
        store.synchronize(&domain);

        let frame = SceneBuilder.build(&domain, store.state(), SurfaceKind::Primary);
        let panel = frame.panel.expect("plan review panel");

        assert_eq!(frame.main_layout.composer, baseline.main_layout.composer);
        assert_eq!(frame.main_layout.composer.y, panel.area.bottom());
        assert!(
            panel
                .rows
                .last()
                .expect("panel border")
                .plain_text()
                .starts_with('╰')
        );
        assert!(
            frame.rows[usize::from(frame.main_layout.composer.y)]
                .plain_text()
                .starts_with('╭')
        );
    }
}
