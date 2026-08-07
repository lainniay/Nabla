use std::collections::HashMap;

use crate::ui::types::{
    ComponentId, CursorPosition, HitRegion, MainLayout, PanelFrame, Rect, RowRange, TerminalSize,
    VisualFrame, VisualRow,
};

pub(crate) struct Canvas {
    revision: u64,
    size: TerminalSize,
    rows: Vec<VisualRow>,
    bounds: HashMap<ComponentId, RowRange>,
    pub(crate) hit_regions: Vec<HitRegion>,
    pub(crate) cursor: Option<CursorPosition>,
    layout: MainLayout,
    pub(crate) viewport: Rect,
    pub(crate) panel: Option<PanelFrame>,
}

impl Canvas {
    pub(crate) fn new(revision: u64, size: TerminalSize, layout: MainLayout) -> Self {
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

    pub(crate) fn place(&mut self, area: Rect, source: &[VisualRow]) {
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

    pub(crate) fn place_tail(&mut self, area: Rect, source: &[VisualRow]) -> Option<Rect> {
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

    pub(crate) fn finish(self) -> VisualFrame {
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
