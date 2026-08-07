use crate::{
    state::{AuthPromptKind, AuthState, UiModalKind, matching_auth_choice_indices},
    ui::{
        scene::{text_row, view_model::SceneViewModel},
        types::{CellStyle, Color, VisualRow},
    },
};

use super::super::{append_choice_window, choice_row};

pub(crate) fn rows(view: &SceneViewModel, width: u16, height: u16) -> Vec<VisualRow> {
    let mut rows = Vec::new();
    let title_style = CellStyle::foreground(Color::Magenta).bold();
    if let Some(UiModalKind::Auth) = view.active_modal_kind() {
        rows.push(text_row("auth", "Authentication", title_style, width));
        match &view.auth_state {
            AuthState::Inactive => {}
            AuthState::LoadingProviders => rows.push(text_row(
                "auth",
                "Loading providers…",
                CellStyle::foreground(Color::Gray),
                width,
            )),
            AuthState::Selecting {
                choices,
                selected,
                filter,
                ..
            } => {
                rows.push(text_row(
                    "auth",
                    &format!(
                        "{} providers",
                        matching_auth_choice_indices(choices, filter.text()).len()
                    ),
                    CellStyle::foreground(Color::Gray).dim(),
                    width,
                ));
                let visible_choices = matching_auth_choice_indices(choices, filter.text());
                let choice_rows = visible_choices
                    .into_iter()
                    .enumerate()
                    .map(|(visible_index, choice_index)| {
                        let choice = &choices[choice_index];
                        let description = format!(
                            "{} · {}{}",
                            choice.label,
                            choice.auth_type,
                            if choice.configured {
                                " · configured"
                            } else {
                                ""
                            }
                        );
                        choice_row(
                            "auth",
                            &choice.provider_name,
                            &description,
                            visible_index == *selected,
                            width,
                        )
                    })
                    .collect();
                append_choice_window(&mut rows, choice_rows, *selected, height);
            }
            AuthState::Running(flow) => {
                rows.push(text_row(
                    "auth",
                    &format!("{} · {}", flow.provider_name, flow.status),
                    CellStyle::foreground(Color::Cyan),
                    width,
                ));
                if let Some(code) = flow.device_code.as_ref() {
                    rows.push(text_row(
                        "auth",
                        &format!("Code: {code}"),
                        CellStyle::foreground(Color::Yellow).bold(),
                        width,
                    ));
                }
                if let Some(prompt) = flow.prompt.as_ref() {
                    rows.push(text_row(
                        "auth",
                        &prompt.message,
                        CellStyle::foreground(Color::White),
                        width,
                    ));
                    if prompt.kind == AuthPromptKind::Select {
                        let choice_rows = prompt
                            .options
                            .iter()
                            .enumerate()
                            .map(|(index, option)| {
                                choice_row(
                                    "auth",
                                    &option.label,
                                    option.description.as_deref().unwrap_or_default(),
                                    index == prompt.selected,
                                    width,
                                )
                            })
                            .collect();
                        append_choice_window(&mut rows, choice_rows, prompt.selected, height);
                    }
                }
            }
        }
    }
    rows
}
