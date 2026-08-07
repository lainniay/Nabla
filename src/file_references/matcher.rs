use std::path::{Component, Path};

use crate::file_references::model::FileCandidate;

pub(crate) fn match_score(candidate: &FileCandidate, query: &str) -> Option<u8> {
    if query.is_empty() {
        return Some(5);
    }
    let basename = candidate.basename.to_lowercase();
    let path = candidate.path.to_lowercase();
    if basename == query {
        Some(0)
    } else if basename.starts_with(query) {
        Some(1)
    } else if path.split('/').any(|segment| segment.starts_with(query)) {
        Some(2)
    } else if path.contains(query) {
        Some(3)
    } else if is_subsequence(query, &path) {
        Some(4)
    } else {
        None
    }
}

pub(crate) fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut chars = haystack.chars();
    needle
        .chars()
        .all(|needle| chars.by_ref().any(|hay| hay == needle))
}

pub(crate) fn path_depth(path: &str) -> usize {
    path.bytes().filter(|byte| *byte == b'/').count()
}

pub(crate) fn slash_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}
