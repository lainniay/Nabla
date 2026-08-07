use crate::state::{PlanArtifact, TranscriptItem};
use crate::ui::{
    markdown, palette,
    types::{AssistantContentKind, AssistantSegment, CellStyle, Color, StyledCell, VisualRow},
};

pub(crate) fn render_assistant_segment(
    id: &str,
    item: &TranscriptItem,
    segment: &AssistantSegment,
    width: u16,
) -> Vec<VisualRow> {
    let TranscriptItem::Assistant(message) = item else {
        return super::render_item(id, item, width, 0);
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

pub(crate) fn prefix_assistant_rows(
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

pub(crate) fn render_plan(
    id: &str,
    plan: &PlanArtifact,
    width: u16,
    expanded: bool,
) -> Vec<VisualRow> {
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
