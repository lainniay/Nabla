use std::path::{Component, Path};

use crate::{
    host::ApprovalDecision,
    state::{ApprovalState, GrantProposal},
    ui::{
        palette,
        panel::PanelRequest,
        scene::{text_row, view_model::SceneViewModel},
        shell,
        transcript::{row_from_cells, tool_operation_summary},
        types::{CellStyle, Color, StyledCell, VisualRow},
    },
};

use super::panel_choice_row;

pub(crate) fn approval_modal(view: &SceneViewModel, width: u16) -> Option<PanelRequest> {
    approval_panel_request(view.approval.as_ref()?, width)
}

pub(crate) fn permissions_modal(view: &SceneViewModel, width: u16) -> Option<PanelRequest> {
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
fn approval_panel_request(approval: &ApprovalState, width: u16) -> Option<PanelRequest> {
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

fn approval_summary(approval: &ApprovalState) -> &str {
    match approval.risk.as_deref().unwrap_or("normal") {
        "outside_workspace" => "Outside trusted project scope",
        "credential" => "May access sensitive credentials",
        "high" => "High-risk command",
        "elevated" => "Elevated operation",
        _ if approval.summary.is_empty() => "This action requires approval",
        _ => &approval.summary,
    }
}

fn approval_operation_row(approval: &ApprovalState, width: u16) -> VisualRow {
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
