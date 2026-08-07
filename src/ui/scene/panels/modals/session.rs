use crate::{
    state::UiModalKind,
    ui::{
        scene::{text_row, view_model::SceneViewModel},
        types::{CellStyle, Color, VisualRow},
    },
};

use super::super::{append_choice_window, choice_row};

pub(crate) fn rows(view: &SceneViewModel, width: u16, height: u16) -> Vec<VisualRow> {
    let mut rows = Vec::new();
    let title_style = CellStyle::foreground(Color::Magenta).bold();
    if let Some(UiModalKind::SessionBrowser) = view.active_modal_kind() {
        rows.push(text_row(
            "session-browser",
            "Resume session",
            title_style,
            width,
        ));
        if let Some(browser) = view.session_browser.as_ref() {
            rows.push(text_row(
                "session-browser",
                &format!(
                    "{} · {} results · {}",
                    browser.sort_mode.label(),
                    browser.total,
                    match browser.scope {
                        crate::state::SessionScope::Current => "current workspace",
                        crate::state::SessionScope::All => "all workspaces",
                    }
                ),
                CellStyle::foreground(Color::Gray).dim(),
                width,
            ));
            let choices = browser
                .sessions
                .iter()
                .enumerate()
                .map(|(index, session)| {
                    let indent = if session.depth > 4 {
                        format!("… {}", "  ".repeat(2))
                    } else {
                        "  ".repeat(session.depth)
                    };
                    let description = if session.current {
                        format!("current · {} messages", session.message_count)
                    } else {
                        format!("{} messages", session.message_count)
                    };
                    choice_row(
                        "session-browser",
                        &format!("{indent}{}", session.label()),
                        &description,
                        index == browser.selected,
                        width,
                    )
                })
                .collect();
            append_choice_window(&mut rows, choices, browser.selected, height);
        }
    }
    rows
}
