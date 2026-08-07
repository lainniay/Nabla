use std::{
    collections::HashMap,
    path::{Component, Path},
};

use crate::ui::{
    palette,
    panel::PanelRequest,
    scene::{append_text_cells, cells_width, text_row, view_model::SceneViewModel},
    selector::VirtualList,
    shell,
    store::UiState,
    text::{display_width, truncate, wrap_text},
    transcript::{render_viewer_item, row_from_cells, tool_operation_summary},
    types::{CellStyle, Color, RowRange, StyledCell, VisualRow},
};
use crate::{
    command::COMMAND_MENU_VISIBLE_ROWS,
    host::ApprovalDecision,
    state::{
        AuthPromptKind, AuthState, GrantProposal, TranscriptItem, TranscriptViewMode, TreeItem,
        TreePhase, UiModalKind, matching_auth_choice_indices,
    },
};

pub(crate) fn primary_panel_request(view: &SceneViewModel, width: u16) -> Option<PanelRequest> {
    let width = width.saturating_sub(2).max(1);
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
        Some(UiModalKind::Approval) => approval_panel_request(view.approval.as_ref()?, width),
        Some(UiModalKind::Permissions) => {
            let manager = view.permission_manager.as_ref()?;
            let mut rows = vec![
                text_row(
                    "permissions",
                    "Persistent Approvals",
                    CellStyle::foreground(palette::LAVENDER).bold(),
                    width,
                ),
                text_row(
                    "permissions",
                    "Current project · [D] revoke · [C] clear · Esc close",
                    CellStyle::foreground(Color::Gray).dim(),
                    width,
                ),
            ];
            if manager.snapshot.grants.is_empty() {
                rows.push(text_row(
                    "permissions",
                    "No persistent approvals",
                    CellStyle::foreground(palette::SUBTEXT_0),
                    width,
                ));
            } else {
                rows.extend(
                    manager
                        .snapshot
                        .grants
                        .iter()
                        .enumerate()
                        .map(|(index, grant)| {
                            panel_choice_row(
                                "permissions",
                                &grant.proposal.scope,
                                &grant_proposal_summary(&grant.proposal),
                                index == manager.selected,
                                true,
                                width,
                            )
                        }),
                );
            }
            let selected =
                (!manager.snapshot.grants.is_empty()).then_some(manager.selected.saturating_add(2));
            let height = rows.len().min(view.selection_page_size.saturating_add(2));
            PanelRequest::new(rows, selected, height)
        }
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

fn approval_panel_request(
    approval: &crate::state::ApprovalState,
    width: u16,
) -> Option<PanelRequest> {
    let risk = approval.risk.as_deref().unwrap_or("normal");
    let mut rows = vec![text_row(
        "approval",
        "Ask for Approval",
        CellStyle::foreground(palette::LAVENDER).bold(),
        width,
    )];
    rows.push(VisualRow::blank("approval-spacing"));
    rows.push(text_row(
        "approval-summary",
        &format!("• {}", approval_summary(approval)),
        match risk {
            "high" | "credential" | "outside_workspace" => CellStyle::foreground(Color::Red),
            "elevated" => CellStyle::foreground(Color::Yellow),
            _ => CellStyle::foreground(palette::SUBTEXT_0),
        },
        width,
    ));
    rows.push(approval_operation_row(approval, width));
    rows.push(VisualRow::blank("approval-spacing"));

    let actions = approval
        .available_decisions
        .iter()
        .map(|decision| match decision {
            ApprovalDecision::AllowOnce => (
                "Allow once".to_owned(),
                "Approve only this request".to_owned(),
            ),
            ApprovalDecision::AllowSession => (
                "Allow for Session".to_owned(),
                "Remember for this session".to_owned(),
            ),
            ApprovalDecision::AllowWorkspace => (
                "Allow for Workspace".to_owned(),
                "Remember for this workspace".to_owned(),
            ),
            ApprovalDecision::Deny => ("Deny".to_owned(), "Reject this tool request".to_owned()),
        })
        .collect::<Vec<_>>();
    let action_offset = rows.len();
    for (index, (label, description)) in actions.iter().enumerate() {
        rows.push(panel_choice_row(
            "approval",
            label,
            description,
            index == approval.selected,
            true,
            width,
        ));
    }
    let height = rows.len();
    PanelRequest::new(
        rows,
        Some(approval.selected.saturating_add(action_offset)),
        height,
    )
}

fn grant_proposal_summary(proposal: &GrantProposal) -> String {
    let mut parts = proposal
        .matchers
        .iter()
        .map(grant_matcher_summary)
        .collect::<Vec<_>>();
    parts.extend(proposal.invalidation_keys.iter().map(|key| {
        let kind = key
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("invalidation");
        let path = key
            .get("path")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if path.is_empty() {
            format!("{kind}={}", key["value"].as_str().unwrap_or("?"))
        } else {
            format!("{kind} {path}={}", key["value"].as_str().unwrap_or("?"))
        }
    }));
    parts.join("; ")
}

fn grant_matcher_summary(matcher: &serde_json::Value) -> String {
    match matcher.get("kind").and_then(serde_json::Value::as_str) {
        Some("exec") => {
            let executable = matcher["executable"].as_str().unwrap_or("?");
            let argv = matcher["argv"]
                .as_array()
                .map(|values| {
                    values
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            let cwd = matcher["cwd"].as_str().unwrap_or("?");
            format!("exec {executable} {argv} @ {cwd}")
        }
        Some("file") => format!(
            "{} {}",
            matcher["operation"].as_str().unwrap_or("file"),
            matcher["path"].as_str().unwrap_or("?")
        ),
        Some("opaque_code") => format!(
            "exact opaque {}:{}",
            matcher["runtime"].as_str().unwrap_or("?"),
            matcher["digest"].as_str().unwrap_or("?")
        ),
        Some(kind) => format!("{kind} {}", matcher),
        None => matcher.to_string(),
    }
}

fn approval_summary(approval: &crate::state::ApprovalState) -> &str {
    match approval.risk.as_deref().unwrap_or("normal") {
        "outside_workspace" => "Outside trusted project scope",
        "credential" => "May access sensitive credentials",
        "high" => "High-risk command",
        "elevated" => "Elevated operation",
        _ if approval.summary.is_empty() => "This action requires approval",
        _ => &approval.summary,
    }
}

fn approval_operation_row(approval: &crate::state::ApprovalState, width: u16) -> VisualRow {
    let normalized_name = approval.tool_name.to_ascii_lowercase();
    let operation = if let Some(command) = input_string(&approval.input, &["command", "cmd"]) {
        command.replace(['\r', '\n'], " ")
    } else if is_file_tool(&normalized_name) {
        let label = tool_operation_summary(&approval.tool_name, &approval.input)
            .split(" · ")
            .next()
            .unwrap_or("File")
            .to_owned();
        input_paths(&approval.input).first().map_or_else(
            || label.clone(),
            |path| format!("{label} {}", normalize_display_path(path)),
        )
    } else {
        tool_operation_summary(&approval.tool_name, &approval.input)
    };
    let mut cells = shell::highlight(&operation)
        .into_iter()
        .next()
        .unwrap_or_default();
    let mut prefixed = vec![StyledCell::new(
        "  └ ",
        4,
        CellStyle::foreground(Color::Cyan).bold(),
    )];
    prefixed.append(&mut cells);
    row_from_cells("approval-input", prefixed, width)
}

fn input_string<'a>(input: &'a serde_json::Value, keys: &[&str]) -> Option<&'a str> {
    let object = input.as_object()?;
    keys.iter().find_map(|key| object.get(*key)?.as_str())
}

fn input_paths(input: &serde_json::Value) -> Vec<String> {
    let Some(object) = input.as_object() else {
        return Vec::new();
    };
    for key in ["path", "filePath", "file", "target"] {
        if let Some(path) = object.get(key).and_then(serde_json::Value::as_str) {
            return vec![path.to_owned()];
        }
    }
    object
        .get("paths")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .collect()
}

fn is_file_tool(name: &str) -> bool {
    ["read", "write", "edit", "patch", "file", "delete", "remove"]
        .iter()
        .any(|operation| name.contains(operation))
}

fn normalize_display_path(value: &str) -> String {
    let path = Path::new(value);
    let absolute = path.is_absolute();
    let mut components = Vec::<String>::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => {
                components.push(prefix.as_os_str().to_string_lossy().into_owned());
            }
            Component::RootDir | Component::CurDir => {}
            Component::ParentDir => {
                if components.last().is_some_and(|part| part != "..") {
                    components.pop();
                } else if !absolute {
                    components.push("..".to_owned());
                }
            }
            Component::Normal(part) => components.push(part.to_string_lossy().into_owned()),
        }
    }
    let body = components.join("/");
    if absolute {
        format!("/{body}")
    } else if body.is_empty() {
        ".".to_owned()
    } else {
        body
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

fn aligned_panel_row(
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

fn choice_row(id: &str, label: &str, description: &str, selected: bool, width: u16) -> VisualRow {
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
    let mut rows = Vec::new();
    let title_style = CellStyle::foreground(Color::Magenta).bold();
    match view.active_modal_kind() {
        Some(UiModalKind::SessionBrowser) => {
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
        Some(UiModalKind::TreeBrowser) => {
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
        Some(UiModalKind::Transcript) => {
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
        Some(UiModalKind::Auth) => {
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
        _ => {
            rows.push(text_row("alternate", "Nabla", title_style, width));
            rows.push(text_row(
                "alternate",
                "No alternate-screen route is active.",
                CellStyle::foreground(Color::Gray),
                width,
            ));
        }
    }
    rows
}

fn append_choice_window(
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
