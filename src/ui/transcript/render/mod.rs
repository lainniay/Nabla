use crate::state::{TranscriptItem, TranscriptViewMode};
use crate::ui::{
    markdown, palette,
    text::wrap_text,
    types::{CellStyle, Color, StyledCell, VisualRow},
};

pub(crate) mod assistant;
pub(crate) mod common;
pub(crate) mod diff;
pub(crate) mod tool;
pub(crate) mod user;

pub(crate) use assistant::render_assistant_segment;
pub(crate) use common::{render_turn_separator, row_from_cells, wrap_styled_breaking};
pub(crate) use tool::{ToolRenderMode, render_tool, tool_operation_summary};
pub(crate) use user::render_user;

use assistant::{prefix_assistant_rows, render_plan};
use common::single_line_row;

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
