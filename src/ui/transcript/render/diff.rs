use crate::state::{ToolDiff, ToolDiffFile, ToolDiffLine, ToolDiffLineKind};
use crate::ui::{
    palette,
    types::{CellStyle, Color, StyledCell, VisualRow},
};

use super::{
    common::{cells_width, row_from_cells, single_line_row, styled_cells},
    tool::ToolRenderMode,
};

const COMPACT_DIFF_LINES_PER_FILE: usize = 40;
pub(crate) fn render_tool_diff(
    id: &str,
    diff: &ToolDiff,
    width: u16,
    mode: ToolRenderMode,
) -> Vec<VisualRow> {
    let mut heading = styled_cells("• ", CellStyle::foreground(palette::MAUVE).bold());
    heading.extend(styled_cells(
        &format!(
            "Edited {} {} ",
            diff.files.len(),
            if diff.files.len() == 1 {
                "file"
            } else {
                "files"
            }
        ),
        CellStyle::foreground(Color::White).bold(),
    ));
    append_diff_stats(&mut heading, diff.additions, diff.deletions);
    let mut rows = vec![row_from_cells(id, heading, width)];
    if mode == ToolRenderMode::Summary {
        return rows;
    }

    for file in &diff.files {
        rows.push(render_diff_file_heading(id, file, width));
        let visible = if mode == ToolRenderMode::Expanded {
            file.lines.len()
        } else {
            file.lines.len().min(COMPACT_DIFF_LINES_PER_FILE)
        };
        let line_number_width = file
            .lines
            .iter()
            .filter_map(|line| line.line_number)
            .map(|line| line.to_string().len())
            .max()
            .unwrap_or(1);
        rows.extend(
            file.lines
                .iter()
                .take(visible)
                .map(|line| render_diff_line(id, line, line_number_width, width)),
        );
        let omitted = file.lines.len().saturating_sub(visible);
        if omitted > 0 {
            rows.push(single_line_row(
                id,
                &format!("    … {omitted} more diff lines · expand in Ctrl+O"),
                CellStyle::foreground(palette::GRAY_MUTED).dim(),
                width,
            ));
        }
    }
    rows
}

fn render_diff_file_heading(id: &str, file: &ToolDiffFile, width: u16) -> VisualRow {
    let mut cells = styled_cells("  └ ", CellStyle::foreground(palette::GRAY_MUTED).dim());
    cells.extend(styled_cells(
        &sanitize_diff_fragment(&file.path),
        CellStyle::foreground(Color::White),
    ));
    cells.push(StyledCell::new(
        " ",
        1,
        CellStyle::foreground(palette::GRAY_MUTED),
    ));
    append_diff_stats(&mut cells, file.additions, file.deletions);
    row_from_cells(id, cells, width)
}

fn append_diff_stats(cells: &mut Vec<StyledCell>, additions: usize, deletions: usize) {
    cells.extend(styled_cells(
        "(",
        CellStyle::foreground(palette::GRAY_MUTED).dim(),
    ));
    cells.extend(styled_cells(
        &format!("+{additions}"),
        CellStyle::foreground(palette::GREEN),
    ));
    cells.extend(styled_cells(
        " ",
        CellStyle::foreground(palette::GRAY_MUTED).dim(),
    ));
    cells.extend(styled_cells(
        &format!("-{deletions}"),
        CellStyle::foreground(palette::RED),
    ));
    cells.extend(styled_cells(
        ")",
        CellStyle::foreground(palette::GRAY_MUTED).dim(),
    ));
}

fn render_diff_line(
    id: &str,
    line: &ToolDiffLine,
    line_number_width: usize,
    width: u16,
) -> VisualRow {
    if line.kind == ToolDiffLineKind::Omission {
        return single_line_row(
            id,
            &format!(
                "    {:line_number_width$}  {}",
                "",
                sanitize_diff_fragment(&line.text)
            ),
            CellStyle::foreground(palette::GRAY_MUTED).dim(),
            width,
        );
    }

    let number = line
        .line_number
        .map_or_else(String::new, |number| number.to_string());
    let mut cells = styled_cells(
        &format!("    {number:>line_number_width$} "),
        CellStyle::foreground(palette::GRAY_MUTED).dim(),
    );
    let (marker, style, background) = match line.kind {
        ToolDiffLineKind::Addition => (
            "+",
            CellStyle::foreground(palette::GREEN),
            Some(palette::DIFF_ADDED_BACKGROUND),
        ),
        ToolDiffLineKind::Deletion => (
            "-",
            CellStyle::foreground(palette::RED),
            Some(palette::DIFF_REMOVED_BACKGROUND),
        ),
        ToolDiffLineKind::Context => (" ", CellStyle::foreground(palette::SUBTEXT_0).dim(), None),
        ToolDiffLineKind::Omission => unreachable!(),
    };
    cells.extend(styled_cells(marker, style.bold()));
    cells.extend(styled_cells(&sanitize_diff_fragment(&line.text), style));
    if let Some(background) = background {
        for cell in &mut cells {
            cell.style.background = background;
        }
        let used = cells_width(&cells).min(width);
        let padding = width.saturating_sub(used);
        if padding > 0 {
            cells.push(StyledCell::new(
                " ".repeat(usize::from(padding)),
                padding,
                CellStyle {
                    background,
                    ..CellStyle::default()
                },
            ));
        }
    }
    row_from_cells(id, cells, width)
}

fn sanitize_diff_fragment(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| match character {
            '\t' => "    ".chars().collect::<Vec<_>>(),
            character if character.is_control() => vec!['�'],
            character => vec![character],
        })
        .collect()
}
