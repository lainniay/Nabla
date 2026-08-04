use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::{
    palette,
    types::{CellStyle, StyledCell},
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
        if let Some(operator) = shell_operator(rest) {
            push(
                &mut cells,
                operator,
                CellStyle::foreground(palette::PEACH).bold(),
            );
            cursor += operator.len();
            command_position = matches!(operator, "|" | "||" | "&&" | ";" | "&");
            continue;
        }
        if matches!(character, '\'' | '"') {
            let end = quoted_end(rest, character);
            if character == '"' {
                push_double_quoted(&mut cells, &rest[..end]);
            } else {
                push(
                    &mut cells,
                    &rest[..end],
                    CellStyle::foreground(palette::YELLOW),
                );
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
                CellStyle::foreground(palette::PINK).bold(),
            );
            cursor += end;
            command_position = false;
            continue;
        }

        let end = word_end(rest);
        let word = &rest[..end];
        let style = if assignment_prefix(word) {
            CellStyle::foreground(palette::PINK)
        } else if command_position {
            CellStyle::foreground(palette::SAPPHIRE).bold()
        } else if word.starts_with('-') {
            CellStyle::foreground(palette::BLUE)
        } else if looks_like_path(word) {
            CellStyle::foreground(palette::GREEN)
        } else {
            CellStyle::foreground(palette::TEXT)
        };
        push(&mut cells, word, style);
        cursor += end;
        if !assignment_prefix(word) {
            command_position = false;
        }
    }
    cells
}

fn push_double_quoted(cells: &mut Vec<StyledCell>, text: &str) {
    let mut cursor = 0usize;
    while cursor < text.len() {
        let rest = &text[cursor..];
        let Some(variable) = rest.find('$') else {
            push(cells, rest, CellStyle::foreground(palette::YELLOW));
            break;
        };
        if variable > 0 {
            push(
                cells,
                &rest[..variable],
                CellStyle::foreground(palette::YELLOW),
            );
        }
        let variable_text = &rest[variable..];
        let end = variable_end(variable_text).max(1);
        push(
            cells,
            &variable_text[..end],
            CellStyle::foreground(palette::PINK).bold(),
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

fn shell_operator(text: &str) -> Option<&str> {
    ["&&", "||", ">>", "<<", "2>", "|", ";", ">", "<", "&"]
        .into_iter()
        .find(|operator| text.starts_with(operator))
}

fn quoted_end(text: &str, quote: char) -> usize {
    let mut escaped = false;
    for (index, character) in text.char_indices().skip(1) {
        if escaped {
            escaped = false;
        } else if character == '\\' && quote == '"' {
            escaped = true;
        } else if character == quote {
            return index + character.len_utf8();
        }
    }
    text.len()
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
        assert!(
            rows[0]
                .iter()
                .any(|cell| cell.style.foreground == palette::SAPPHIRE)
        );
        assert!(
            rows[0]
                .iter()
                .any(|cell| cell.style.foreground == palette::BLUE)
        );
        assert!(
            rows[0]
                .iter()
                .any(|cell| cell.style.foreground == palette::YELLOW)
        );
        assert!(
            rows[0]
                .iter()
                .any(|cell| cell.style.foreground == palette::GREEN)
        );
        assert!(
            rows[0]
                .iter()
                .any(|cell| cell.style.foreground == palette::PEACH)
        );
        assert!(
            rows[0]
                .iter()
                .any(|cell| cell.style.foreground == palette::GRAY_FAINT)
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
    }
}
