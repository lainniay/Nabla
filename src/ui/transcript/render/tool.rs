use crate::state::{ToolExecution, ToolStatus};
use crate::ui::{
    palette, shell,
    text::{wrap_styled_lines, wrap_text},
    types::{CellStyle, Color, StyledCell, VisualRow},
};

use super::{
    common::{
        cells_width, clip_cells, indent_styled_rows, row_from_cells, single_line_row,
        single_line_text, styled_cells,
    },
    diff::render_tool_diff,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolRenderMode {
    Compact,
    Expanded,
    Summary,
}
pub(crate) fn render_tool(
    id: &str,
    tool: &ToolExecution,
    width: u16,
    mode: ToolRenderMode,
    animation_frame: u8,
) -> Vec<VisualRow> {
    if tool.status == ToolStatus::Succeeded
        && let Some(diff) = tool.diff.as_ref()
    {
        return render_tool_diff(id, diff, width, mode);
    }
    let heading = tool_heading(&tool.name);
    let operation = tool_operation_summary(&tool.name, &tool.args);
    if mode == ToolRenderMode::Summary {
        let tail = match tool.status {
            ToolStatus::Failed | ToolStatus::Denied => {
                Some(tool_status_label(tool.status).to_owned())
            }
            _ => tool_compact_tail(tool).map(|(label, _)| label),
        };
        let marker = tool_marker(tool.status, animation_frame).0;
        return vec![single_line_row(
            id,
            &format!(
                "{marker} {heading} · {operation}{}",
                tail.map_or_else(String::new, |label| format!(" · {label}"))
            ),
            CellStyle::foreground(Color::White),
            width,
        )];
    }

    let (marker, marker_color) = tool_marker(tool.status, animation_frame);
    let mut title_cells = styled_cells(
        &format!("{marker} "),
        CellStyle::foreground(marker_color).bold(),
    );
    title_cells.extend(styled_cells(
        heading,
        CellStyle::foreground(Color::White).bold(),
    ));
    let mut rows = vec![row_from_cells(id, title_cells, width)];

    let prefix = styled_cells("  └ ", CellStyle::foreground(Color::Gray).dim());
    let detail = tool_detail_cells(tool, &operation);
    let tail = tool_compact_tail(tool);
    rows.push(tool_detail_row(id, prefix, detail, tail, width));
    if matches!(tool.status, ToolStatus::Failed | ToolStatus::Denied) {
        let mut failure = styled_cells("    ", CellStyle::foreground(Color::Gray).dim());
        failure.extend(styled_cells(
            tool_status_label(tool.status),
            CellStyle::foreground(Color::Red).bold(),
        ));
        if !tool.output.is_empty() {
            failure.extend(styled_cells(
                &format!(" · {}", tool_output_scale(&tool.output)),
                CellStyle::foreground(Color::Gray).dim(),
            ));
        }
        rows.push(row_from_cells(id, failure, width));
    }
    if mode == ToolRenderMode::Compact {
        return rows;
    }

    if let Some(command) = shell_command(tool) {
        rows.push(single_line_row(
            id,
            "  Command",
            CellStyle::foreground(Color::Cyan).bold(),
            width,
        ));
        rows.extend(indent_styled_rows(
            wrap_styled_lines(
                id,
                &shell::highlight(command),
                width.saturating_sub(2).max(1),
            ),
            "  ",
            CellStyle::foreground(Color::Gray).dim(),
        ));
        if shell_extra_arguments(&tool.args).is_some() {
            rows.push(single_line_row(
                id,
                "  Other arguments",
                CellStyle::foreground(Color::Cyan).bold(),
                width,
            ));
            let arguments = serde_json::to_string_pretty(
                &shell_extra_arguments(&tool.args).unwrap_or(serde_json::Value::Null),
            )
            .unwrap_or_else(|_| "null".to_owned());
            rows.extend(wrap_text(
                id,
                &arguments,
                width.max(1),
                CellStyle::foreground(Color::White),
            ));
        }
    } else {
        rows.push(single_line_row(
            id,
            "  Arguments",
            CellStyle::foreground(Color::Cyan).bold(),
            width,
        ));
        let arguments =
            serde_json::to_string_pretty(&tool.args).unwrap_or_else(|_| "null".to_owned());
        rows.extend(wrap_text(
            id,
            &arguments,
            width.max(1),
            CellStyle::foreground(Color::White),
        ));
    }
    rows.push(single_line_row(
        id,
        &format!("  Output · {}", tool_output_scale(&tool.output)),
        CellStyle::foreground(Color::Cyan).bold(),
        width,
    ));
    rows.extend(wrap_text(
        id,
        if tool.output.is_empty() {
            "(no output)"
        } else {
            &tool.output
        },
        width.max(1),
        CellStyle::foreground(Color::White),
    ));
    rows
}
fn tool_marker(status: ToolStatus, animation_frame: u8) -> (&'static str, Color) {
    const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    const BREATH: [Color; 4] = [
        palette::SURFACE_2,
        palette::OVERLAY_0,
        palette::YELLOW,
        palette::OVERLAY_0,
    ];
    match status {
        ToolStatus::Running => (
            SPINNER[usize::from(animation_frame) % SPINNER.len()],
            palette::SAPPHIRE,
        ),
        ToolStatus::WaitingApproval => ("●", BREATH[usize::from(animation_frame) % BREATH.len()]),
        ToolStatus::Succeeded => ("•", palette::MAUVE),
        ToolStatus::Failed | ToolStatus::Denied => ("•", palette::RED),
    }
}

fn tool_heading(name: &str) -> &'static str {
    let normalized = name.to_ascii_lowercase();
    if is_shell_name(&normalized) {
        "Ran"
    } else if ["write", "edit", "patch", "delete", "remove", "create"]
        .iter()
        .any(|operation| normalized.contains(operation))
    {
        "Edited"
    } else if ["read", "search", "grep", "find", "glob", "list"]
        .iter()
        .any(|operation| normalized.contains(operation))
        || normalized == "rg"
        || normalized == "ls"
    {
        "Explored"
    } else {
        "Called"
    }
}

