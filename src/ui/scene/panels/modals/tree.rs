use crate::{
    state::{TreeItem, TreePhase, UiModalKind},
    ui::{
        palette,
        scene::{append_text_cells, cells_width, text_row, view_model::SceneViewModel},
        selector::VirtualList,
        text::truncate,
        types::{CellStyle, Color, StyledCell, VisualRow},
    },
};

use super::super::{aligned_panel_row, append_choice_window, choice_row};

pub(crate) fn rows(view: &SceneViewModel, width: u16, height: u16) -> Vec<VisualRow> {
    let mut rows = Vec::new();
    if let Some(UiModalKind::TreeBrowser) = view.active_modal_kind() {
        rows.push(text_row(
            "tree-browser",
            "Session tree",
            CellStyle::foreground(palette::TEXT).bold(),
            width,
        ));
        if let Some(browser) = view.tree_browser.as_ref() {
            rows.push(text_row(
                "tree-browser",
                &format!(
                    "{} filter · {} entries",
                    browser.filter_mode.label(),
                    browser.items.len()
                ),
                CellStyle::foreground(Color::Gray).dim(),
                width,
            ));
            match &browser.phase {
                TreePhase::ChooseSummary { selected, .. } => {
                    rows.push(text_row(
                        "tree-browser",
                        "How should Nabla preserve the abandoned branch?",
                        CellStyle::foreground(Color::Yellow).bold(),
                        width,
                    ));
                    let choices = [
                        ("Navigate directly", "Do not create a branch summary"),
                        ("Generate summary", "Summarize the abandoned branch"),
                        ("Custom summary", "Provide summary instructions"),
                    ]
                    .iter()
                    .enumerate()
                    .map(|(index, (label, description))| {
                        choice_row(
                            "tree-browser",
                            label,
                            description,
                            index == *selected,
                            width,
                        )
                    })
                    .collect();
                    append_choice_window(&mut rows, choices, *selected, height);
                }
                TreePhase::Navigating {
                    summarizing,
                    aborting,
                    ..
                } => rows.push(text_row(
                    "tree-browser",
                    if *aborting {
                        "Cancelling tree navigation…"
                    } else if *summarizing {
                        "Summarizing branch before navigation…"
                    } else {
                        "Navigating session tree…"
                    },
                    CellStyle::foreground(Color::Cyan),
                    width,
                )),
                _ => {
                    let choices = browser
                        .items
                        .iter()
                        .enumerate()
                        .map(|(index, item)| {
                            tree_choice_rows(item, index == browser.selected, width)
                        })
                        .collect();
                    append_tree_choice_window(&mut rows, choices, browser.selected, height);
                }
            }
        }
    }
    rows
}

fn append_tree_choice_window(
    rows: &mut Vec<VisualRow>,
    choices: Vec<Vec<VisualRow>>,
    selected: usize,
    height: u16,
) {
    let visible_rows = usize::from(height).saturating_sub(rows.len());
    let visible_items = (visible_rows / 2).max(1);
    let range = VirtualList {
        total: choices.len(),
        selected,
        visible_rows: visible_items,
    }
    .visible_range();
    rows.extend(choices[range].iter().flatten().take(visible_rows).cloned());
}

pub(crate) fn tree_choice_rows(item: &TreeItem, selected: bool, width: u16) -> Vec<VisualRow> {
    let subject = tree_subject(item);
    let mut metadata = Vec::<String>::new();
    if let Some(label) = item.label.as_deref() {
        metadata.push(label.to_owned());
    }
    if item.is_active_path {
        metadata.push("active".to_owned());
    }
    if item.is_leaf {
        metadata.push("leaf".to_owned());
    }
    if item.foldable {
        metadata.push(if item.folded {
            "folded".to_owned()
        } else {
            "expanded".to_owned()
        });
    }
    let identity_style = if selected {
        palette::selected()
    } else if item.is_active_path {
        CellStyle::foreground(palette::ACTIVE_PATH).bold()
    } else {
        CellStyle::foreground(tree_identity_color(item)).bold()
    };
    let content_style = if selected {
        palette::selected()
    } else {
        CellStyle::foreground(palette::TEXT)
    };
    let description_style = if selected {
        palette::selected_muted()
    } else {
        CellStyle::foreground(palette::GRAY_MUTED)
    };
    let identity = tree_identity_label(item);
    let heading = aligned_panel_row(
        "tree-browser",
        &format!("• {identity}"),
        &metadata.join(" · "),
        identity_style,
        description_style,
        width,
    );

    let indent = truncate("  └ ", usize::from(width));
    let mut cells = styled_tree_cells(
        &indent,
        if selected {
            palette::selected()
        } else {
            CellStyle::foreground(palette::GRAY_FAINT)
        },
    );
    let branch = truncate(
        &tree_prefix(item),
        usize::from(width.saturating_sub(cells_width(&cells))),
    );
    cells.extend(styled_tree_cells(&branch, identity_style));
    let used = cells_width(&cells);
    let subject = truncate(&subject, usize::from(width.saturating_sub(used)));
    append_text_cells(&mut cells, &subject, content_style);
    vec![
        heading,
        VisualRow {
            component_id: "tree-browser".to_owned(),
            logical_line: 1,
            wrap_index: 0,
            cells,
        },
    ]
}

