use super::*;

pub(super) fn is_previous_selection_key(key: KeyEvent) -> bool {
    matches!(
        crate::ui::selection_navigation(key),
        Some(crate::ui::SelectionNavigation::Previous)
    )
}

pub(super) fn is_next_selection_key(key: KeyEvent) -> bool {
    matches!(
        crate::ui::selection_navigation(key),
        Some(crate::ui::SelectionNavigation::Next)
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ChoiceNavAction {
    Handled,
    Confirm(usize),
    Cancel,
    Unhandled,
}

pub(super) fn update_choice_navigation(
    key: KeyEvent,
    selected: &mut usize,
    enabled: &[bool],
) -> ChoiceNavAction {
    if matches!(key.code, KeyCode::Esc) {
        return ChoiceNavAction::Cancel;
    }
    if enabled.is_empty() || !enabled.iter().any(|enabled| *enabled) {
        return ChoiceNavAction::Unhandled;
    }
    if is_previous_selection_key(key) {
        *selected = next_enabled_choice(*selected, enabled, false);
        return ChoiceNavAction::Handled;
    }
    if is_next_selection_key(key) {
        *selected = next_enabled_choice(*selected, enabled, true);
        return ChoiceNavAction::Handled;
    }
    if let KeyCode::Char(character @ '1'..='9') = key.code
        && key.modifiers.is_empty()
    {
        let index = character.to_digit(10).unwrap_or(1) as usize - 1;
        if enabled.get(index).copied().unwrap_or(false) {
            *selected = index;
        }
        return ChoiceNavAction::Handled;
    }
    if key.code == KeyCode::Enter && enabled.get(*selected).copied().unwrap_or(false) {
        return ChoiceNavAction::Confirm(*selected);
    }
    ChoiceNavAction::Unhandled
}

pub(super) fn next_enabled_choice(selected: usize, enabled: &[bool], forward: bool) -> usize {
    let mut index = selected.min(enabled.len().saturating_sub(1));
    for _ in 0..enabled.len() {
        index = if forward {
            next_wrapped(index, enabled.len())
        } else {
            previous_wrapped(index, enabled.len())
        };
        if enabled[index] {
            return index;
        }
    }
    selected
}

pub(super) fn is_missing_credentials(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("no api key found")
        || error.contains("no credentials")
        || (error.contains("/login") && error.contains("api key"))
}

pub(super) fn string_field(value: &serde_json::Value, name: &str) -> Option<String> {
    value.get(name)?.as_str().map(ToOwned::to_owned)
}

pub(super) fn short_session_id(session_id: &str) -> String {
    session_id.chars().take(8).collect()
}

pub(super) fn tree_branch_segment_index(
    items: &[TreeItem],
    selected: usize,
    down: bool,
) -> Option<usize> {
    let index_by_id = items
        .iter()
        .enumerate()
        .map(|(index, item)| (item.entry_id.as_str(), index))
        .collect::<std::collections::HashMap<_, _>>();
    let mut current_id = items.get(selected)?.entry_id.as_str();

    if down {
        loop {
            let children = items
                .iter()
                .enumerate()
                .filter(|(_, item)| item.parent_id.as_deref() == Some(current_id))
                .collect::<Vec<_>>();
            match children.as_slice() {
                [] => return index_by_id.get(current_id).copied(),
                [(index, _)] => current_id = items[*index].entry_id.as_str(),
                [(index, _), ..] => return Some(*index),
            }
        }
    }

    loop {
        let current = items.get(*index_by_id.get(current_id)?)?;
        let Some(parent_id) = current.parent_id.as_deref() else {
            return index_by_id.get(current_id).copied();
        };
        let sibling_count = items
            .iter()
            .filter(|item| item.parent_id.as_deref() == Some(parent_id))
            .count();
        let current_index = *index_by_id.get(current_id)?;
        if sibling_count > 1 && current_index < selected {
            return Some(current_index);
        }
        current_id = parent_id;
    }
}

pub(super) fn tool_result_text(value: &serde_json::Value) -> Option<String> {
    let content = value.get("content")?.as_array()?;
    let text = content
        .iter()
        .filter(|part| part.get("type").and_then(serde_json::Value::as_str) == Some("text"))
        .filter_map(|part| part.get("text").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty()).then_some(text)
}

pub(super) fn parse_compaction_record(
    payload: &serde_json::Value,
) -> Result<CompactionRecord, String> {
    let reason = payload["reason"]
        .as_str()
        .ok_or_else(|| "Compaction event has no reason.".to_owned())?
        .to_owned();
    let result = payload["result"]
        .as_object()
        .ok_or_else(|| "Compaction completed without a result.".to_owned())?;
    let first_kept_entry_id = result
        .get("firstKeptEntryId")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "Compaction result has no first kept entry.".to_owned())?
        .to_owned();
    let tokens_before = result
        .get("tokensBefore")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "Compaction result has no token count.".to_owned())?;
    let estimated_tokens_after = result
        .get("estimatedTokensAfter")
        .and_then(serde_json::Value::as_u64);
    let tokens_saved = estimated_tokens_after.map(|after| tokens_before.saturating_sub(after));
    let saved_percent = tokens_saved.and_then(|saved| {
        (tokens_before > 0).then_some((saved as f64 / tokens_before as f64) * 100.0)
    });
    let details = result.get("details").and_then(serde_json::Value::as_object);
    let read_files = detail_files(details, "readFiles");
    let modified_files = detail_files(details, "modifiedFiles");
    let file_count = read_files
        .iter()
        .chain(&modified_files)
        .collect::<std::collections::HashSet<_>>()
        .len() as u64;

    Ok(CompactionRecord {
        reason,
        first_kept_entry_id,
        tokens_before,
        estimated_tokens_after,
        tokens_saved,
        saved_percent,
        file_count,
        read_file_count: read_files.len() as u64,
        modified_file_count: modified_files.len() as u64,
    })
}

pub(super) fn detail_files(
    details: Option<&serde_json::Map<String, serde_json::Value>>,
    field: &str,
) -> Vec<String> {
    details
        .and_then(|details| details.get(field))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .collect()
}

pub(super) fn compaction_reason_label(reason: &str) -> &'static str {
    match reason {
        "manual" => "Manual",
        "threshold" => "Threshold",
        "overflow" => "Overflow recovery",
        _ => "Context",
    }
}
