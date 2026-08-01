use ratatui::layout::Rect;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UiLayoutMetrics {
    pub terminal_columns: u16,
    pub terminal_rows: u16,
    pub desired_height: u16,
    pub output_height: u16,
    pub auxiliary_height: u16,
    pub composer_height: u16,
    pub footer_height: u16,
    pub body_height: u16,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ComposerViewport {
    pub first_visual_row: usize,
    pub visible_rows: u16,
    pub total_visual_rows: usize,
    pub cursor_visual_row: usize,
    pub cursor_visual_column: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseCaptureMode {
    Off,
    Surface,
}

impl Default for MouseCaptureMode {
    fn default() -> Self {
        Self::Off
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiHitTarget {
    CommandCandidate(usize),
    ChoiceOption(usize),
    ListRow(usize),
    TranscriptItem(usize),
    SurfaceBody,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiHitRegion {
    pub area: Rect,
    pub target: UiHitTarget,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UiHitMap {
    pub regions: Vec<UiHitRegion>,
}

impl UiHitMap {
    pub fn push(&mut self, area: Rect, target: UiHitTarget) {
        if area.width > 0 && area.height > 0 {
            self.regions.push(UiHitRegion { area, target });
        }
    }

    pub fn target_at(&self, column: u16, row: u16) -> Option<UiHitTarget> {
        self.regions
            .iter()
            .rev()
            .find(|region| {
                column >= region.area.x
                    && column < region.area.right()
                    && row >= region.area.y
                    && row < region.area.bottom()
            })
            .map(|region| region.target.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiInputEvent {
    ScrollUp { lines: usize },
    ScrollDown { lines: usize },
    Click(UiHitTarget),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RenderOutcome {
    pub desired_height: u16,
    pub hit_map: UiHitMap,
    pub mouse_capture: MouseCaptureMode,
    pub metrics: UiLayoutMetrics,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_map_prefers_the_most_specific_latest_region() {
        let mut map = UiHitMap::default();
        map.push(Rect::new(0, 0, 20, 5), UiHitTarget::SurfaceBody);
        map.push(Rect::new(0, 2, 20, 1), UiHitTarget::CommandCandidate(3));

        assert_eq!(map.target_at(4, 2), Some(UiHitTarget::CommandCandidate(3)));
        assert_eq!(map.target_at(4, 1), Some(UiHitTarget::SurfaceBody));
        assert_eq!(map.target_at(30, 1), None);
    }
}
