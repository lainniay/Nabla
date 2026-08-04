use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDiff {
    pub files: Vec<ToolDiffFile>,
    pub additions: usize,
    pub deletions: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDiffFile {
    pub path: String,
    pub lines: Vec<ToolDiffLine>,
    pub additions: usize,
    pub deletions: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolDiffLineKind {
    Context,
    Addition,
    Deletion,
    Omission,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDiffLine {
    pub kind: ToolDiffLineKind,
    pub line_number: Option<usize>,
    pub text: String,
}

pub fn parse_tool_diff(args: &Value, details: &Value) -> Option<ToolDiff> {
    let details = details.as_object()?;
    let display_diff = details.get("diff").and_then(Value::as_str);
    let patch = details.get("patch").and_then(Value::as_str);
    let parsed_patch = patch
        .filter(|patch| !patch.trim().is_empty())
        .and_then(parse_unified_patch);

    if parsed_patch
        .as_ref()
        .is_some_and(|diff| diff.files.len() > 1)
    {
        return parsed_patch;
    }

    if let (Some(path), Some(display_diff)) = (tool_path(args), display_diff)
        && !display_diff.trim().is_empty()
    {
        return finish_diff(vec![parse_display_diff(path, display_diff)]);
    }

    parsed_patch
}

fn tool_path(args: &Value) -> Option<String> {
    let args = args.as_object()?;
    ["path", "filePath", "file_path", "file", "target"]
        .iter()
        .find_map(|key| args.get(*key).and_then(Value::as_str))
        .filter(|path| !path.is_empty())
        .map(ToOwned::to_owned)
}

fn parse_display_diff(path: String, diff: &str) -> ToolDiffFile {
    let lines = diff.lines().map(parse_display_line).collect::<Vec<_>>();
    finish_file(path, lines)
}

fn parse_display_line(line: &str) -> ToolDiffLine {
    let Some(prefix) = line.chars().next() else {
        return ToolDiffLine {
            kind: ToolDiffLineKind::Context,
            line_number: None,
            text: String::new(),
        };
    };
    let rest = &line[prefix.len_utf8()..];
    let trimmed = rest.trim_start();
    if trimmed == "..." {
        return ToolDiffLine {
            kind: ToolDiffLineKind::Omission,
            line_number: None,
            text: "...".to_owned(),
        };
    }
    let digits = trimmed.bytes().take_while(u8::is_ascii_digit).count();
    let line_number = (digits > 0)
        .then(|| trimmed[..digits].parse::<usize>().ok())
        .flatten();
    let text = trimmed
        .get(digits..)
        .unwrap_or_default()
        .strip_prefix(' ')
        .unwrap_or_else(|| trimmed.get(digits..).unwrap_or_default())
        .to_owned();
    let kind = match prefix {
        '+' => ToolDiffLineKind::Addition,
        '-' => ToolDiffLineKind::Deletion,
        _ => ToolDiffLineKind::Context,
    };
    ToolDiffLine {
        kind,
        line_number,
        text,
    }
}

fn parse_unified_patch(patch: &str) -> Option<ToolDiff> {
    let mut files = Vec::new();
    let mut current_path = None;
    let mut current_lines = Vec::new();
    let mut old_line = 0usize;
    let mut new_line = 0usize;
    let mut in_hunk = false;

    let flush = |files: &mut Vec<ToolDiffFile>,
                 current_path: &mut Option<String>,
                 current_lines: &mut Vec<ToolDiffLine>| {
        if let Some(path) = current_path.take()
            && !current_lines.is_empty()
        {
            files.push(finish_file(path, std::mem::take(current_lines)));
        }
    };

    let lines = patch.lines().collect::<Vec<_>>();
    let mut index = 0usize;
    while index < lines.len() {
        let line = lines[index];
        if let Some(paths) = line.strip_prefix("diff --git ") {
            flush(&mut files, &mut current_path, &mut current_lines);
            current_path = split_git_words(paths)
                .pop()
                .as_deref()
                .map(patch_path)
                .filter(|path| path != "/dev/null");
            in_hunk = false;
            index += 1;
            continue;
        }
        if let Some(old_header) = line.strip_prefix("--- ")
            && let Some(new_header) = lines
                .get(index.saturating_add(1))
                .and_then(|line| line.strip_prefix("+++ "))
        {
            flush(&mut files, &mut current_path, &mut current_lines);
            let old_path = patch_path(old_header);
            let new_path = patch_path(new_header);
            current_path = Some(new_path)
                .filter(|path| path.as_str() != "/dev/null")
                .or_else(|| (old_path != "/dev/null").then_some(old_path));
            in_hunk = false;
            index = index.saturating_add(2);
            continue;
        }
        if let Some((old_start, new_start)) = parse_hunk_header(line) {
            if !current_lines.is_empty() {
                current_lines.push(ToolDiffLine {
                    kind: ToolDiffLineKind::Omission,
                    line_number: None,
                    text: "...".to_owned(),
                });
            }
            old_line = old_start;
            new_line = new_start;
            in_hunk = true;
            index += 1;
            continue;
        }
        if !in_hunk {
            if current_path.is_some()
                && (line == "GIT binary patch" || line.starts_with("Binary files "))
            {
                current_lines.push(ToolDiffLine {
                    kind: ToolDiffLineKind::Omission,
                    line_number: None,
                    text: "binary patch".to_owned(),
                });
            }
            index += 1;
            continue;
        }
        let Some(prefix) = line.chars().next() else {
            index += 1;
            continue;
        };
        let text = line[prefix.len_utf8()..].to_owned();
        match prefix {
            ' ' => {
                current_lines.push(ToolDiffLine {
                    kind: ToolDiffLineKind::Context,
                    line_number: Some(new_line),
                    text,
                });
                old_line = old_line.saturating_add(1);
                new_line = new_line.saturating_add(1);
            }
            '+' => {
                current_lines.push(ToolDiffLine {
                    kind: ToolDiffLineKind::Addition,
                    line_number: Some(new_line),
                    text,
                });
                new_line = new_line.saturating_add(1);
            }
            '-' => {
                current_lines.push(ToolDiffLine {
                    kind: ToolDiffLineKind::Deletion,
                    line_number: Some(old_line),
                    text,
                });
                old_line = old_line.saturating_add(1);
            }
            '\\' => current_lines.push(ToolDiffLine {
                kind: ToolDiffLineKind::Omission,
                line_number: None,
                text,
            }),
            _ => in_hunk = false,
        }
        index += 1;
    }
    flush(&mut files, &mut current_path, &mut current_lines);
    finish_diff(files)
}

fn parse_hunk_header(line: &str) -> Option<(usize, usize)> {
    let body = line.strip_prefix("@@ ")?;
    let mut parts = body.split_whitespace();
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    Some((range_start(old)?, range_start(new)?))
}

fn range_start(value: &str) -> Option<usize> {
    value.split(',').next()?.parse().ok()
}

fn patch_path(value: &str) -> String {
    let path = value.split_once('\t').map_or(value, |(path, _)| path);
    let unquoted = if path.starts_with('"') && path.ends_with('"') && path.len() >= 2 {
        decode_git_quoted(&path[1..path.len() - 1])
    } else {
        path.to_owned()
    };
    unquoted
        .strip_prefix("a/")
        .or_else(|| unquoted.strip_prefix("b/"))
        .unwrap_or(&unquoted)
        .to_owned()
}

fn split_git_words(value: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        match character {
            '"' => quoted = !quoted,
            '\\' if quoted => {
                let Some(escaped) = characters.next() else {
                    current.push('\\');
                    break;
                };
                current.push(decode_git_escape(escaped));
            }
            character if character.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            character => current.push(character),
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn decode_git_quoted(value: &str) -> String {
    let mut decoded = String::new();
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character == '\\' {
            decoded.push(characters.next().map_or('\\', decode_git_escape));
        } else {
            decoded.push(character);
        }
    }
    decoded
}

fn decode_git_escape(character: char) -> char {
    match character {
        't' => '\t',
        'n' => '\n',
        'r' => '\r',
        character => character,
    }
}

fn finish_file(path: String, lines: Vec<ToolDiffLine>) -> ToolDiffFile {
    let additions = lines
        .iter()
        .filter(|line| line.kind == ToolDiffLineKind::Addition)
        .count();
    let deletions = lines
        .iter()
        .filter(|line| line.kind == ToolDiffLineKind::Deletion)
        .count();
    ToolDiffFile {
        path,
        lines,
        additions,
        deletions,
    }
}

fn finish_diff(files: Vec<ToolDiffFile>) -> Option<ToolDiff> {
    if files.is_empty() {
        return None;
    }
    let additions = files.iter().map(|file| file.additions).sum();
    let deletions = files.iter().map(|file| file.deletions).sum();
    Some(ToolDiff {
        files,
        additions,
        deletions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_display_diff_with_stats_and_line_numbers() {
        let diff = parse_tool_diff(
            &json!({"path": "src/lib.rs"}),
            &json!({"diff": " 9 before\n-10 old\n+10 new\n 11 after"}),
        )
        .unwrap();

        assert_eq!(diff.additions, 1);
        assert_eq!(diff.deletions, 1);
        assert_eq!(diff.files[0].path, "src/lib.rs");
        assert_eq!(diff.files[0].lines[1].line_number, Some(10));
        assert_eq!(diff.files[0].lines[2].kind, ToolDiffLineKind::Addition);
    }

    #[test]
    fn parses_multi_file_unified_patch() {
        let patch = "\
--- a/one.txt
+++ b/one.txt
@@ -1,2 +1,2 @@
-old
+new
 same
--- a/two.txt
+++ b/two.txt
@@ -0,0 +1,2 @@
+first
+second
";
        let diff = parse_tool_diff(
            &json!({"path": "fallback.txt"}),
            &json!({"diff": "+1 fallback", "patch": patch}),
        )
        .unwrap();

        assert_eq!(diff.files.len(), 2);
        assert_eq!(diff.additions, 3);
        assert_eq!(diff.deletions, 1);
        assert_eq!(diff.files[0].path, "one.txt");
        assert_eq!(diff.files[1].path, "two.txt");
        assert_eq!(diff.files[1].lines[0].line_number, Some(1));
    }

    #[test]
    fn parses_created_deleted_and_binary_files() {
        let patch = "\
--- /dev/null
+++ b/new.txt
@@ -0,0 +1 @@
+new
--- a/old.txt
+++ /dev/null
@@ -1 +0,0 @@
-old
diff --git \"a/image file.bin\" \"b/image file.bin\"
index 123..456 100644
GIT binary patch
literal 1
";
        let diff = parse_tool_diff(&Value::Null, &json!({"patch": patch})).unwrap();

        assert_eq!(diff.files.len(), 3);
        assert_eq!(diff.files[0].path, "new.txt");
        assert_eq!(diff.files[1].path, "old.txt");
        assert_eq!(diff.files[2].path, "image file.bin");
        assert_eq!(diff.files[2].lines[0].kind, ToolDiffLineKind::Omission);
        assert_eq!((diff.additions, diff.deletions), (1, 1));
    }

    #[test]
    fn malformed_details_are_ignored() {
        assert!(parse_tool_diff(&json!({}), &json!({"diff": "broken"})).is_none());
        assert!(parse_tool_diff(&Value::Null, &json!({"patch": "not a patch"})).is_none());
    }
}
