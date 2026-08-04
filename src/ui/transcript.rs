use std::{collections::HashMap, io, sync::Arc};

use crate::state::{
    AppState, ToolDiff, ToolDiffFile, ToolDiffLine, ToolDiffLineKind, ToolExecution, ToolStatus,
    TranscriptItem, TranscriptViewMode, TurnSeparator, UserMessage, UserMessageStatus,
};

use super::{
    markdown, palette, shell,
    text::{display_width, truncate, wrap_file_references, wrap_styled_lines, wrap_text},
    types::{CellStyle, Color, CommittedHistoryBlock, ComponentId, StyledCell, VisualRow},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentPhase {
    Streaming,
    Stable,
    Sealed,
    Committed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptBlock {
    pub id: ComponentId,
    pub item: TranscriptItem,
    leading_blank: bool,
    trailing_blank: bool,
}

pub trait TranscriptComponent {
    fn id(&self) -> &ComponentId;
    fn phase(&self) -> ComponentPhase;
    fn measure(&self, width: u16) -> usize;
    fn render(&self, width: u16) -> Vec<VisualRow>;
}

impl TranscriptComponent for TranscriptBlock {
    fn id(&self) -> &ComponentId {
        &self.id
    }

    fn phase(&self) -> ComponentPhase {
        match &self.item {
            TranscriptItem::User(message) => match message.status {
                UserMessageStatus::Pending => ComponentPhase::Streaming,
                UserMessageStatus::Accepted | UserMessageStatus::Failed => ComponentPhase::Sealed,
            },
            TranscriptItem::Assistant(message) => {
                if message.complete {
                    ComponentPhase::Sealed
                } else {
                    ComponentPhase::Streaming
                }
            }
            TranscriptItem::Tool(tool) => match tool.status {
                ToolStatus::WaitingApproval | ToolStatus::Running => ComponentPhase::Streaming,
                ToolStatus::Succeeded | ToolStatus::Failed | ToolStatus::Denied => {
                    ComponentPhase::Sealed
                }
            },
            _ => ComponentPhase::Sealed,
        }
    }

    fn measure(&self, width: u16) -> usize {
        self.render(width).len()
    }

    fn render(&self, width: u16) -> Vec<VisualRow> {
        self.render_animated(width, 0)
    }
}

impl TranscriptBlock {
    pub fn render_animated(&self, width: u16, animation_frame: u8) -> Vec<VisualRow> {
        let mut rows = render_item(&self.id, &self.item, width, animation_frame);
        if self.leading_blank {
            rows.insert(0, VisualRow::blank(self.id.clone()));
        }
        if self.trailing_blank {
            rows.push(VisualRow::blank(self.id.clone()));
        }
        rows
    }
}

pub trait HistorySink {
    fn append(&mut self, blocks: &[CommittedHistoryBlock]) -> io::Result<()>;
}

#[derive(Debug, Clone, Default)]
pub struct TranscriptStore {
    pub order: Vec<ComponentId>,
    pub components: HashMap<ComponentId, Arc<TranscriptBlock>>,
    pub revision: u64,
    phases: HashMap<ComponentId, ComponentPhase>,
    committed_cursor: usize,
}

impl TranscriptStore {
    pub fn sync(&mut self, state: &AppState) -> bool {
        let mut changed = self.order.len() != state.transcript.len();
        let mut order = Vec::with_capacity(state.transcript.len());
        let mut components = HashMap::with_capacity(state.transcript.len());
        let mut occurrences = HashMap::<String, usize>::new();

        for (index, item) in state.transcript.iter().enumerate() {
            let base = match item {
                TranscriptItem::Tool(tool) => format!("tool:{}", tool.id),
                TranscriptItem::TurnSeparator(separator) => {
                    format!("turn:{}", separator.turn_id)
                }
                _ => format!("transcript:{index}"),
            };
            let occurrence = occurrences.entry(base.clone()).or_default();
            let id = if *occurrence == 0 {
                base
            } else {
                format!("{base}:{}", *occurrence)
            };
            *occurrence += 1;
            let block = Arc::new(TranscriptBlock {
                id: id.clone(),
                item: item.clone(),
                leading_blank: index == 0 && matches!(item, TranscriptItem::User(_))
                    || index > 0
                        && (transcript_group(&state.transcript[index - 1])
                            != transcript_group(item)
                            || matches!(
                                (&state.transcript[index - 1], item),
                                (TranscriptItem::Tool(_), TranscriptItem::Tool(_))
                            )),
                trailing_blank: false,
            });
            changed |= self
                .components
                .get(&id)
                .is_none_or(|previous| previous.as_ref() != block.as_ref());
            order.push(id.clone());
            components.insert(id, block);
        }

        // A branch/session replacement may reuse positional IDs. Its canonical
        // projection starts over; committed terminal snapshots remain immutable.
        let prefix_unchanged = self
            .order
            .iter()
            .zip(order.iter())
            .take(self.committed_cursor)
            .all(|(old_id, new_id)| {
                old_id == new_id
                    && self
                        .components
                        .get(old_id)
                        .zip(components.get(new_id))
                        .is_some_and(|(old, new)| old.item == new.item)
            });
        if !prefix_unchanged || order.len() < self.committed_cursor {
            self.committed_cursor = 0;
            self.phases.clear();
            changed = true;
        }

        if changed {
            self.order = order;
            self.components = components;
            self.revision = self.revision.saturating_add(1);
            self.refresh_phases();
        }
        changed
    }

    fn refresh_phases(&mut self) {
        for id in &self.order {
            if self.phases.get(id) == Some(&ComponentPhase::Committed) {
                continue;
            }
            if let Some(block) = self.components.get(id) {
                self.phases.insert(id.clone(), block.phase());
            }
        }
        self.phases.retain(|id, _| self.components.contains_key(id));
    }

    pub fn phase(&self, id: &str) -> Option<ComponentPhase> {
        self.phases.get(id).copied()
    }

    pub fn committed_cursor(&self) -> usize {
        self.committed_cursor
    }

    pub fn active_components(&self) -> impl Iterator<Item = &Arc<TranscriptBlock>> {
        self.order[self.committed_cursor.min(self.order.len())..]
            .iter()
            .filter_map(|id| self.components.get(id))
    }

    pub fn active_components_after(
        &self,
        pending_history: usize,
    ) -> impl Iterator<Item = &Arc<TranscriptBlock>> {
        let start = self
            .committed_cursor
            .saturating_add(pending_history)
            .min(self.order.len());
        self.order[start..]
            .iter()
            .filter_map(|id| self.components.get(id))
    }

    pub fn pending_history(
        &self,
        width: u16,
        source_revision: u64,
        maximum_rows: u16,
    ) -> Vec<CommittedHistoryBlock> {
        let mut blocks = Vec::new();
        let mut rows = 0u16;
        for id in self.order.iter().skip(self.committed_cursor) {
            if self.phase(id) != Some(ComponentPhase::Sealed) {
                break;
            }
            let Some(component) = self.components.get(id) else {
                break;
            };
            let rendered = component.render(width.max(1));
            let rendered_rows = u16::try_from(rendered.len()).unwrap_or(u16::MAX);
            if !blocks.is_empty() && rows.saturating_add(rendered_rows) > maximum_rows {
                break;
            }
            rows = rows.saturating_add(rendered_rows);
            blocks.push(CommittedHistoryBlock {
                component_id: id.clone(),
                source_revision,
                rows: rendered,
            });
            if rows >= maximum_rows {
                break;
            }
        }
        blocks
    }

    /// Advances history only after a successful terminal commit.
    pub fn acknowledge_history(&mut self, blocks: &[CommittedHistoryBlock]) {
        for block in blocks {
            let expected = self.order.get(self.committed_cursor);
            if expected != Some(&block.component_id) {
                break;
            }
            self.phases
                .insert(block.component_id.clone(), ComponentPhase::Committed);
            self.committed_cursor += 1;
        }
    }

    pub fn reset_projection(&mut self) {
        self.committed_cursor = 0;
        self.phases.clear();
        self.refresh_phases();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TranscriptGroup {
    User,
    Assistant,
    Tool,
    Turn,
    Other,
}

fn transcript_group(item: &TranscriptItem) -> TranscriptGroup {
    match item {
        TranscriptItem::User(_) => TranscriptGroup::User,
        TranscriptItem::Assistant(_) => TranscriptGroup::Assistant,
        TranscriptItem::Tool(_) => TranscriptGroup::Tool,
        TranscriptItem::TurnSeparator(_) => TranscriptGroup::Turn,
        _ => TranscriptGroup::Other,
    }
}

fn render_item(id: &str, item: &TranscriptItem, width: u16, animation_frame: u8) -> Vec<VisualRow> {
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
        let body = format!(
            "**Plan · {} [{}]**\n\n{}\n\n{}",
            plan.title,
            plan.status.label(),
            plan.summary,
            plan.body_markdown
        );
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
        return rows;
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
        TranscriptItem::Goal(snapshot) => {
            let body = snapshot.goal.as_ref().map_or_else(
                || "No active Goal".to_owned(),
                |goal| {
                    format!(
                        "{} [{}]\n{} tasks",
                        goal.objective,
                        goal.stage,
                        goal.tasks.len()
                    )
                },
            );
            ("Goal", body, CellStyle::foreground(Color::Cyan))
        }
        TranscriptItem::Goals(snapshot) => (
            "Goals",
            snapshot
                .goals
                .iter()
                .map(|goal| format!("{} [{}]", goal.objective, goal.stage))
                .collect::<Vec<_>>()
                .join("\n"),
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

fn render_user(id: &str, message: &UserMessage, width: u16) -> Vec<VisualRow> {
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
enum ToolRenderMode {
    Compact,
    Expanded,
    Summary,
}

const COMPACT_DIFF_LINES_PER_FILE: usize = 40;

fn render_tool(
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

fn row_from_cells(id: &str, cells: Vec<StyledCell>, width: u16) -> VisualRow {
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

fn render_turn_separator(id: &str, separator: &TurnSeparator, width: u16) -> Vec<VisualRow> {
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{
        rpc::PiState,
        state::{AssistantMessage, ToolExecution, UserMessage},
    };

    use super::*;

    fn state() -> AppState {
        AppState::new(PiState {
            model: Some(json!({"provider": "test", "id": "model"})),
            thinking_level: "off".to_owned(),
            is_streaming: false,
            is_compacting: false,
            steering_mode: "one-at-a-time".to_owned(),
            follow_up_mode: "one-at-a-time".to_owned(),
            session_file: None,
            session_id: "session".to_owned(),
            session_name: None,
            auto_compaction_enabled: true,
            message_count: 0,
            pending_message_count: 0,
        })
    }

    #[test]
    fn only_the_contiguous_sealed_prefix_can_enter_history() {
        let mut state = state();
        state.transcript = vec![
            TranscriptItem::User(UserMessage {
                text: "done".to_owned(),
                status: UserMessageStatus::Accepted,
            }),
            TranscriptItem::Assistant(AssistantMessage {
                text: "stream".to_owned(),
                complete: false,
                ..AssistantMessage::default()
            }),
            TranscriptItem::Notice("later".to_owned()),
        ];
        let mut store = TranscriptStore::default();
        assert!(store.sync(&state));
        let pending = store.pending_history(80, 1, 24);
        assert_eq!(pending.len(), 1);
        assert_eq!(store.committed_cursor(), 0);
        store.acknowledge_history(&pending);
        assert_eq!(store.committed_cursor(), 1);
        assert_eq!(
            store.phase(&pending[0].component_id),
            Some(ComponentPhase::Committed)
        );
    }

    #[test]
    fn assistant_messages_render_markdown_in_the_primary_transcript() {
        let block = TranscriptBlock {
            id: "assistant:markdown".to_owned(),
            item: TranscriptItem::Assistant(AssistantMessage {
                text: "# Result\n\nUse **bold** and `cargo test`.\n\n- first\n- second".to_owned(),
                complete: true,
                ..AssistantMessage::default()
            }),
            leading_blank: false,
            trailing_blank: false,
        };

        let rows = block.render(42);
        let text = rows
            .iter()
            .map(VisualRow::plain_text)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rows[0].plain_text().starts_with("• # Result"));
        assert!(text.contains("Use bold and cargo test."));
        assert!(text.contains("  - first"));
        assert!(rows.iter().all(|row| row.display_width() <= 42));
        assert!(
            rows.iter()
                .flat_map(|row| &row.cells)
                .any(|cell| { cell.symbol == "b" && cell.style.bold })
        );
        assert!(
            rows.iter()
                .flat_map(|row| &row.cells)
                .any(|cell| { cell.symbol == "c" && cell.style.foreground == palette::SAPPHIRE })
        );
    }

    #[test]
    fn tool_updates_replace_the_canonical_snapshot_by_id() {
        let mut state = state();
        state.transcript.push(TranscriptItem::Tool(ToolExecution {
            id: "call-1".to_owned(),
            name: "read".to_owned(),
            args: json!({"path": "a"}),
            output: "partial".to_owned(),
            diff: None,
            status: ToolStatus::Running,
        }));
        let mut store = TranscriptStore::default();
        store.sync(&state);
        let revision = store.revision;
        let TranscriptItem::Tool(tool) = &mut state.transcript[0] else {
            unreachable!()
        };
        tool.output = "complete".to_owned();
        tool.status = ToolStatus::Succeeded;
        store.sync(&state);
        assert!(store.revision > revision);
        assert_eq!(store.order, vec!["tool:call-1"]);
        assert_eq!(store.phase("tool:call-1"), Some(ComponentPhase::Sealed));
        assert_eq!(
            store.active_components().next().unwrap().render(40).len(),
            2
        );
    }

    #[test]
    fn primary_tools_are_two_line_summaries_and_never_render_output_body() {
        let tool = ToolExecution {
            id: "call-1".to_owned(),
            name: "bash".to_owned(),
            args: json!({"command": "cargo test --all"}),
            output: "PRIVATE OUTPUT\nsecond line\nthird line".to_owned(),
            diff: None,
            status: ToolStatus::Succeeded,
        };

        let rows = render_item("tool:call-1", &TranscriptItem::Tool(tool), 60, 0);
        let text = rows
            .iter()
            .map(VisualRow::plain_text)
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(rows.len(), 2);
        assert!(text.contains("• Ran"));
        assert!(text.contains("└ cargo test --all"));
        assert!(text.contains("· 3 lines"));
        assert!(!text.contains("succeeded"));
        assert!(!text.contains("PRIVATE OUTPUT"));
    }

    #[test]
    fn successful_structured_edits_render_inline_file_diffs() {
        let patch = "\
--- a/one.rs
+++ b/one.rs
@@ -1,2 +1,2 @@
-old
+new
 same
--- a/two.rs
+++ b/two.rs
@@ -0,0 +1,2 @@
+first
+second
--- a/three.rs
+++ b/three.rs
@@ -1 +0,0 @@
-gone
";
        let tool = ToolExecution {
            id: "edit-1".to_owned(),
            name: "edit".to_owned(),
            args: serde_json::Value::Null,
            output: "done".to_owned(),
            diff: crate::state::parse_tool_diff(&serde_json::Value::Null, &json!({"patch": patch})),
            status: ToolStatus::Succeeded,
        };

        let rows = render_tool("edit", &tool, 80, ToolRenderMode::Compact, 0);
        let text = rows
            .iter()
            .map(VisualRow::plain_text)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Edited 3 files (+3 -2)"));
        assert!(text.contains("└ one.rs (+1 -1)"));
        assert!(text.contains("└ two.rs (+2 -0)"));
        assert!(text.contains("└ three.rs (+0 -1)"));
        assert!(text.contains("1 -old"));
        assert!(text.contains("1 +new"));
        assert!(rows.iter().all(|row| row.display_width() <= 80));
        assert!(
            rows.iter()
                .flat_map(|row| &row.cells)
                .any(|cell| { cell.symbol == "+" && cell.style.foreground == palette::GREEN })
        );
        assert!(
            rows.iter()
                .flat_map(|row| &row.cells)
                .any(|cell| { cell.symbol == "-" && cell.style.foreground == palette::RED })
        );
        let added = rows
            .iter()
            .find(|row| row.plain_text().contains("1 +new"))
            .expect("added diff row");
        assert!(
            added
                .cells
                .iter()
                .all(|cell| cell.style.background == palette::DIFF_ADDED_BACKGROUND)
        );
        assert_eq!(added.display_width(), 80);
        let removed = rows
            .iter()
            .find(|row| row.plain_text().contains("1 -old"))
            .expect("removed diff row");
        assert!(
            removed
                .cells
                .iter()
                .all(|cell| cell.style.background == palette::DIFF_REMOVED_BACKGROUND)
        );
        assert_eq!(removed.display_width(), 80);
        let context = rows
            .iter()
            .find(|row| row.plain_text().contains("2  same"))
            .expect("context diff row");
        assert!(
            context
                .cells
                .iter()
                .all(|cell| cell.style.background == Color::Default)
        );
        assert!(context.display_width() < 80);
        assert!(
            rows[0]
                .cells
                .iter()
                .all(|cell| cell.style.background == Color::Default)
        );
    }

    #[test]
    fn compact_edit_diffs_are_bounded_and_viewer_expansion_is_complete() {
        let display_diff = (1..=45)
            .map(|line| format!("+{line} line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let tool = ToolExecution {
            id: "edit-large".to_owned(),
            name: "edit".to_owned(),
            args: json!({"path": "src/\u{1b}[31mlib.rs"}),
            output: "done".to_owned(),
            diff: crate::state::parse_tool_diff(
                &json!({"path": "src/\u{1b}[31mlib.rs"}),
                &json!({"diff": display_diff}),
            ),
            status: ToolStatus::Succeeded,
        };

        let compact = render_tool("edit", &tool, 64, ToolRenderMode::Compact, 0);
        let compact_text = compact
            .iter()
            .map(VisualRow::plain_text)
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(compact.len(), 43);
        assert!(compact_text.contains("5 more diff lines"));
        assert!(!compact_text.contains('\u{1b}'));

        let expanded = render_viewer_item(
            "edit",
            &TranscriptItem::Tool(tool),
            64,
            TranscriptViewMode::Normal,
            true,
            false,
        );
        assert_eq!(expanded.len(), 47);
        assert!(
            expanded
                .last()
                .unwrap()
                .plain_text()
                .contains("45 +line 45")
        );
    }

    #[test]
    fn primary_shell_tools_use_token_colors_and_only_surface_non_success_states() {
        let tool = ToolExecution {
            id: "call-shell".to_owned(),
            name: "bash".to_owned(),
            args: json!({"command": "cargo test --all ./src | rg '你好'"}),
            output: String::new(),
            diff: None,
            status: ToolStatus::Succeeded,
        };
        let rows = render_tool("tool", &tool, 80, ToolRenderMode::Compact, 0);
        assert_eq!(rows.len(), 2);
        assert!(!rows[1].plain_text().contains("0 B"));
        for color in [
            palette::SAPPHIRE,
            palette::BLUE,
            palette::GREEN,
            palette::PEACH,
            palette::YELLOW,
        ] {
            assert!(
                rows[1]
                    .cells
                    .iter()
                    .any(|cell| cell.style.foreground == color),
                "missing {color:?}"
            );
        }

        let running = render_tool(
            "running",
            &ToolExecution {
                status: ToolStatus::Running,
                ..tool.clone()
            },
            80,
            ToolRenderMode::Compact,
            2,
        );
        assert!(running[0].plain_text().starts_with("⠹ Ran"));
        assert!(running[1].plain_text().ends_with("· running"));

        let waiting = render_tool(
            "waiting",
            &ToolExecution {
                status: ToolStatus::WaitingApproval,
                ..tool.clone()
            },
            80,
            ToolRenderMode::Compact,
            2,
        );
        assert!(waiting[0].plain_text().starts_with("● Ran"));
        assert_eq!(waiting[0].cells[0].style.foreground, palette::YELLOW);

        let failed = render_tool(
            "failed",
            &ToolExecution {
                output: "private failure body".to_owned(),
                status: ToolStatus::Failed,
                ..tool
            },
            80,
            ToolRenderMode::Compact,
            0,
        );
        assert_eq!(failed.len(), 3);
        assert!(failed[2].plain_text().contains("failed"));
        assert!(
            !failed
                .iter()
                .map(VisualRow::plain_text)
                .collect::<String>()
                .contains("private failure body")
        );
    }

    #[test]
    fn transcript_viewer_modes_fold_expand_summarize_and_highlight_tools() {
        let item = TranscriptItem::Tool(ToolExecution {
            id: "call-1".to_owned(),
            name: "read_file".to_owned(),
            args: json!({"path": "src/lib.rs", "line": 4}),
            output: "complete output\nwith every line".to_owned(),
            diff: None,
            status: ToolStatus::Succeeded,
        });

        let normal = render_viewer_item(
            "viewer",
            &item,
            48,
            TranscriptViewMode::Normal,
            false,
            false,
        );
        let normal_text = normal
            .iter()
            .map(VisualRow::plain_text)
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(normal.len(), 2);
        assert!(!normal_text.contains("complete output"));
        assert!(!normal_text.contains("\"line\""));

        let verbose =
            render_viewer_item("viewer", &item, 48, TranscriptViewMode::Verbose, true, true);
        let verbose_text = verbose
            .iter()
            .map(VisualRow::plain_text)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(verbose_text.contains("Arguments"));
        assert!(verbose_text.contains("\"line\": 4"));
        assert!(verbose_text.contains("complete output"));
        assert!(
            verbose
                .iter()
                .flat_map(|row| &row.cells)
                .all(|cell| cell.style.background == palette::SURFACE_0)
        );
        assert!(verbose.iter().all(|row| row.display_width() == 48));

        let summary = render_viewer_item(
            "viewer",
            &item,
            48,
            TranscriptViewMode::Summary,
            true,
            false,
        );
        assert_eq!(summary.len(), 1);
        assert!(!summary[0].plain_text().contains("complete output"));
    }

    #[test]
    fn user_messages_use_full_width_unicode_safe_frames_and_narrow_fallbacks() {
        let message = UserMessage {
            text: "Markdown **stays literal** and CJK wraps: 你好世界".to_owned(),
            status: UserMessageStatus::Accepted,
        };
        let rows = render_user("user", &message, 40);
        assert!(rows[0].plain_text().starts_with('╭'));
        assert!(rows.last().unwrap().plain_text().starts_with('╰'));
        assert!(rows.iter().all(|row| row.display_width() == 40));
        assert!(
            rows.iter()
                .map(VisualRow::plain_text)
                .collect::<String>()
                .contains("**stays literal**")
        );
        assert_eq!(rows[0].cells[0].style.foreground, palette::HISTORY_BORDER);
        assert!(rows[0].cells[0].style.dim);

        let narrow = render_user(
            "narrow",
            &UserMessage {
                status: UserMessageStatus::Failed,
                ..message
            },
            5,
        );
        assert!(narrow[0].plain_text().starts_with("› "));
        assert!(narrow.iter().all(|row| row.display_width() <= 5));
        assert!(
            narrow
                .iter()
                .flat_map(|row| &row.cells)
                .any(|cell| cell.style.foreground == Color::Red)
        );
    }

    #[test]
    fn transcript_spacing_never_duplicates_blank_rows_between_groups() {
        let mut state = state();
        state.transcript = vec![
            TranscriptItem::User(UserMessage {
                text: "question".to_owned(),
                status: UserMessageStatus::Accepted,
            }),
            TranscriptItem::Assistant(AssistantMessage {
                text: "answer".to_owned(),
                complete: true,
                ..AssistantMessage::default()
            }),
            TranscriptItem::Tool(ToolExecution {
                id: "tool".to_owned(),
                name: "read".to_owned(),
                args: json!({"path": "src/lib.rs"}),
                output: String::new(),
                diff: None,
                status: ToolStatus::Succeeded,
            }),
        ];
        let mut store = TranscriptStore::default();
        store.sync(&state);
        let rows = store
            .active_components()
            .flat_map(|component| component.render(40))
            .collect::<Vec<_>>();
        assert!(
            !rows
                .windows(2)
                .any(|pair| pair[0].plain_text().is_empty() && pair[1].plain_text().is_empty())
        );
    }

    #[test]
    fn thinking_and_each_tool_call_have_single_blank_separators() {
        let mut state = state();
        state.transcript = vec![
            TranscriptItem::Assistant(AssistantMessage {
                text: "final answer".to_owned(),
                thinking: "consider the options".to_owned(),
                complete: true,
            }),
            TranscriptItem::Tool(ToolExecution {
                id: "tool-1".to_owned(),
                name: "read".to_owned(),
                args: json!({"path": "src/lib.rs"}),
                output: String::new(),
                diff: None,
                status: ToolStatus::Succeeded,
            }),
            TranscriptItem::Tool(ToolExecution {
                id: "tool-2".to_owned(),
                name: "find".to_owned(),
                args: json!({"query": "palette"}),
                output: String::new(),
                diff: None,
                status: ToolStatus::Succeeded,
            }),
        ];
        let mut store = TranscriptStore::default();
        store.sync(&state);
        let blocks = store.active_components().collect::<Vec<_>>();
        assert!(blocks[1].render(48)[0].plain_text().is_empty());
        assert!(blocks[2].render(48)[0].plain_text().is_empty());

        let assistant = blocks[0].render(48);
        assert!(
            assistant.iter().flat_map(|row| &row.cells).any(|cell| {
                cell.symbol == "c" && cell.style.foreground == palette::THINKING_TEXT
            })
        );
        assert!(
            assistant
                .iter()
                .flat_map(|row| &row.cells)
                .any(|cell| { cell.symbol == "f" && cell.style.foreground == palette::TEXT })
        );
    }

    #[test]
    fn turn_duration_formats_and_estimated_separator_are_stable() {
        assert_eq!(format_turn_duration(0), "<1s");
        assert_eq!(format_turn_duration(999), "<1s");
        assert_eq!(format_turn_duration(12_999), "12s");
        assert_eq!(format_turn_duration(65_000), "1m 05s");
        assert_eq!(format_turn_duration(3_720_000), "1h 02m");

        let rows = render_turn_separator(
            "turn",
            &TurnSeparator {
                turn_id: "turn".to_owned(),
                started_at: "start".to_owned(),
                ended_at: "end".to_owned(),
                duration_ms: 65_000,
                estimated: true,
            },
            40,
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].display_width(), 40);
        assert!(rows[0].plain_text().ends_with(" Worked for ~1m 05s ─"));
    }
}
