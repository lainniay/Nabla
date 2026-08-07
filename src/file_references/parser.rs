use unicode_segmentation::UnicodeSegmentation;

use crate::file_references::model::FileReferenceToken;

pub fn token_at_cursor(text: &str, grapheme_cursor: usize) -> Option<FileReferenceToken> {
    let cursor = text
        .grapheme_indices(true)
        .nth(grapheme_cursor)
        .map_or(text.len(), |(index, _)| index);
    references_including_open(text)
        .into_iter()
        .find(|token| token.range.start <= cursor && cursor <= token.range.end)
}

pub fn references(text: &str) -> Vec<FileReferenceToken> {
    references_including_open(text)
        .into_iter()
        .filter(|token| token.closed && !token.path.is_empty())
        .collect()
}

pub(crate) fn references_including_open(text: &str) -> Vec<FileReferenceToken> {
    let mut result = Vec::new();
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'@'
            || bytes.get(index + 1) == Some(&b'@')
            || (index > 0
                && !text[..index]
                    .chars()
                    .next_back()
                    .is_none_or(char::is_whitespace))
        {
            index += 1;
            continue;
        }
        let start = index;
        index += 1;
        let braced = bytes.get(index) == Some(&b'{');
        if braced {
            index += 1;
            let path_start = index;
            while index < bytes.len() && bytes[index] != b'}' && bytes[index] != b'\n' {
                index += 1;
            }
            let closed = bytes.get(index) == Some(&b'}');
            let path = text[path_start..index].to_owned();
            if closed {
                index += 1;
            }
            result.push(FileReferenceToken {
                range: start..index,
                path,
                braced,
                closed,
            });
        } else {
            let path_start = index;
            while index < bytes.len() && !bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            let path = text[path_start..index].to_owned();
            result.push(FileReferenceToken {
                range: start..index,
                path,
                braced: false,
                closed: true,
            });
        }
    }
    result
}

pub fn reference_tokens(text: &str) -> Vec<FileReferenceToken> {
    references_including_open(text)
}

pub fn completion_text(path: &str) -> String {
    if path.chars().any(char::is_whitespace) {
        format!("@{{{path}}}")
    } else {
        format!("@{path}")
    }
}