fn is_shell_name(name: &str) -> bool {
    name.contains("bash") || name.contains("shell") || name.contains("exec") || name == "run"
}

fn shell_command(tool: &ToolExecution) -> Option<&str> {
    if !is_shell_name(&tool.name.to_ascii_lowercase()) {
        return None;
    }
    tool.args
        .as_object()?
        .get("command")
        .or_else(|| tool.args.as_object()?.get("cmd"))?
        .as_str()
}

fn shell_extra_arguments(args: &serde_json::Value) -> Option<serde_json::Value> {
    let object = args.as_object()?;
    let extra = object
        .iter()
        .filter(|(key, _)| !matches!(key.as_str(), "command" | "cmd"))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<serde_json::Map<_, _>>();
    (!extra.is_empty()).then_some(serde_json::Value::Object(extra))
}

fn tool_detail_cells(tool: &ToolExecution, operation: &str) -> Vec<StyledCell> {
    if let Some(command) = shell_command(tool) {
        return shell::highlight(&single_line_text(command))
            .into_iter()
            .next()
            .unwrap_or_default();
    }
    styled_cells(operation, CellStyle::foreground(Color::White))
}

fn tool_compact_tail(tool: &ToolExecution) -> Option<(String, CellStyle)> {
    match tool.status {
        ToolStatus::Succeeded if !tool.output.is_empty() => Some((
            tool_output_scale(&tool.output),
            CellStyle::foreground(Color::Gray).dim(),
        )),
        ToolStatus::Succeeded | ToolStatus::Failed | ToolStatus::Denied => None,
        ToolStatus::Running => Some(("running".to_owned(), CellStyle::foreground(Color::Cyan))),
        ToolStatus::WaitingApproval => Some((
            "waiting approval".to_owned(),
            CellStyle::foreground(Color::Yellow),
        )),
    }
}

