use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::{
    palette,
    types::{CellStyle, Color, StyledCell},
};

pub fn highlight(command: &str) -> Vec<Vec<StyledCell>> {
    command.split('\n').map(highlight_line).collect()
}

fn highlight_line(line: &str) -> Vec<StyledCell> {
    let mut cells = Vec::new();
    let mut cursor = 0usize;
    let mut command_position = true;
    while cursor < line.len() {
        let rest = &line[cursor..];
        let character = rest.chars().next().unwrap_or_default();
        if character.is_whitespace() {
            let end = take_while(rest, char::is_whitespace);
            push(
                &mut cells,
                &rest[..end],
                CellStyle::foreground(palette::TEXT),
            );
            cursor += end;
            continue;
        }
        if character == '#' {
            push(&mut cells, rest, CellStyle::foreground(palette::GRAY_FAINT));
            break;
        }
        if let Some((operator, kind)) = shell_operator(rest) {
            let style = match kind {
                OperatorKind::Separator => CellStyle::foreground(palette::RED),
                OperatorKind::Redirection => CellStyle::foreground(palette::TEXT),
            };
            push(&mut cells, operator, style);
            cursor += operator.len();
            command_position = kind == OperatorKind::Separator;
            continue;
        }
        if character == '$' && rest.starts_with("$'") {
            let (end, closed) = match dollar_quoted_end(rest) {
                Some(end) => (end, true),
                None => (rest.len(), false),
            };
            let style = if closed {
                CellStyle::foreground(palette::TEXT)
            } else {
                CellStyle::foreground(palette::MAROON)
            };
            push(&mut cells, &rest[..end], style);
            cursor += end;
            command_position = false;
            continue;
        }
        if matches!(character, '\'' | '"') {
            let (end, closed) = match quoted_end(rest, character) {
                Some(end) => (end, true),
                None => (rest.len(), false),
            };
            let quote_color = if closed {
                palette::YELLOW
            } else {
                palette::MAROON
            };
            if character == '"' {
                push_double_quoted(&mut cells, &rest[..end], quote_color);
            } else {
                push(&mut cells, &rest[..end], CellStyle::foreground(quote_color));
            }
            cursor += end;
            command_position = false;
            continue;
        }
        if character == '$' {
            let end = variable_end(rest);
            push(
                &mut cells,
                &rest[..end],
                CellStyle::foreground(palette::TEXT),
            );
            cursor += end;
            command_position = false;
            continue;
        }

        let end = word_end(rest);
        let word = &rest[..end];
        if assignment_prefix(word) {
            push(&mut cells, word, CellStyle::foreground(palette::TEXT));
        } else if command_position {
            push(&mut cells, word, CellStyle::foreground(palette::GREEN));
        } else if word.starts_with('-') {
            push(&mut cells, word, CellStyle::foreground(palette::PEACH));
        } else if looks_like_path(word) {
            push_path(&mut cells, word);
        } else {
            push(&mut cells, word, CellStyle::foreground(palette::TEXT));
        }
        cursor += end;
        if !assignment_prefix(word) {
            command_position = false;
        }
    }
    cells
}

fn push_double_quoted(cells: &mut Vec<StyledCell>, text: &str, base_color: Color) {
    let mut cursor = 0usize;
    while cursor < text.len() {
        let rest = &text[cursor..];
        let Some(variable) = rest.find('$') else {
            push(cells, rest, CellStyle::foreground(base_color));
            break;
        };
        if variable > 0 {
            push(cells, &rest[..variable], CellStyle::foreground(base_color));
        }
        let variable_text = &rest[variable..];
        let end = variable_end(variable_text).max(1);
        push(
            cells,
            &variable_text[..end],
            CellStyle::foreground(palette::TEXT),
        );
        cursor += variable + end;
    }
}

fn push(cells: &mut Vec<StyledCell>, text: &str, style: CellStyle) {
    cells.extend(text.graphemes(true).map(|grapheme| {
        StyledCell::new(
            grapheme,
            UnicodeWidthStr::width(grapheme)
                .max(1)
                .min(usize::from(u16::MAX)) as u16,
            style,
        )
    }));
}

