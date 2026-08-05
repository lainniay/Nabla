use super::types::{MainLayout, Rect, TerminalSize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutRequest {
    pub composer_height: u16,
    pub status_height: u16,
    /// Total visual height requested by the panel owner. The layout engine
    /// only clips it to the rows physically available above the composer.
    pub panel_height: Option<u16>,
}

impl Default for LayoutRequest {
    fn default() -> Self {
        Self {
            composer_height: 1,
            status_height: 1,
            panel_height: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct LayoutEngine;

impl LayoutEngine {
    pub fn layout(self, size: TerminalSize, request: LayoutRequest) -> MainLayout {
        let width = size.width.max(1);
        let height = size.height.max(1);
        let status_height = request.status_height.min(height);
        let composer_height = request
            .composer_height
            .max(1)
            .min(height.saturating_sub(status_height).max(1));
        let available_above = height.saturating_sub(status_height.saturating_add(composer_height));
        let panel_height = request
            .panel_height
            .map(|requested| requested.min(available_above))
            .unwrap_or(0);
        let panel_y = available_above.saturating_sub(panel_height);
        let composer_y = available_above;
        let status_y = height.saturating_sub(status_height);

        let history_window = Rect::new(0, 0, width, available_above);
        MainLayout {
            transcript: history_window,
            history_window,
            owned_surface: Rect::new(0, 0, width, height),
            panel: (panel_height > 0).then_some(Rect::new(0, panel_y, width, panel_height)),
            composer: Rect::new(0, composer_y, width, composer_height),
            status: Rect::new(0, status_y, width, status_height),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_overlays_transcript_without_changing_base_layout() {
        let engine = LayoutEngine;
        let base = engine.layout(TerminalSize::new(80, 24), LayoutRequest::default());
        let panel = engine.layout(
            TerminalSize::new(80, 24),
            LayoutRequest {
                panel_height: Some(8),
                ..LayoutRequest::default()
            },
        );
        assert_eq!(base.composer, panel.composer);
        assert_eq!(base.status, panel.status);
        assert_eq!(panel.panel.unwrap(), Rect::new(0, 14, 80, 8));
        assert_eq!(base.transcript, panel.transcript);
        assert_eq!(panel.transcript.height, 22);
    }

    #[test]
    fn external_panel_height_is_only_clipped_by_available_terminal_rows() {
        let layout = LayoutEngine.layout(
            TerminalSize::new(20, 5),
            LayoutRequest {
                composer_height: 2,
                status_height: 1,
                panel_height: Some(20),
            },
        );
        assert_eq!(layout.transcript.height, 2);
        assert_eq!(layout.panel.unwrap(), Rect::new(0, 0, 20, 2));
        assert_eq!(layout.composer.y, 2);
    }

    #[test]
    fn primary_history_window_is_content_independent_and_owns_the_full_width() {
        let request = LayoutRequest {
            composer_height: 3,
            status_height: 1,
            panel_height: None,
        };
        let first = LayoutEngine.layout(TerminalSize::new(40, 24), request);
        let second = LayoutEngine.layout(TerminalSize::new(40, 24), request);

        assert_eq!(first.history_window, second.history_window);
        assert_eq!(first.history_window, first.transcript);
        assert_eq!(first.history_window, Rect::new(0, 0, 40, 20));
        assert_eq!(first.owned_surface, Rect::new(0, 0, 40, 24));
    }

    #[test]
    fn composer_and_status_heights_only_change_resident_capacity() {
        let compact = LayoutEngine.layout(
            TerminalSize::new(60, 24),
            LayoutRequest {
                composer_height: 1,
                status_height: 1,
                panel_height: None,
            },
        );
        let expanded_composer = LayoutEngine.layout(
            TerminalSize::new(60, 24),
            LayoutRequest {
                composer_height: 4,
                status_height: 1,
                panel_height: None,
            },
        );
        let expanded_status = LayoutEngine.layout(
            TerminalSize::new(60, 24),
            LayoutRequest {
                composer_height: 1,
                status_height: 2,
                panel_height: None,
            },
        );

        assert_eq!(compact.history_window.height, 22);
        assert_eq!(expanded_composer.history_window.height, 19);
        assert_eq!(expanded_status.history_window.height, 21);
        assert_eq!(compact.owned_surface, expanded_composer.owned_surface);
        assert_eq!(compact.owned_surface, expanded_status.owned_surface);
    }
}