fn tool_detail_row(
    id: &str,
    prefix: Vec<StyledCell>,
    detail: Vec<StyledCell>,
    tail: Option<(String, CellStyle)>,
    width: u16,
) -> VisualRow {
    let tail_cells = tail.map_or_else(Vec::new, |(label, style)| {
        styled_cells(&format!(" · {label}"), style)
    });
    let prefix_width = cells_width(&prefix);
    let tail_width = cells_width(&tail_cells);
    let detail_budget = width.saturating_sub(prefix_width.saturating_add(tail_width));
    let mut cells = prefix;
    cells.extend(clip_cells(detail, detail_budget));
    cells.extend(tail_cells);
    row_from_cells(id, cells, width)
}

pub(crate) fn tool_operation_summary(name: &str, args: &serde_json::Value) -> String {
    let normalized = name.to_ascii_lowercase();
    let command = argument_preview(args, &["command", "cmd"]);
    let path = argument_preview(args, &["path", "filePath", "file", "target"]);
    let query = argument_preview(args, &["query", "pattern", "search"]);
    let scope = argument_preview(args, &["cwd", "directory", "root", "scope"]);

    if normalized.contains("bash")
        || normalized.contains("shell")
        || normalized.contains("exec")
        || normalized == "run"
    {
        return operation_with_detail("Run", command);
    }
    if normalized.contains("read") {
        return operation_with_detail("Read", path);
    }
    if normalized.contains("write") || normalized.contains("create") {
        return operation_with_detail("Write", path);
    }
    if normalized.contains("edit") || normalized.contains("patch") {
        return operation_with_detail("Edit", path);
    }
    if normalized.contains("delete") || normalized.contains("remove") {
        return operation_with_detail("Delete", path);
    }
    if normalized.contains("search") || normalized.contains("grep") || normalized == "rg" {
        let detail = match (query, scope) {
            (Some(query), Some(scope)) => Some(format!("{query} in {scope}")),
            (query, scope) => query.or(scope),
        };
        return operation_with_detail("Search", detail);
    }
    if normalized.contains("find")
        || normalized.contains("glob")
        || normalized == "ls"
        || normalized.contains("list")
    {
        return operation_with_detail("Find", path.or(scope).or(query));
    }

    let label = name
        .replace(['_', '-'], " ")
        .split_whitespace()
        .map(|word| {
            let mut characters = word.chars();
            characters.next().map_or_else(String::new, |first| {
                first.to_uppercase().collect::<String>() + characters.as_str()
            })
        })
        .collect::<Vec<_>>()
        .join(" ");
    let detail = serde_json::to_string(args)
        .ok()
        .filter(|value| value != "{}" && value != "null");
    operation_with_detail(if label.is_empty() { "Tool" } else { &label }, detail)
}

fn argument_preview(args: &serde_json::Value, keys: &[&str]) -> Option<String> {
    let object = args.as_object()?;
    keys.iter().find_map(|key| {
        let value = object.get(*key)?;
        match value {
            serde_json::Value::String(value) if !value.is_empty() => Some(single_line_text(value)),
            serde_json::Value::Array(values) if !values.is_empty() => Some(
                values
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(single_line_text)
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            serde_json::Value::Null => None,
            value => serde_json::to_string(value).ok(),
        }
    })
}

fn operation_with_detail(label: &str, detail: Option<String>) -> String {
    detail
        .filter(|detail| !detail.is_empty())
        .map_or_else(|| label.to_owned(), |detail| format!("{label} · {detail}"))
}

fn tool_status_label(status: ToolStatus) -> &'static str {
    match status {
        ToolStatus::WaitingApproval => "waiting approval",
        ToolStatus::Running => "running",
        ToolStatus::Succeeded => "succeeded",
        ToolStatus::Failed => "failed",
        ToolStatus::Denied => "denied",
    }
}

fn tool_output_scale(output: &str) -> String {
    if output.is_empty() {
        return "0 B".to_owned();
    }
    let lines = output.lines().count();
    if lines > 1 || output.contains('\n') {
        return format!("{lines} lines");
    }
    let bytes = output.len();
    if bytes >= 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}
