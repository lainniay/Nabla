use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::types::{CellStyle, StyledCell, VisualRow};
use crate::file_references::reference_tokens;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct GraphemeIndex(pub usize);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct DisplayColumn(pub usize);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct ByteOffset(pub usize);

pub fn grapheme_count(text: &str) -> usize {
    text.graphemes(true).count()
}

pub fn grapheme_to_byte(text: &str, index: GraphemeIndex) -> ByteOffset {
    ByteOffset(
        text.grapheme_indices(true)
            .nth(index.0)
            .map_or(text.len(), |(byte, _)| byte),
    )
}

pub fn byte_to_grapheme(text: &str, offset: ByteOffset) -> GraphemeIndex {
    let byte = offset.0.min(text.len());
    let byte = (0..=byte)
        .rev()
        .find(|candidate| text.is_char_boundary(*candidate))
        .unwrap_or(0);
    GraphemeIndex(text[..byte].graphemes(true).count())
}

pub fn grapheme_to_column(text: &str, index: GraphemeIndex) -> DisplayColumn {
    DisplayColumn(
        text.graphemes(true)
            .take(index.0)
            .map(UnicodeWidthStr::width)
            .sum(),
    )
}

pub fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

pub(crate) fn styled_cells(text: &str, style: CellStyle) -> Vec<StyledCell> {
    text.graphemes(true)
        .map(|grapheme| {
            let width = display_width(grapheme).max(1);
            StyledCell::new(grapheme, u16::try_from(width).unwrap_or(u16::MAX), style)
        })
        .collect()
}

pub(crate) fn cells_width(cells: &[StyledCell]) -> u16 {
    cells
        .iter()
        .fold(0u16, |total, cell| total.saturating_add(cell.width))
}

pub(crate) fn clip_cells(cells: Vec<StyledCell>, width: u16) -> Vec<StyledCell> {
    let mut used = 0u16;
    cells
        .into_iter()
        .take_while(|cell| {
            let fits = used.saturating_add(cell.width) <= width;
            if fits {
                used = used.saturating_add(cell.width);
            }
            fits
        })
        .collect()
}

pub(crate) fn plain_cells(cells: &[StyledCell]) -> String {
    cells.iter().map(|cell| cell.symbol.as_str()).collect()
}

pub(crate) fn take_graphemes_by_width(text: &str, width: usize) -> (&str, &str) {
    if text.is_empty() || width == 0 {
        return ("", text);
    }
    let mut used = 0usize;
    let mut end = 0usize;
    for (byte, grapheme) in text.grapheme_indices(true) {
        let next = used.saturating_add(display_width(grapheme));
        if next > width {
            break;
        }
        used = next;
        end = byte + grapheme.len();
    }
    if end == 0 {
        let grapheme = text.graphemes(true).next().unwrap_or_default();
        end = grapheme.len();
    }
    (&text[..end], &text[end..])
}

pub fn truncate(text: &str, width: usize) -> String {
    let mut used = 0usize;
    text.graphemes(true)
        .take_while(|grapheme| {
            let next = used.saturating_add(UnicodeWidthStr::width(*grapheme));
            if next > width {
                false
            } else {
                used = next;
                true
            }
        })
        .collect()
}

pub fn wrap_text(component_id: &str, text: &str, width: u16, style: CellStyle) -> Vec<VisualRow> {
    let lines = text
        .split('\n')
        .map(|line| styled_cells(line, style))
        .collect::<Vec<_>>();
    wrap_styled_lines(component_id, &lines, width)
}

pub fn wrap_file_references(
    component_id: &str,
    text: &str,
    width: u16,
    style: CellStyle,
) -> Vec<VisualRow> {
    let tokens = reference_tokens(text);
    if tokens.is_empty() {
        return wrap_text(component_id, text, width, style);
    }
    let mut lines = vec![Vec::new()];
    for (byte, grapheme) in text.grapheme_indices(true) {
        if grapheme == "\n" {
            lines.push(Vec::new());
            continue;
        }
        let token = tokens
            .iter()
            .find(|token| token.range.start <= byte && byte < token.range.end);
        let cell_style = match token {
            Some(token) if token.closed => CellStyle::foreground(crate::ui::palette::TEAL).bold(),
            Some(_) => CellStyle::foreground(crate::ui::palette::LAVENDER).bold(),
            None => style,
        };
        let grapheme_width = UnicodeWidthStr::width(grapheme).max(1);
        lines
            .last_mut()
            .expect("at least one line")
            .push(StyledCell::new(
                grapheme,
                grapheme_width.min(usize::from(u16::MAX)) as u16,
                cell_style,
            ));
    }
    wrap_styled_lines(component_id, &lines, width)
}

