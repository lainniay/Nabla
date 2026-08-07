use crate::state::{UserMessage, UserMessageStatus};
use crate::ui::{
    palette,
    text::{display_width, wrap_file_references},
    types::{CellStyle, Color, StyledCell, VisualRow},
};

pub(crate) fn render_user(id: &str, message: &UserMessage, width: u16) -> Vec<VisualRow> {
    let border_style = match message.status {
        UserMessageStatus::Pending => CellStyle::foreground(Color::Yellow).bold(),
        UserMessageStatus::Accepted => CellStyle::foreground(palette::HISTORY_BORDER).dim(),
        UserMessageStatus::Failed => CellStyle::foreground(Color::Red).bold(),
    };
    let body_style = CellStyle::foreground(Color::White);
    if width < 6 {
        let content_width = width.saturating_sub(2).max(1);
        let mut rows = wrap_file_references(id, &message.text, content_width, body_style);
        for (index, row) in rows.iter_mut().enumerate() {
            let mut cells = if index == 0 {
                vec![
                    StyledCell::new("›", 1, border_style),
                    StyledCell::new(" ", 1, body_style),
                ]
            } else {
                vec![StyledCell::new("  ", 2, body_style)]
            };
            cells.extend(std::mem::take(&mut row.cells));
            row.cells = cells;
        }
        return rows;
    }

    let inner_width = width.saturating_sub(4).max(1);
    let mut rows = vec![user_border_row(
        id,
        width,
        true,
        match message.status {
            UserMessageStatus::Pending => Some("pending"),
            UserMessageStatus::Failed => Some("failed"),
            UserMessageStatus::Accepted => None,
        },
        border_style,
    )];
    for mut row in wrap_file_references(id, &message.text, inner_width, body_style) {
        let content_width = row.display_width();
        let padding = inner_width.saturating_sub(content_width);
        let mut cells = vec![
            StyledCell::new("│", 1, border_style),
            StyledCell::new(" ", 1, body_style),
        ];
        cells.extend(std::mem::take(&mut row.cells));
        if padding > 0 {
            cells.push(StyledCell::new(
                " ".repeat(usize::from(padding)),
                padding,
                body_style,
            ));
        }
        cells.push(StyledCell::new(" ", 1, body_style));
        cells.push(StyledCell::new("│", 1, border_style));
        row.cells = cells;
        rows.push(row);
    }
    rows.push(user_border_row(id, width, false, None, border_style));
    rows
}

fn user_border_row(
    id: &str,
    width: u16,
    top: bool,
    label: Option<&str>,
    style: CellStyle,
) -> VisualRow {
    let (left, right) = if top { ("╭", "╮") } else { ("╰", "╯") };
    let available = usize::from(width.saturating_sub(2));
    let middle = label
        .filter(|label| available >= display_width(label).saturating_add(3))
        .map_or_else(
            || "─".repeat(available),
            |label| {
                let prefix = format!("─ {label} ");
                format!(
                    "{prefix}{}",
                    "─".repeat(available.saturating_sub(display_width(&prefix)))
                )
            },
        );
    VisualRow {
        component_id: id.to_owned(),
        logical_line: 0,
        wrap_index: 0,
        cells: vec![
            StyledCell::new(left, 1, style),
            StyledCell::new(middle, width.saturating_sub(2), style),
            StyledCell::new(right, 1, style),
        ],
    }
}