fn tree_subject(item: &TreeItem) -> String {
    let mut preview = item.preview.trim();
    if let Some(label) = item.label.as_deref() {
        let label_prefix = format!("[{label}]");
        if preview
            .get(..label_prefix.len())
            .is_some_and(|prefix| prefix == label_prefix)
        {
            preview = preview[label_prefix.len()..].trim_start();
        }
    }
    let identity = item.role.as_deref().unwrap_or(&item.kind);
    let prefix_length = identity.len().saturating_add(1);
    if preview.len() >= prefix_length
        && preview
            .get(..identity.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(identity))
        && preview.as_bytes().get(identity.len()) == Some(&b':')
    {
        return preview[prefix_length..].trim_start().to_owned();
    }
    preview.to_owned()
}

fn tree_identity_label(item: &TreeItem) -> String {
    match item.role.as_deref().unwrap_or(&item.kind) {
        "toolResult" | "tool_result" => "Tool result".to_owned(),
        "toolCall" | "tool_call" => "Tool call".to_owned(),
        "branch_summary" => "Branch summary".to_owned(),
        "custom_message" => "Custom message".to_owned(),
        "model_change" => "Model change".to_owned(),
        "thinking_level_change" => "Thinking level".to_owned(),
        "session_info" => "Session info".to_owned(),
        identity => identity
            .split(['_', '-'])
            .map(|part| {
                let mut chars = part.chars();
                chars.next().map_or_else(String::new, |first| {
                    first.to_uppercase().collect::<String>() + chars.as_str()
                })
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

fn styled_tree_cells(text: &str, style: CellStyle) -> Vec<StyledCell> {
    let mut cells = Vec::new();
    append_text_cells(&mut cells, text, style);
    cells
}

pub(crate) fn tree_identity_color(item: &TreeItem) -> Color {
    match item.role.as_deref().unwrap_or(&item.kind) {
        "user" => palette::BLUE,
        "assistant" | "agent" => palette::MAUVE,
        "tool" | "toolCall" | "tool_call" => palette::TEAL,
        "toolResult" | "tool_result" => palette::PEACH,
        "system" => palette::YELLOW,
        "custom" | "custom_message" => palette::PINK,
        "compaction" => palette::RED,
        "branch_summary" => palette::GREEN,
        "label" => palette::ROSEWATER,
        "model_change" => palette::SAPPHIRE,
        "thinking_level_change" => palette::LAVENDER,
        "session_info" => palette::SKY,
        _ => palette::SAPPHIRE,
    }
}

pub(crate) fn tree_prefix(item: &TreeItem) -> String {
    let depth = item.visual_depth;
    let ancestor_count = depth.saturating_sub(1);
    let mut prefix = String::new();
    let start = if depth > 4 {
        prefix.push_str("… ");
        ancestor_count.saturating_sub(2)
    } else {
        0
    };
    for position in start..ancestor_count {
        prefix.push_str(if item.gutter_positions.contains(&position) {
            "│ "
        } else {
            "  "
        });
    }
    if depth > 0 {
        prefix.push_str(if item.show_connector {
            if item.is_last { "└─" } else { "├─" }
        } else {
            "  "
        });
    }
    prefix.push_str(if item.foldable {
        if item.folded { "▸ " } else { "▾ " }
    } else {
        "· "
    });
    prefix
}