fn take_while(text: &str, predicate: impl Fn(char) -> bool) -> usize {
    text.char_indices()
        .find_map(|(index, character)| (!predicate(character)).then_some(index))
        .unwrap_or(text.len())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OperatorKind {
    Separator,
    Redirection,
}

fn shell_operator(text: &str) -> Option<(&str, OperatorKind)> {
    const SEPARATORS: &[&str] = &["&&", "||", "|", ";", "&"];
    const REDIRECTIONS: &[&str] = &["2>", ">>", "<<", ">", "<"];
    for operator in SEPARATORS {
        if text.starts_with(*operator) {
            return Some((operator, OperatorKind::Separator));
        }
    }
    for operator in REDIRECTIONS {
        if text.starts_with(*operator) {
            return Some((operator, OperatorKind::Redirection));
        }
    }
    None
}

fn quoted_end(text: &str, quote: char) -> Option<usize> {
    let mut escaped = false;
    for (index, character) in text.char_indices().skip(1) {
        if escaped {
            escaped = false;
        } else if character == '\\' && quote == '"' {
            escaped = true;
        } else if character == quote {
            return Some(index + character.len_utf8());
        }
    }
    None
}

fn dollar_quoted_end(text: &str) -> Option<usize> {
    let mut escaped = false;
    for (index, character) in text.char_indices().skip(2) {
        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '\'' {
            return Some(index + character.len_utf8());
        }
    }
    None
}

fn variable_end(text: &str) -> usize {
    if text.starts_with("${") {
        return text
            .find('}')
            .map_or(text.len(), |index| index.saturating_add(1));
    }
    let tail = &text[1..];
    1 + take_while(tail, |character| {
        character.is_alphanumeric() || character == '_'
    })
}

fn word_end(text: &str) -> usize {
    let mut escaped = false;
    for (index, character) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if index > 0
            && (character.is_whitespace()
                || matches!(character, '\'' | '"' | '$' | '|' | ';' | '>' | '<' | '&'))
        {
            return index;
        }
    }
    text.len()
}

fn assignment_prefix(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && name
            .chars()
            .all(|character| character.is_alphanumeric() || character == '_')
}

fn looks_like_path(word: &str) -> bool {
    word.starts_with('/') || word.starts_with("./") || word.starts_with("../") || word.contains('/')
}

fn push_path(cells: &mut Vec<StyledCell>, word: &str) {
    let mut start = 0usize;
    for (index, character) in word.char_indices() {
        if character == '/' {
            if index > start {
                push(
                    cells,
                    &word[start..index],
                    CellStyle::foreground(palette::TEXT),
                );
            }
            push(cells, "/", CellStyle::foreground(palette::RED));
            start = index + 1;
        }
    }
    if start < word.len() {
        push(cells, &word[start..], CellStyle::foreground(palette::TEXT));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_highlight_classifies_commands_flags_strings_variables_paths_and_operators() {
        let rows = highlight("FOO=bar cargo test --all \"$HOME/x\" ./src | rg '测试' # note");
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0]
                .iter()
                .map(|cell| cell.symbol.as_str())
                .collect::<String>(),
            "FOO=bar cargo test --all \"$HOME/x\" ./src | rg '测试' # note"
        );
        for color in [
            palette::GREEN,
            palette::PEACH,
            palette::YELLOW,
            palette::TEXT,
            palette::RED,
            palette::GRAY_FAINT,
        ] {
            assert!(
                rows[0].iter().any(|cell| cell.style.foreground == color),
                "missing {color:?}"
            );
        }
        assert!(
            rows[0]
                .iter()
                .any(|cell| cell.symbol == "/" && cell.style.foreground == palette::RED)
        );
        assert!(
            rows[0]
                .iter()
                .any(|cell| cell.symbol == "$" && cell.style.foreground == palette::TEXT)
        );
        assert!(
            rows[0]
                .iter()
                .all(|cell| !cell.style.bold && !cell.style.italic && !cell.style.underlined)
        );
    }

    #[test]
    fn unterminated_quotes_and_unicode_are_preserved() {
        let rows = highlight("printf \"你好 $NAME");
        assert_eq!(
            rows[0]
                .iter()
                .map(|cell| cell.symbol.as_str())
                .collect::<String>(),
            "printf \"你好 $NAME"
        );
        assert!(
            rows[0]
                .iter()
                .any(|cell| cell.style.foreground == palette::MAROON)
        );
        assert!(
            rows[0]
                .iter()
                .any(|cell| cell.symbol == "$" && cell.style.foreground == palette::TEXT)
        );
        assert!(
            rows[0]
                .iter()
                .all(|cell| !cell.style.bold && !cell.style.italic && !cell.style.underlined)
        );
    }

    #[test]
    fn dollar_quoted_and_unclosed_quotes_use_theme_colors() {
        let rows = highlight("printf $'ok' $'bad");
        assert_eq!(
            rows[0]
                .iter()
                .map(|cell| cell.symbol.as_str())
                .collect::<String>(),
            "printf $'ok' $'bad"
        );
        assert!(
            rows[0]
                .iter()
                .any(|cell| cell.symbol == "o" && cell.style.foreground == palette::TEXT)
        );
        assert!(
            rows[0]
                .iter()
                .any(|cell| cell.symbol == "b" && cell.style.foreground == palette::MAROON)
        );
        assert!(
            rows[0]
                .iter()
                .all(|cell| !cell.style.bold && !cell.style.italic && !cell.style.underlined)
        );
    }

    #[test]
    fn redirections_and_command_separators_use_theme_colors() {
        let rows = highlight("cargo run > out.txt 2>&1 | tail");
        assert!(
            rows[0]
                .iter()
                .any(|cell| cell.symbol == ">" && cell.style.foreground == palette::TEXT)
        );
        assert!(
            rows[0]
                .iter()
                .any(|cell| cell.symbol == "|" && cell.style.foreground == palette::RED)
        );
        assert!(
            rows[0]
                .iter()
                .any(|cell| cell.symbol == "&" && cell.style.foreground == palette::RED)
        );
    }
}