pub fn wrap_styled_lines(
    component_id: &str,
    logical_lines: &[Vec<StyledCell>],
    width: u16,
) -> Vec<VisualRow> {
    let width = width.max(1);
    let mut rows = Vec::new();
    for (logical_line, line) in logical_lines.iter().enumerate() {
        if line.is_empty() {
            rows.push(VisualRow {
                component_id: component_id.to_owned(),
                logical_line,
                wrap_index: 0,
                cells: Vec::new(),
            });
            continue;
        }
        let mut current = Vec::new();
        let mut current_width = 0u16;
        let mut wrap_index = 0usize;
        for cell in line {
            if !current.is_empty() && current_width.saturating_add(cell.width) > width {
                rows.push(VisualRow {
                    component_id: component_id.to_owned(),
                    logical_line,
                    wrap_index,
                    cells: std::mem::take(&mut current),
                });
                current_width = 0;
                wrap_index += 1;
            }
            current.push(cell.clone());
            current_width = current_width.saturating_add(cell.width);
            if current_width >= width {
                rows.push(VisualRow {
                    component_id: component_id.to_owned(),
                    logical_line,
                    wrap_index,
                    cells: std::mem::take(&mut current),
                });
                current_width = 0;
                wrap_index += 1;
            }
        }
        if !current.is_empty() {
            rows.push(VisualRow {
                component_id: component_id.to_owned(),
                logical_line,
                wrap_index,
                cells: current,
            });
        }
    }
    if rows.is_empty() {
        rows.push(VisualRow::blank(component_id));
    }
    rows
}

/// Returns the wrapped row and terminal column for a grapheme cursor.
pub fn cursor_geometry(text: &str, cursor: GraphemeIndex, width: u16) -> (usize, usize) {
    let width = usize::from(width.max(1));
    let mut row = 0usize;
    let mut column = 0usize;
    for grapheme in text.graphemes(true).take(cursor.0) {
        if grapheme == "\n" {
            row += 1;
            column = 0;
            continue;
        }
        let grapheme_width = UnicodeWidthStr::width(grapheme).max(1);
        if column > 0 && column.saturating_add(grapheme_width) > width {
            row += 1;
            column = 0;
        }
        column = column.saturating_add(grapheme_width);
        if column >= width {
            row += column / width;
            column %= width;
        }
    }
    (row, column)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_columns_and_wraps_keep_unicode_domains_separate() {
        let text = "你e\u{301}👩🏽‍💻🇨🇳x";
        assert_eq!(grapheme_count(text), 5);
        let emoji = grapheme_to_byte(text, GraphemeIndex(2));
        assert_eq!(&text[emoji.0..], "👩🏽‍💻🇨🇳x");
        assert_eq!(byte_to_grapheme(text, emoji), GraphemeIndex(2));
        assert_eq!(grapheme_to_column(text, GraphemeIndex(2)), DisplayColumn(3));

        let rows = wrap_text("unicode", text, 4, CellStyle::default());
        assert_eq!(
            rows.iter().map(VisualRow::plain_text).collect::<String>(),
            text
        );
        assert!(rows.iter().all(|row| row.display_width() <= 4));
    }

    #[test]
    fn cursor_geometry_handles_combining_zwj_skin_tone_variation_and_flags() {
        let samples = [
            ("e\u{301}", 1usize),
            ("👩‍💻", 2),
            ("👍🏽", 2),
            ("✈️", 2),
            ("🇨🇳", 2),
        ];
        for (sample, expected_width) in samples {
            assert_eq!(grapheme_count(sample), 1, "{sample}");
            assert_eq!(
                grapheme_to_column(sample, GraphemeIndex(1)),
                DisplayColumn(expected_width),
                "{sample}"
            );
        }
    }

    #[test]
    fn truncation_never_splits_a_grapheme_cluster() {
        assert_eq!(truncate("a👩🏽‍💻b", 2), "a");
        assert_eq!(truncate("a👩🏽‍💻b", 3), "a👩🏽‍💻");
    }

    #[test]
    fn file_references_use_teal_and_open_braces_use_lavender() {
        let rows = wrap_file_references(
            "refs",
            "read @src/lib.rs and @{draft file",
            80,
            CellStyle::default(),
        );
        let cells = rows
            .into_iter()
            .flat_map(|row| row.cells)
            .collect::<Vec<_>>();
        let teal = cells
            .iter()
            .filter(|cell| cell.style.foreground == crate::ui::palette::TEAL)
            .map(|cell| cell.symbol.as_str())
            .collect::<String>();
        let lavender = cells
            .iter()
            .filter(|cell| cell.style.foreground == crate::ui::palette::LAVENDER)
            .map(|cell| cell.symbol.as_str())
            .collect::<String>();
        assert_eq!(teal, "@src/lib.rs");
        assert_eq!(lavender, "@{draft file");
    }

    #[test]
    fn styled_wrapping_preserves_styles_and_unicode_cells() {
        let style = CellStyle::foreground(super::super::types::Color::Cyan);
        let cells = wrap_text("source", "a你好b", 20, style).remove(0).cells;
        let rows = wrap_styled_lines("styled", &[cells], 3);
        assert_eq!(
            rows.iter().map(VisualRow::plain_text).collect::<String>(),
            "a你好b"
        );
        assert!(rows.iter().all(|row| row.display_width() <= 3));
        assert!(
            rows.iter()
                .flat_map(|row| &row.cells)
                .all(|cell| cell.style.foreground == super::super::types::Color::Cyan)
        );
    }
}
