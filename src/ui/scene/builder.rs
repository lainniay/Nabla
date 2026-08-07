use crate::ui::{
    layout::{LayoutEngine, LayoutRequest},
    store::UiState,
    text::{GraphemeIndex, cursor_geometry, wrap_text},
    types::{
        CellStyle, Color, CursorPosition, HitRegion, HitTarget, MainLayout,
        PrimaryTranscriptProjection, Rect, SurfaceKind, VisualFrame,
    },
};

use super::{
    COMPOSER_CHROME_HEIGHT, MAX_COMPOSER_CONTENT_HEIGHT,
    canvas::Canvas,
    composer::{
        alternate_composer_rows, alternate_input_model, alternate_status, composer_content_width,
        composer_rows,
    },
    panels::{alternate_rows, primary_panel_request},
    status::status_row,
    text_row,
    view_model::SceneViewModel,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct SceneBuilder;

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
        projection: &PrimaryTranscriptProjection,
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
        projection: Option<&PrimaryTranscriptProjection>,
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
