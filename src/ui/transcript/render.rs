use crate::state::{
    PlanArtifact, ToolDiff, ToolDiffFile, ToolDiffLine, ToolDiffLineKind, ToolExecution,
    ToolStatus, TranscriptItem, TranscriptViewMode, TurnSeparator, UserMessage, UserMessageStatus,
};
use crate::ui::{
    markdown, palette, shell,
    text::{display_width, truncate, wrap_file_references, wrap_styled_lines, wrap_text},
    types::{AssistantContentKind, AssistantSegment, CellStyle, Color, StyledCell, VisualRow},
};

pub(crate) fn render_assistant_segment(
    id: &str,
    item: &TranscriptItem,
    segment: &AssistantSegment,
    width: u16,
) -> Vec<VisualRow> {
    let TranscriptItem::Assistant(message) = item else {
        return render_item(id, item, width, 0);
    };
    let marker_style = CellStyle::foreground(Color::Magenta);
    let content_width = width.saturating_sub(2).max(1);
    let (source, style) = match segment.content_kind {
        AssistantContentKind::Thinking => (
            if segment.segment_index == 0 {
                format!("*Thinking*\n\n{}", message.thinking)
            } else {
                message.thinking.clone()
            },
            CellStyle::foreground(palette::THINKING_TEXT).italic(),
        ),
        AssistantContentKind::Text => (message.text.clone(), CellStyle::foreground(palette::TEXT)),
    };
    let mut rows = markdown::render(&source, id, content_width, style);
    prefix_assistant_rows(&mut rows, marker_style, style, segment.first_in_message);
    rows
}

pub(crate) fn render_item(
    id: &str,
    item: &TranscriptItem,
    width: u16,
    animation_frame: u8,
) -> Vec<VisualRow> {
    if let TranscriptItem::Assistant(message) = item {
        let marker_style = CellStyle::foreground(Color::Magenta);
        let content_width = width.saturating_sub(2).max(1);
        let mut rows = Vec::new();
        if !message.thinking.is_empty() {
            let thinking_style = CellStyle::foreground(palette::THINKING_TEXT).italic();
            let mut thinking = markdown::render(
                &format!("*Thinking*\n\n{}", message.thinking),
                id,
                content_width,
                thinking_style,
            );
            prefix_assistant_rows(&mut thinking, marker_style, thinking_style, true);
            rows.extend(thinking);
        }
        if !message.text.is_empty() {
            if !rows.is_empty() {
                rows.push(VisualRow::blank(id));
            }
            let body_style = CellStyle::foreground(palette::TEXT);
            let mut body = markdown::render(&message.text, id, content_width, body_style);
            prefix_assistant_rows(&mut body, marker_style, body_style, rows.is_empty());
            rows.extend(body);
        }
        return rows;
    }
    if let TranscriptItem::User(message) = item {
        return render_user(id, message, width);
    }
    if let TranscriptItem::Tool(tool) = item {
        return render_tool(id, tool, width, ToolRenderMode::Compact, animation_frame);
    }
    if let TranscriptItem::TurnSeparator(separator) = item {
        return render_turn_separator(id, separator, width);
    }
    if let TranscriptItem::Plan(plan) = item {
        return render_plan(id, plan, width, false);
    }

    let (prefix, body, style) = match item {
        TranscriptItem::User(_) => unreachable!("user messages are rendered above"),
        TranscriptItem::Assistant(_) | TranscriptItem::Plan(_) => {
            unreachable!("Markdown transcript items are rendered above")
        }
        TranscriptItem::Tool(_) => unreachable!("tools are rendered above"),
        TranscriptItem::Context(snapshot) => (
            "Context",
            format!(
                "{} tokens / {:?} window ({:.1}%)",
                snapshot.actual_tokens.unwrap_or_default(),
                snapshot.context_window,
                snapshot.actual_percent.unwrap_or_default()
            ),
            CellStyle::foreground(Color::Cyan),
        ),
        TranscriptItem::Resources(snapshot) => (
            "Resources",
            format!(
                "{} skills · {} prompts · {} extensions · trusted={}",
                snapshot.skills.len(),
                snapshot.prompts.len(),
                snapshot.extensions.len(),
                snapshot.trusted
            ),
            CellStyle::foreground(Color::Cyan),
        ),
        TranscriptItem::Agents(snapshot) => (
            "Agents",
            format!(
                "{} active · {} pending · {} profiles",
                snapshot.active.len(),
                snapshot.pending.len(),
                snapshot.profiles.len()
            ),
            CellStyle::foreground(Color::Cyan),
        ),
        TranscriptItem::Subagent(event) => (
            "Agent",
            format!(
                "{} · {} · {}",
                event.agent.profile, event.event, event.agent.task
            ),
            CellStyle::foreground(Color::Cyan),
        ),
        TranscriptItem::Compaction(record) => (
            "Compaction",
            format!("{record:?}"),
            CellStyle::foreground(Color::Yellow),
        ),
        TranscriptItem::TurnSeparator(_) => {
            unreachable!("turn separators are rendered above")
        }
        TranscriptItem::BranchSummary(summary) => (
            "Branch",
            summary.clone(),
            CellStyle::foreground(Color::Cyan),
        ),
        TranscriptItem::SessionBoundary { action, label, cwd } => (
            "Session",
            format!("{action}: {label}\n{cwd}"),
            CellStyle::foreground(Color::Yellow),
        ),
        TranscriptItem::Notice(message) => (
            "Notice",
            message.clone(),
            CellStyle::foreground(Color::Yellow),
        ),
        TranscriptItem::Error(message) => (
            "Error",
            message.clone(),
            CellStyle::foreground(Color::Red).bold(),
        ),
    };

    let marker = match item {
        TranscriptItem::User(_) => unreachable!("user messages are rendered above"),
        TranscriptItem::Assistant(_) => "•",
        TranscriptItem::Tool(_) => unreachable!("tools are rendered above"),
        TranscriptItem::TurnSeparator(_) => unreachable!("turn separators are rendered above"),
        TranscriptItem::Error(_) => "×",
        TranscriptItem::Notice(_) | TranscriptItem::Compaction(_) => "!",
        TranscriptItem::Plan(_) => "◇",
        _ => "·",
    };
    let body = match item {
        TranscriptItem::User(_) | TranscriptItem::Assistant(_) | TranscriptItem::Tool(_) => body,
        _ => format!("{prefix} · {body}"),
    };
    let body_style = match item {
        TranscriptItem::Error(_) => style,
        TranscriptItem::Notice(_) | TranscriptItem::Compaction(_) => style,
        _ => CellStyle::foreground(Color::White),
    };
    let content_width = width.saturating_sub(2).max(1);
    let mut rows = wrap_text(id, &body, content_width, body_style);
    for (index, row) in rows.iter_mut().enumerate() {
        let mut prefixed = if index == 0 {
            vec![
                StyledCell::new(marker, 1, style.bold()),
                StyledCell::new(" ", 1, style),
            ]
        } else {
            vec![StyledCell::new("  ", 2, body_style)]
        };
        prefixed.extend(std::mem::take(&mut row.cells));
        row.cells = prefixed;
    }
    rows
}

