use super::{
    palette,
    selector::VirtualList,
    text::{display_width, truncate},
    types::{CellStyle, PanelFrame, Rect, StyledCell, VisualRow},
};

/// Presentation-only request for a full-width primary-screen panel.
///
/// The owner supplies the total visual height. Layout remains responsible
/// only for clipping that height to the rows available above the composer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelRequest {
    pub height: u16,
    pub rows: Vec<VisualRow>,
    pub selected_row: Option<usize>,
}

impl PanelRequest {
    pub fn new(rows: Vec<VisualRow>, selected_row: Option<usize>, height: usize) -> Option<Self> {
        if rows.is_empty() || height == 0 {
            return None;
        }
        Some(Self {
            height: u16::try_from(height.saturating_add(2)).unwrap_or(u16::MAX),
            rows,
            selected_row,
        })
    }

    pub fn render(self, area: Rect) -> PanelFrame {
        if area.height == 0 {
            return PanelFrame {
                area,
                rows: Vec::new(),
            };
        }
        let content_rows = usize::from(area.height.saturating_sub(2));
        let selected = self
            .selected_row
            .unwrap_or_else(|| self.rows.len().saturating_sub(1));
        let range = VirtualList {
            total: self.rows.len(),
            selected,
            visible_rows: content_rows,
        }
        .visible_range();
        let border = CellStyle::foreground(palette::PANEL_BORDER).bold();
        let mut rows = Vec::with_capacity(usize::from(area.height));
        rows.push(border_row("╭", "╮", area.width, border));
        if area.height > 1 {
            rows.extend(
                self.rows[range]
                    .iter()
                    .map(|row| framed_content_row(row, area.width, border)),
            );
            rows.resize_with(usize::from(area.height.saturating_sub(1)), || {
                framed_content_row(&VisualRow::blank("panel"), area.width, border)
            });
            rows.push(border_row("╰", "╯", area.width, border));
        }
        rows.truncate(usize::from(area.height));
        PanelFrame { area, rows }
    }
}

fn border_row(left: &str, right: &str, width: u16, style: CellStyle) -> VisualRow {
    if width < 2 {
        return VisualRow {
            component_id: "panel-border".to_owned(),
            logical_line: 0,
            wrap_index: 0,
            cells: vec![StyledCell::new(left, 1, style)],
        };
    }
    VisualRow {
        component_id: "panel-border".to_owned(),
        logical_line: 0,
        wrap_index: 0,
        cells: vec![
            StyledCell::new(left, 1, style),
            StyledCell::new(
                "─".repeat(usize::from(width.saturating_sub(2))),
                width.saturating_sub(2),
                style,
            ),
            StyledCell::new(right, 1, style),
        ],
    }
}

fn framed_content_row(row: &VisualRow, width: u16, border: CellStyle) -> VisualRow {
    if width < 2 {
        return border_row("│", "│", width, border);
    }
    let inner_width = width.saturating_sub(2);
    let mut cells = vec![StyledCell::new("│", 1, border)];
    let mut remaining = inner_width;
    for cell in &row.cells {
        if remaining == 0 {
            break;
        }
        if cell.width <= remaining {
            cells.push(cell.clone());
            remaining = remaining.saturating_sub(cell.width);
        } else {
            let symbol = truncate(&cell.symbol, usize::from(remaining));
            let symbol_width = u16::try_from(display_width(&symbol)).unwrap_or(remaining);
            if symbol_width > 0 {
                cells.push(StyledCell::new(symbol, symbol_width, cell.style));
                remaining = remaining.saturating_sub(symbol_width);
            }
            break;
        }
    }
    if remaining > 0 {
        cells.push(StyledCell::new(
            " ".repeat(usize::from(remaining)),
            remaining,
            CellStyle::foreground(palette::TEXT),
        ));
    }
    cells.push(StyledCell::new("│", 1, border));
    VisualRow {
        component_id: row.component_id.clone(),
        logical_line: row.logical_line,
        wrap_index: row.wrap_index,
        cells,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_height_is_preserved_and_selected_row_stays_visible() {
        let rows = (0..20)
            .map(|index| VisualRow::blank(format!("row-{index}")))
            .collect::<Vec<_>>();
        let panel = PanelRequest::new(rows, Some(12), 5)
            .unwrap()
            .render(Rect::new(0, 10, 80, 5));

        assert_eq!(panel.area, Rect::new(0, 10, 80, 5));
        assert_eq!(panel.rows.len(), 5);
        assert!(panel.rows.iter().any(|row| row.component_id == "row-12"));
        assert!(panel.rows[0].plain_text().starts_with('╭'));
        assert!(panel.rows[4].plain_text().starts_with('╰'));
    }
}
