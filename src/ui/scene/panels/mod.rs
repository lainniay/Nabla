use crate::{
    state::UiModalKind,
    ui::{
        palette,
        panel::PanelRequest,
        scene::{append_text_cells, text_row, view_model::SceneViewModel},
        selector::VirtualList,
        store::UiState,
        text::{display_width, truncate},
        types::{CellStyle, Color, StyledCell, VisualRow},
    },
};

pub(crate) mod approval;
pub(crate) mod completion;
pub(crate) mod modals;
pub(crate) mod selection;

pub(crate) fn primary_panel_request(view: &SceneViewModel, width: u16) -> Option<PanelRequest> {
    let width = width.saturating_sub(2).max(1);
    match view.active_modal_kind() {
        None => completion::completion_panel_request(view, width),
        Some(UiModalKind::Approval) => approval::approval_modal(view, width),
        Some(UiModalKind::Permissions) => approval::permissions_modal(view, width),
        Some(UiModalKind::Question) => modals::question::question_modal(view, width),
        Some(
            UiModalKind::Selection
            | UiModalKind::AgentPicker
            | UiModalKind::Integration
            | UiModalKind::PlanReview,
        ) => selection::selection_modal(view, width),
        _ => None,
    }
}

pub(crate) fn panel_choice_row(
    id: &str,
    label: &str,
    description: &str,
    selected: bool,
    enabled: bool,
    width: u16,
) -> VisualRow {
    let label_style = if selected {
        palette::selected()
    } else if enabled {
        CellStyle::foreground(Color::White)
    } else {
        CellStyle::foreground(Color::Gray).dim()
    };
    let description_style = if selected {
        palette::selected_muted()
    } else {
        CellStyle::foreground(Color::Gray).dim()
    };
    aligned_panel_row(
        id,
        label,
        description,
        label_style,
        description_style,
        width,
    )
}

pub(crate) fn aligned_panel_row(
    id: &str,
    label: &str,
    description: &str,
    label_style: CellStyle,
    description_style: CellStyle,
    width: u16,
) -> VisualRow {
    let available = usize::from(width);
    let label = truncate(label, available);
    let label_width = display_width(&label);
    let description_budget = available.saturating_sub(label_width.saturating_add(1));
    let description = if description_budget >= 4 {
        truncate(description, description_budget)
    } else {
        String::new()
    };
    let description_width = display_width(&description);
    let padding = available.saturating_sub(label_width.saturating_add(description_width));
    let mut cells = Vec::new();
    append_text_cells(&mut cells, &label, label_style);
    if padding > 0 {
        cells.push(StyledCell::new(
            " ".repeat(padding),
            u16::try_from(padding).unwrap_or(width),
            label_style,
        ));
    }
    append_text_cells(&mut cells, &description, description_style);
    VisualRow {
        component_id: id.to_owned(),
        logical_line: 0,
        wrap_index: 0,
        cells,
    }
}

pub(crate) fn choice_row(
    id: &str,
    label: &str,
    description: &str,
    selected: bool,
    width: u16,
) -> VisualRow {
    aligned_panel_row(
        id,
        label,
        description,
        if selected {
            palette::selected()
        } else {
            CellStyle::foreground(Color::White)
        },
        if selected {
            palette::selected_muted()
        } else {
            CellStyle::foreground(Color::Gray).dim()
        },
        width,
    )
}

pub(crate) fn alternate_rows(
    view: &SceneViewModel,
    _ui: &UiState,
    width: u16,
    height: u16,
) -> Vec<VisualRow> {
    match view.active_modal_kind() {
        Some(UiModalKind::SessionBrowser) => modals::session::rows(view, width, height),
        Some(UiModalKind::TreeBrowser) => modals::tree::rows(view, width, height),
        Some(UiModalKind::Transcript) => modals::transcript_viewer::rows(view, width, height),
        Some(UiModalKind::Auth) => modals::auth::rows(view, width, height),
        _ => {
            let title_style = CellStyle::foreground(Color::Magenta).bold();
            let rows = vec![
                text_row("alternate", "Nabla", title_style, width),
                text_row(
                    "alternate",
                    "No alternate-screen route is active.",
                    CellStyle::foreground(Color::Gray),
                    width,
                ),
            ];
            rows
        }
    }
}

pub(crate) fn append_choice_window(
    rows: &mut Vec<VisualRow>,
    choices: Vec<VisualRow>,
    selected: usize,
    height: u16,
) {
    let visible_rows = usize::from(height).saturating_sub(rows.len());
    let range = VirtualList {
        total: choices.len(),
        selected,
        visible_rows,
    }
    .visible_range();
    rows.extend(choices[range].iter().cloned());
}