fn prefix_assistant_rows(
    rows: &mut [VisualRow],
    marker_style: CellStyle,
    body_style: CellStyle,
    show_marker: bool,
) {
    for (index, row) in rows.iter_mut().enumerate() {
        let mut prefixed = if index == 0 && show_marker {
            vec![
                StyledCell::new("•", 1, marker_style.bold()),
                StyledCell::new(" ", 1, marker_style),
            ]
        } else {
            vec![StyledCell::new("  ", 2, body_style)]
        };
        prefixed.extend(std::mem::take(&mut row.cells));
        row.cells = prefixed;
    }
}

fn render_plan(id: &str, plan: &PlanArtifact, width: u16, expanded: bool) -> Vec<VisualRow> {
    let mut body = format!(
        "**Plan · {}** (r{})\n\n{}\n\n{}",
        plan.title, plan.revision, plan.summary, plan.body_markdown
    );
    if !plan.assumptions.is_empty() {
        body.push_str("\n\n## Assumptions\n");
        body.push_str(
            &plan
                .assumptions
                .iter()
                .map(|item| format!("- {item}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    if !plan.test_plan.is_empty() {
        body.push_str("\n\n## Test plan\n");
        body.push_str(
            &plan
                .test_plan
                .iter()
                .map(|item| format!("- {item}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    if expanded && !plan.handoff_markdown.is_empty() {
        body.push_str("\n\n## Handoff\n");
        body.push_str(&plan.handoff_markdown);
    }

    let marker_style = CellStyle::foreground(Color::Cyan);
    let body_style = CellStyle::foreground(Color::White);
    let content_width = width.saturating_sub(2).max(1);
    let mut rows = markdown::render(&body, id, content_width, body_style);
    for (index, row) in rows.iter_mut().enumerate() {
        let mut prefixed = if index == 0 {
            vec![
                StyledCell::new("◇", 1, marker_style.bold()),
                StyledCell::new(" ", 1, marker_style),
            ]
        } else {
            vec![StyledCell::new("  ", 2, body_style)]
        };
        prefixed.extend(std::mem::take(&mut row.cells));
        row.cells = prefixed;
    }
    rows
}

pub(crate) fn render_user(id: &str, message: &UserMessage, width: u16) -> Vec<VisualRow> {
    let border_style = match message.status {
        UserMessageStatus::Pending => CellStyle::foreground(Color::Yellow).bold(),
        UserMessageStatus::Accepted => CellStyle::foreground(palette::HISTORY_BORDER).dim(),
        UserMessageStatus::Failed => CellStyle::foreground(Color::Red).bold(),
    };
    let body_style = CellStyle::foreground(Color::White);
    if width < 6 {
        let content_width = width.saturating_sub(2).max(1);
        let mut rows = wrap_file_references(id, &message.text, content_width, body_style);
        for (index, row) in rows.iter_mut().enumerate() {
            let mut cells = if index == 0 {
                vec![
                    StyledCell::new("›", 1, border_style),
                    StyledCell::new(" ", 1, body_style),
                ]
            } else {
                vec![StyledCell::new("  ", 2, body_style)]
            };
            cells.extend(std::mem::take(&mut row.cells));
            row.cells = cells;
        }
        return rows;
    }

    let inner_width = width.saturating_sub(4).max(1);
    let mut rows = vec![user_border_row(
        id,
        width,
        true,
        match message.status {
            UserMessageStatus::Pending => Some("pending"),
            UserMessageStatus::Failed => Some("failed"),
            UserMessageStatus::Accepted => None,
        },
        border_style,
    )];
    for mut row in wrap_file_references(id, &message.text, inner_width, body_style) {
        let content_width = row.display_width();
        let padding = inner_width.saturating_sub(content_width);
        let mut cells = vec![
            StyledCell::new("│", 1, border_style),
            StyledCell::new(" ", 1, body_style),
        ];
        cells.extend(std::mem::take(&mut row.cells));
        if padding > 0 {
            cells.push(StyledCell::new(
                " ".repeat(usize::from(padding)),
                padding,
                body_style,
            ));
        }
        cells.push(StyledCell::new(" ", 1, body_style));
        cells.push(StyledCell::new("│", 1, border_style));
        row.cells = cells;
        rows.push(row);
    }
    rows.push(user_border_row(id, width, false, None, border_style));
    rows
}

fn user_border_row(
    id: &str,
    width: u16,
    top: bool,
    label: Option<&str>,
    style: CellStyle,
) -> VisualRow {
    let (left, right) = if top { ("╭", "╮") } else { ("╰", "╯") };
    let available = usize::from(width.saturating_sub(2));
    let middle = label
        .filter(|label| available >= display_width(label).saturating_add(3))
        .map_or_else(
            || "─".repeat(available),
            |label| {
                let prefix = format!("─ {label} ");
                format!(
                    "{prefix}{}",
                    "─".repeat(available.saturating_sub(display_width(&prefix)))
                )
            },
        );
    VisualRow {
        component_id: id.to_owned(),
        logical_line: 0,
        wrap_index: 0,
        cells: vec![
            StyledCell::new(left, 1, style),
            StyledCell::new(middle, width.saturating_sub(2), style),
            StyledCell::new(right, 1, style),
        ],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolRenderMode {
    Compact,
    Expanded,
    Summary,
}

const COMPACT_DIFF_LINES_PER_FILE: usize = 40;

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

fn render_tool_diff(id: &str, diff: &ToolDiff, width: u16, mode: ToolRenderMode) -> Vec<VisualRow> {
    let mut heading = styled_cells("• ", CellStyle::foreground(palette::MAUVE).bold());
    heading.extend(styled_cells(
        &format!(
            "Edited {} {} ",
            diff.files.len(),
            if diff.files.len() == 1 {
                "file"
            } else {
                "files"
            }
        ),
        CellStyle::foreground(Color::White).bold(),
    ));
    append_diff_stats(&mut heading, diff.additions, diff.deletions);
    let mut rows = vec![row_from_cells(id, heading, width)];
    if mode == ToolRenderMode::Summary {
        return rows;
    }

    for file in &diff.files {
        rows.push(render_diff_file_heading(id, file, width));
        let visible = if mode == ToolRenderMode::Expanded {
            file.lines.len()
        } else {
            file.lines.len().min(COMPACT_DIFF_LINES_PER_FILE)
        };
        let line_number_width = file
            .lines
            .iter()
            .filter_map(|line| line.line_number)
            .map(|line| line.to_string().len())
            .max()
            .unwrap_or(1);
        rows.extend(
            file.lines
                .iter()
                .take(visible)
                .map(|line| render_diff_line(id, line, line_number_width, width)),
        );
        let omitted = file.lines.len().saturating_sub(visible);
        if omitted > 0 {
            rows.push(single_line_row(
                id,
                &format!("    … {omitted} more diff lines · expand in Ctrl+O"),
                CellStyle::foreground(palette::GRAY_MUTED).dim(),
                width,
            ));
        }
    }
    rows
}

fn render_diff_file_heading(id: &str, file: &ToolDiffFile, width: u16) -> VisualRow {
    let mut cells = styled_cells("  └ ", CellStyle::foreground(palette::GRAY_MUTED).dim());
    cells.extend(styled_cells(
        &sanitize_diff_fragment(&file.path),
        CellStyle::foreground(Color::White),
    ));
    cells.push(StyledCell::new(
        " ",
        1,
        CellStyle::foreground(palette::GRAY_MUTED),
    ));
    append_diff_stats(&mut cells, file.additions, file.deletions);
    row_from_cells(id, cells, width)
}

fn append_diff_stats(cells: &mut Vec<StyledCell>, additions: usize, deletions: usize) {
    cells.extend(styled_cells(
        "(",
        CellStyle::foreground(palette::GRAY_MUTED).dim(),
    ));
    cells.extend(styled_cells(
        &format!("+{additions}"),
        CellStyle::foreground(palette::GREEN),
    ));
    cells.extend(styled_cells(
        " ",
        CellStyle::foreground(palette::GRAY_MUTED).dim(),
    ));
    cells.extend(styled_cells(
        &format!("-{deletions}"),
        CellStyle::foreground(palette::RED),
    ));
    cells.extend(styled_cells(
        ")",
        CellStyle::foreground(palette::GRAY_MUTED).dim(),
    ));
}

fn render_diff_line(
    id: &str,
    line: &ToolDiffLine,
    line_number_width: usize,
    width: u16,
) -> VisualRow {
    if line.kind == ToolDiffLineKind::Omission {
        return single_line_row(
            id,
            &format!(
                "    {:line_number_width$}  {}",
                "",
                sanitize_diff_fragment(&line.text)
            ),
            CellStyle::foreground(palette::GRAY_MUTED).dim(),
            width,
        );
    }

    let number = line
        .line_number
        .map_or_else(String::new, |number| number.to_string());
    let mut cells = styled_cells(
        &format!("    {number:>line_number_width$} "),
        CellStyle::foreground(palette::GRAY_MUTED).dim(),
    );
    let (marker, style, background) = match line.kind {
        ToolDiffLineKind::Addition => (
            "+",
            CellStyle::foreground(palette::GREEN),
            Some(palette::DIFF_ADDED_BACKGROUND),
        ),
        ToolDiffLineKind::Deletion => (
            "-",
            CellStyle::foreground(palette::RED),
            Some(palette::DIFF_REMOVED_BACKGROUND),
        ),
        ToolDiffLineKind::Context => (" ", CellStyle::foreground(palette::SUBTEXT_0).dim(), None),
        ToolDiffLineKind::Omission => unreachable!(),
    };
    cells.extend(styled_cells(marker, style.bold()));
    cells.extend(styled_cells(&sanitize_diff_fragment(&line.text), style));
    if let Some(background) = background {
        for cell in &mut cells {
            cell.style.background = background;
        }
        let used = cells_width(&cells).min(width);
        let padding = width.saturating_sub(used);
        if padding > 0 {
            cells.push(StyledCell::new(
                " ".repeat(usize::from(padding)),
                padding,
                CellStyle {
                    background,
                    ..CellStyle::default()
                },
            ));
        }
    }
    row_from_cells(id, cells, width)
}

fn sanitize_diff_fragment(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| match character {
            '\t' => "    ".chars().collect::<Vec<_>>(),
            character if character.is_control() => vec!['�'],
            character => vec![character],
        })
        .collect()
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

fn styled_cells(text: &str, style: CellStyle) -> Vec<StyledCell> {
    wrap_text("inline", text, u16::MAX, style)
        .into_iter()
        .next()
        .map(|row| row.cells)
        .unwrap_or_default()
}

fn cells_width(cells: &[StyledCell]) -> u16 {
    cells
        .iter()
        .fold(0u16, |width, cell| width.saturating_add(cell.width))
}

fn clip_cells(cells: Vec<StyledCell>, width: u16) -> Vec<StyledCell> {
    let mut used = 0u16;
    cells
        .into_iter()
        .take_while(|cell| {
            let fits = used.saturating_add(cell.width) <= width;
            if fits {
                used = used.saturating_add(cell.width);
            }
            fits
        })
        .collect()
}

pub(crate) fn row_from_cells(id: &str, cells: Vec<StyledCell>, width: u16) -> VisualRow {
    VisualRow {
        component_id: id.to_owned(),
        logical_line: 0,
        wrap_index: 0,
        cells: clip_cells(cells, width.max(1)),
    }
}

fn indent_styled_rows(rows: Vec<VisualRow>, prefix: &str, style: CellStyle) -> Vec<VisualRow> {
    rows.into_iter()
        .map(|mut row| {
            let mut cells = styled_cells(prefix, style);
            cells.extend(row.cells);
            row.cells = cells;
            row
        })
        .collect()
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

fn single_line_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
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

fn single_line_row(id: &str, text: &str, style: CellStyle, width: u16) -> VisualRow {
    wrap_text(
        id,
        &truncate(&single_line_text(text), usize::from(width.max(1))),
        width.max(1),
        style,
    )
    .into_iter()
    .next()
    .unwrap_or_else(|| VisualRow::blank(id))
}

pub(crate) fn render_turn_separator(
    id: &str,
    separator: &TurnSeparator,
    width: u16,
) -> Vec<VisualRow> {
    let approximate = if separator.estimated { "~" } else { "" };
    let label = format!(
        " Worked for {approximate}{} ─",
        format_turn_duration(separator.duration_ms)
    );
    let available = usize::from(width.max(1));
    let text = if display_width(&label) >= available {
        truncate(label.trim_start(), available)
    } else {
        format!(
            "{}{label}",
            "─".repeat(available.saturating_sub(display_width(&label)))
        )
    };
    vec![single_line_row(
        id,
        &text,
        CellStyle::foreground(palette::GRAY_FAINT),
        width,
    )]
}

pub(crate) fn format_turn_duration(duration_ms: u64) -> String {
    if duration_ms < 1_000 {
        return "<1s".to_owned();
    }
    let seconds = duration_ms / 1_000;
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m {:02}s", seconds % 60);
    }
    format!("{}h {:02}m", minutes / 60, minutes % 60)
}

pub(crate) fn render_viewer_item(
    id: &str,
    item: &TranscriptItem,
    width: u16,
    mode: TranscriptViewMode,
    expanded: bool,
    selected: bool,
) -> Vec<VisualRow> {
    let mut rows = match (mode, item) {
        (TranscriptViewMode::Summary, TranscriptItem::Tool(tool)) => {
            render_tool(id, tool, width, ToolRenderMode::Summary, 0)
        }
        (TranscriptViewMode::Summary, TranscriptItem::User(message)) => vec![single_line_row(
            id,
            &format!("› You · {}", message.text),
            CellStyle::foreground(Color::Blue),
            width,
        )],
        (TranscriptViewMode::Summary, TranscriptItem::Assistant(message)) => {
            let (text, style) = if message.text.is_empty() {
                (
                    &message.thinking,
                    CellStyle::foreground(palette::THINKING_TEXT),
                )
            } else {
                (&message.text, CellStyle::foreground(palette::TEXT))
            };
            vec![single_line_row(
                id,
                &format!("• Agent · {text}"),
                style,
                width,
            )]
        }
        (TranscriptViewMode::Summary, _) => {
            let summary = render_item(id, item, width, 0)
                .iter()
                .map(VisualRow::plain_text)
                .collect::<Vec<_>>()
                .join(" ");
            vec![single_line_row(
                id,
                &summary,
                CellStyle::foreground(Color::Gray),
                width,
            )]
        }
        (_, TranscriptItem::Tool(tool)) => render_tool(
            id,
            tool,
            width,
            if expanded {
                ToolRenderMode::Expanded
            } else {
                ToolRenderMode::Compact
            },
            0,
        ),
        (_, TranscriptItem::Plan(plan)) if expanded => render_plan(id, plan, width, true),
        _ => render_item(id, item, width, 0),
    };
    if selected {
        highlight_rows(&mut rows, width);
    }
    rows
}

fn highlight_rows(rows: &mut [VisualRow], width: u16) {
    let background = palette::SURFACE_0;
    for row in rows {
        for cell in &mut row.cells {
            cell.style.background = background;
        }
        let padding = width.saturating_sub(row.display_width());
        if padding > 0 {
            let style = CellStyle {
                background,
                ..CellStyle::default()
            };
            row.cells.push(StyledCell::new(
                " ".repeat(usize::from(padding)),
                padding,
                style,
            ));
        }
    }
}
