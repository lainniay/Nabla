use crate::{
    command::COMMAND_MENU_VISIBLE_ROWS,
    ui::{
        panel::PanelRequest,
        scene::{text_row, view_model::SceneViewModel},
        types::{CellStyle, Color},
    },
};

use super::panel_choice_row;

pub(crate) fn completion_panel_request(view: &SceneViewModel, width: u16) -> Option<PanelRequest> {
    match view.active_modal_kind() {
        None => {
            if let Some(completion) = view.file_completion.as_ref() {
                let rows = if let Some(error) = completion.error.as_ref() {
                    vec![text_row(
                        "file-panel",
                        error,
                        CellStyle::foreground(Color::Red),
                        width,
                    )]
                } else if completion.loading && completion.candidates.is_empty() {
                    vec![text_row(
                        "file-panel",
                        "Searching files…",
                        CellStyle::foreground(Color::Gray).dim(),
                        width,
                    )]
                } else {
                    completion
                        .candidates
                        .iter()
                        .enumerate()
                        .map(|(index, candidate)| {
                            panel_choice_row(
                                "file-panel",
                                &candidate.basename,
                                &candidate.parent,
                                index == completion.selected,
                                true,
                                width,
                            )
                        })
                        .collect()
                };
                let height = rows.len().min(COMMAND_MENU_VISIBLE_ROWS);
                return PanelRequest::new(rows, Some(completion.selected), height);
            }
            let rows = view
                .command_candidates()
                .iter()
                .enumerate()
                .map(|(index, command)| {
                    panel_choice_row(
                        "command-panel",
                        &format!("/{}", command.name),
                        &command.description,
                        index == view.command_menu_selected(),
                        true,
                        width,
                    )
                })
                .collect::<Vec<_>>();
            let height = rows.len().min(COMMAND_MENU_VISIBLE_ROWS);
            PanelRequest::new(rows, Some(view.command_menu_selected()), height)
        }
        _ => None,
    }
}
