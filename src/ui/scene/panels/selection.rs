use crate::{
    state::UiModalKind,
    ui::{
        panel::PanelRequest,
        scene::{text_row, view_model::SceneViewModel},
        types::{CellStyle, Color},
    },
};

use super::panel_choice_row;

pub(crate) fn selection_modal(view: &SceneViewModel, width: u16) -> Option<PanelRequest> {
    match view.active_modal_kind() {
        Some(UiModalKind::Selection) => view.selection_panel.as_ref().and_then(|panel| {
            let mut rows = vec![text_row(
                "selection-panel",
                &panel.title,
                CellStyle::foreground(Color::Cyan).bold(),
                width,
            )];
            if panel.loading {
                rows.push(text_row(
                    "selection-panel",
                    "Loading…",
                    CellStyle::foreground(Color::Gray).dim(),
                    width,
                ));
            } else if panel.options.is_empty() {
                rows.push(text_row(
                    "selection-panel",
                    "No options available",
                    CellStyle::foreground(Color::Gray).dim(),
                    width,
                ));
            } else {
                rows.extend(panel.options.iter().enumerate().map(|(index, option)| {
                    panel_choice_row(
                        "selection-panel",
                        &option.label,
                        &option.description,
                        index == panel.selected,
                        true,
                        width,
                    )
                }));
            }
            let height = rows.len().min(view.selection_page_size.saturating_add(1));
            PanelRequest::new(rows, Some(panel.selected.saturating_add(1)), height)
        }),
        Some(UiModalKind::AgentPicker) => view.agent_picker.as_ref().and_then(|picker| {
            let rows = picker
                .profiles
                .iter()
                .enumerate()
                .map(|(index, profile)| {
                    panel_choice_row(
                        "agent-picker",
                        &profile.name,
                        &profile.description,
                        index == picker.selected,
                        true,
                        width,
                    )
                })
                .collect::<Vec<_>>();
            let height = rows.len().min(view.selection_page_size);
            PanelRequest::new(rows, Some(picker.selected), height)
        }),
        Some(UiModalKind::Integration) => view.integration_prompt.as_ref().and_then(|prompt| {
            let mut rows = vec![text_row(
                "integration",
                &format!("Integrate changes from {}?", prompt.agent.profile),
                CellStyle::foreground(Color::Yellow).bold(),
                width,
            )];
            for (index, (label, description, enabled)) in [
                ("Apply", "Apply changes automatically", true),
                (
                    "Resolve",
                    "Resolve conflicts interactively",
                    prompt.integration.resolver_available,
                ),
                ("Keep worktree", "Leave changes isolated", true),
                ("Discard", "Discard isolated changes", true),
            ]
            .iter()
            .enumerate()
            {
                rows.push(panel_choice_row(
                    "integration",
                    label,
                    description,
                    index == prompt.selected,
                    *enabled,
                    width,
                ));
            }
            let height = rows.len();
            PanelRequest::new(rows, Some(prompt.selected.saturating_add(1)), height)
        }),
        Some(UiModalKind::PlanReview) => view.plan_review.as_ref().and_then(|review| {
            let labels = ["Execute", "Fresh execute", "Close"];
            let descriptions = [
                "Continue in this conversation",
                "Start a new session with the Plan and handoff",
                "Keep the Plan without executing",
            ];
            let mut rows = vec![text_row(
                "plan-review",
                &view.context.remaining_percent().map_or_else(
                    || "Current context remaining: unknown".to_owned(),
                    |remaining| {
                        format!(
                            "Current context remaining: {:.0}% ({})",
                            remaining,
                            view.context.usage_state.label()
                        )
                    },
                ),
                CellStyle::foreground(Color::Gray),
                width,
            )];
            rows.extend(labels.iter().enumerate().map(|(index, label)| {
                panel_choice_row(
                    "plan-review",
                    label,
                    descriptions[index],
                    index == review.selected,
                    true,
                    width,
                )
            }));
            let height = rows.len();
            PanelRequest::new(rows, Some(review.selected.saturating_add(1)), height)
        }),
        _ => None,
    }
}
