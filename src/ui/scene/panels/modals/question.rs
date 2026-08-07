use crate::{
    state::UiModalKind,
    ui::{
        panel::PanelRequest,
        scene::{text_row, view_model::SceneViewModel},
        text::wrap_text,
        types::{CellStyle, Color},
    },
};

use super::super::panel_choice_row;

pub(crate) fn question_modal(view: &SceneViewModel, width: u16) -> Option<PanelRequest> {
    match view.active_modal_kind() {
        Some(UiModalKind::Question) => {
            let flow = view.question.as_ref()?;
            let question = flow.current_question()?;
            let mut rows = vec![text_row(
                "question",
                &question.prompt,
                CellStyle::foreground(Color::Cyan).bold(),
                width,
            )];
            rows.extend(question.options.iter().enumerate().map(|(index, option)| {
                panel_choice_row(
                    "question",
                    &option.label,
                    option.description.as_deref().unwrap_or_default(),
                    index == flow.selected,
                    true,
                    width,
                )
            }));
            rows.push(panel_choice_row(
                "question",
                "Custom answer",
                "Type a different response",
                flow.selected == question.options.len(),
                true,
                width,
            ));
            if flow.custom_answer {
                rows.extend(wrap_text(
                    "question-input",
                    flow.editor.text(),
                    width,
                    CellStyle::foreground(Color::White),
                ));
            }
            let height = rows.len().min(view.selection_page_size.saturating_add(2));
            PanelRequest::new(rows, Some(flow.selected.saturating_add(1)), height)
        }
        _ => None,
    }
}
