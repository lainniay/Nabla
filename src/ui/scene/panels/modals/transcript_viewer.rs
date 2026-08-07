use std::collections::HashMap;

use crate::{
    state::{TranscriptItem, TranscriptViewMode, UiModalKind},
    ui::{
        scene::{text_row, view_model::SceneViewModel},
        transcript::render_viewer_item,
        types::{CellStyle, Color, RowRange, VisualRow},
    },
};

pub(crate) fn rows(view: &SceneViewModel, width: u16, height: u16) -> Vec<VisualRow> {
    let mut rows = Vec::new();
    let title_style = CellStyle::foreground(Color::Magenta).bold();
    if let Some(UiModalKind::Transcript) = view.active_modal_kind() {
        rows.push(text_row(
            "transcript-viewer",
            "Transcript viewer",
            title_style,
            width,
        ));
        if let Some(viewer) = view.transcript_viewer.as_ref() {
            rows.push(text_row(
                "transcript-viewer",
                &format!(
                    "{} · {} matches",
                    viewer.mode.label(),
                    viewer.search_matches.len()
                ),
                CellStyle::foreground(Color::Gray).dim(),
                width,
            ));
            let mut body = Vec::new();
            let mut ranges = HashMap::<usize, RowRange>::new();
            for (index, item) in view.transcript.iter().enumerate() {
                if viewer.mode != TranscriptViewMode::Summary
                    && (index == 0
                        || viewer_item_group(&view.transcript[index - 1])
                            != viewer_item_group(item))
                {
                    body.push(VisualRow::blank("transcript-viewer-spacing"));
                }
                let start = body.len();
                let expanded = match item {
                    TranscriptItem::Tool(tool) => viewer
                        .tool_expansion_overrides
                        .get(&tool.id)
                        .copied()
                        .unwrap_or(viewer.mode == TranscriptViewMode::Verbose),
                    _ => false,
                };
                body.extend(render_viewer_item(
                    &format!("viewer:{index}"),
                    item,
                    width,
                    viewer.mode,
                    expanded,
                    viewer.selected_item == Some(index),
                ));
                ranges.insert(
                    index,
                    RowRange {
                        start,
                        end: body.len(),
                    },
                );
            }

            let body_height = usize::from(height).saturating_sub(rows.len());
            let maximum_start = body.len().saturating_sub(body_height);
            let start = if viewer.scroll_to_selected {
                viewer
                    .selected_item
                    .and_then(|selected| ranges.get(&selected))
                    .map(|range| {
                        range
                            .start
                            .saturating_sub(body_height.saturating_sub(1) / 2)
                            .min(maximum_start)
                    })
                    .unwrap_or(maximum_start)
            } else if viewer.follow_tail {
                maximum_start
            } else {
                maximum_start.saturating_sub(viewer.scroll_from_bottom)
            };
            rows.extend(body.into_iter().skip(start).take(body_height));
        }
    }
    rows
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewerItemGroup {
    User,
    Assistant,
    Tool,
    Turn,
    Other,
}

fn viewer_item_group(item: &TranscriptItem) -> ViewerItemGroup {
    match item {
        TranscriptItem::User(_) => ViewerItemGroup::User,
        TranscriptItem::Assistant(_) => ViewerItemGroup::Assistant,
        TranscriptItem::Tool(_) => ViewerItemGroup::Tool,
        TranscriptItem::TurnSeparator(_) => ViewerItemGroup::Turn,
        _ => ViewerItemGroup::Other,
    }
}
