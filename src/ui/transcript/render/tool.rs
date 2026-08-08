use crate::state::{ToolExecution, ToolStatus};
use crate::ui::{
    palette, shell,
    text::wrap_text,
    types::{CellStyle, Color, StyledCell, VisualRow},
};

use super::{
    common::{
        cells_width, clip_cells, row_from_cells, single_line_row, single_line_text, styled_cells,
        wrap_styled_breaking,
    },
    diff::render_tool_diff,
};

const PREVIEW_HEAD_LINES: usize = 2;
const PREVIEW_TAIL_LINES: usize = 4;
const COMMAND_CONTINUATION_PREFIX: &str = "  │ ";
const OUTPUT_FIRST_PREFIX: &str = "  └ ";
const OUTPUT_BODY_PREFIX: &str = "    ";
const OUTPUT_PREFIX_WIDTH: u16 = 4;

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
    if is_todo_tool(tool)
        && let Some(items) = todo_items(tool)
    {
        return render_todo_tool(id, tool, items, width, mode, animation_frame);
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
    if let Some(command) = shell_command(tool) {
        return render_shell_tool(id, tool, command, width, mode, animation_frame);
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
    let detail = tool_detail_cells(&operation);
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

    rows.push(single_line_row(
        id,
        "  Arguments",
        CellStyle::foreground(Color::Cyan).bold(),
        width,
    ));
    let arguments = serde_json::to_string_pretty(&tool.args).unwrap_or_else(|_| "null".to_owned());
    rows.extend(wrap_text(
        id,
        &arguments,
        width.max(1),
        CellStyle::foreground(Color::White),
    ));
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

fn is_todo_tool(tool: &ToolExecution) -> bool {
    tool.name.eq_ignore_ascii_case("todo_write")
}

struct TodoRenderItem {
    content: String,
    status: String,
}

fn todo_items(tool: &ToolExecution) -> Option<Vec<TodoRenderItem>> {
    let output = serde_json::from_str::<serde_json::Value>(&tool.output).ok();
    output
        .as_ref()
        .and_then(|value| value.get("todos"))
        .and_then(parse_todo_items)
        .or_else(|| tool.args.get("todos").and_then(parse_todo_items))
}

fn todo_action(tool: &ToolExecution) -> &'static str {
    let output = serde_json::from_str::<serde_json::Value>(&tool.output).ok();
    match output
        .as_ref()
        .and_then(|value| value.get("action"))
        .and_then(|value| value.as_str())
    {
        Some("created") => "created",
        _ => "updated",
    }
}

fn parse_todo_items(value: &serde_json::Value) -> Option<Vec<TodoRenderItem>> {
    let array = value.as_array()?;
    let mut items = Vec::with_capacity(array.len());
    for raw in array {
        let object = raw.as_object()?;
        let content = object.get("content")?.as_str()?;
        let status = match object.get("status")?.as_str()? {
            "pending" => "pending",
            "in_progress" => "in_progress",
            "completed" => "completed",
            _ => return None,
        };
        items.push(TodoRenderItem {
            content: content.to_owned(),
            status: status.to_owned(),
        });
    }
    Some(items)
}

fn render_todo_tool(
    id: &str,
    tool: &ToolExecution,
    items: Vec<TodoRenderItem>,
    width: u16,
    mode: ToolRenderMode,
    animation_frame: u8,
) -> Vec<VisualRow> {
    let heading = if todo_action(tool) == "created" {
        "Create TODO"
    } else {
        "Edit TODO"
    };
    let (marker, marker_color) = tool_marker(tool.status, animation_frame);
    if mode == ToolRenderMode::Summary {
        let count = if items.is_empty() {
            "empty".to_owned()
        } else {
            let done = items
                .iter()
                .filter(|item| item.status == "completed")
                .count();
            format!("{done}/{}", items.len())
        };
        return vec![single_line_row(
            id,
            &format!("{marker} {heading} · {count}"),
            CellStyle::foreground(Color::White),
            width,
        )];
    }

    let mut title_cells = styled_cells(
        &format!("{marker} "),
        CellStyle::foreground(marker_color).bold(),
    );
    title_cells.extend(styled_cells(
        heading,
        CellStyle::foreground(Color::White).bold(),
    ));
    let mut rows = vec![row_from_cells(id, title_cells, width)];

    let item_width = width.saturating_sub(4).max(1);
    let logical_lines = items
        .iter()
        .map(|item| {
            let (glyph, marker_style, content_style) = match item.status.as_str() {
                "pending" => (
                    "○",
                    CellStyle::foreground(Color::Gray).dim(),
                    CellStyle::foreground(Color::White),
                ),
                "in_progress" => (
                    "◐",
                    CellStyle::foreground(palette::SAPPHIRE),
                    CellStyle::foreground(Color::White),
                ),
                _ => (
                    "●",
                    CellStyle::foreground(palette::MAUVE),
                    CellStyle::foreground(Color::Gray).dim().crossed_out(),
                ),
            };
            let mut cells = vec![
                StyledCell::new(glyph, 1, marker_style),
                StyledCell::new(" ", 1, CellStyle::default()),
            ];
            cells.extend(styled_cells(&item.content, content_style));
            cells
        })
        .collect::<Vec<_>>();
    let wrapped = wrap_styled_breaking(id, &logical_lines, item_width);
    let mut previous_line = usize::MAX;
    for row in wrapped {
        let prefix = if row.logical_line == previous_line {
            "    "
        } else {
            previous_line = row.logical_line;
            "  "
        };
        let mut cells = styled_cells(prefix, CellStyle::default());
        cells.extend(row.cells);
        rows.push(row_from_cells(id, cells, width));
    }

    if matches!(tool.status, ToolStatus::Failed | ToolStatus::Denied) {
        let mut failure = styled_cells("    ", CellStyle::foreground(Color::Gray).dim());
        failure.extend(styled_cells(
            tool_status_label(tool.status),
            CellStyle::foreground(Color::Red).bold(),
        ));
        rows.push(row_from_cells(id, failure, width));
    }
    rows
}

fn render_shell_tool(
    id: &str,
    tool: &ToolExecution,
    command: &str,
    width: u16,
    mode: ToolRenderMode,
    animation_frame: u8,
) -> Vec<VisualRow> {
    let (marker, marker_color) = tool_marker(tool.status, animation_frame);
    let mut rows = Vec::new();

    // The full command wraps under a "• Ran " title; continuation lines keep
    // the tree alive with a │ gutter.
    let command_width = width.saturating_sub(6).max(1);
    let command_rows = wrap_styled_breaking(id, &shell::highlight(command), command_width);
    for (index, row) in command_rows.into_iter().enumerate() {
        let mut cells = if index == 0 {
            vec![
                StyledCell::new(marker, 1, CellStyle::foreground(marker_color).bold()),
                StyledCell::new(" ", 1, CellStyle::foreground(marker_color)),
                StyledCell::new("Ran ", 4, CellStyle::foreground(Color::White).bold()),
            ]
        } else {
            styled_cells(
                COMMAND_CONTINUATION_PREFIX,
                CellStyle::foreground(Color::Gray).dim(),
            )
        };
        cells.extend(row.cells);
        rows.push(row_from_cells(id, cells, width));
    }

    if mode == ToolRenderMode::Expanded
        && let Some(extra) = shell_extra_arguments(&tool.args)
    {
        rows.push(single_line_row(
            id,
            "  Other arguments",
            CellStyle::foreground(Color::Cyan).bold(),
            width,
        ));
        let arguments = serde_json::to_string_pretty(&extra).unwrap_or_else(|_| "null".to_owned());
        rows.extend(wrap_text(
            id,
            &arguments,
            width.max(1),
            CellStyle::foreground(Color::White),
        ));
    }

    let output_lines = tool.output.lines().collect::<Vec<_>>();
    if !output_lines.is_empty() {
        rows.extend(render_output_block(id, &output_lines, width, mode));
    } else if matches!(
        tool.status,
        ToolStatus::Running | ToolStatus::WaitingApproval
    ) {
        rows.push(status_leaf_row(id, tool.status, width));
    }

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
    rows
}

fn render_output_block(
    id: &str,
    output_lines: &[&str],
    width: u16,
    mode: ToolRenderMode,
) -> Vec<VisualRow> {
    let total = output_lines.len();
    let (head, tail, omitted) =
        if mode == ToolRenderMode::Compact && total > PREVIEW_HEAD_LINES + PREVIEW_TAIL_LINES {
            (
                PREVIEW_HEAD_LINES,
                PREVIEW_TAIL_LINES,
                total - PREVIEW_HEAD_LINES - PREVIEW_TAIL_LINES,
            )
        } else {
            (total, 0, 0)
        };
    let output_style = CellStyle::foreground(Color::Gray).dim();
    let mut logical_lines = Vec::<Vec<StyledCell>>::new();
    for line in output_lines.iter().take(head) {
        logical_lines.push(styled_cells(line, output_style));
    }
    if omitted > 0 {
        logical_lines.push(styled_cells(
            &format!("… +{omitted} lines · expand in Ctrl+O"),
            CellStyle::foreground(palette::GRAY_MUTED).dim(),
        ));
        for line in output_lines.iter().skip(total - tail) {
            logical_lines.push(styled_cells(line, output_style));
        }
    }
    let inner_width = width.saturating_sub(OUTPUT_PREFIX_WIDTH).max(1);
    let wrapped = wrap_styled_breaking(id, &logical_lines, inner_width);
    let mut rows = Vec::with_capacity(wrapped.len());
    for (index, row) in wrapped.into_iter().enumerate() {
        let prefix = if index == 0 {
            OUTPUT_FIRST_PREFIX
        } else {
            OUTPUT_BODY_PREFIX
        };
        let mut cells = styled_cells(prefix, CellStyle::foreground(Color::Gray).dim());
        cells.extend(row.cells);
        rows.push(row_from_cells(id, cells, width));
    }
    rows
}

fn status_leaf_row(id: &str, status: ToolStatus, width: u16) -> VisualRow {
    let (label, color) = match status {
        ToolStatus::Running => ("running", Color::Cyan),
        ToolStatus::WaitingApproval => ("waiting approval", Color::Yellow),
        _ => unreachable!("only transient statuses render a leaf row"),
    };
    let mut cells = styled_cells(
        OUTPUT_FIRST_PREFIX,
        CellStyle::foreground(Color::Gray).dim(),
    );
    cells.extend(styled_cells(label, CellStyle::foreground(color)));
    row_from_cells(id, cells, width)
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

fn tool_detail_cells(operation: &str) -> Vec<StyledCell> {
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
