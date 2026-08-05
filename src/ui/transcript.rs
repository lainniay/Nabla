use std::{
    collections::{HashMap, HashSet},
    io,
    sync::{Arc, Mutex},
};

use crate::state::{
    AppState, ToolDiff, ToolDiffFile, ToolDiffLine, ToolDiffLineKind, ToolExecution, ToolStatus,
    TranscriptItem, TranscriptViewMode, TurnSeparator, UserMessage, UserMessageStatus,
};

use super::{
    markdown, palette, shell,
    text::{display_width, truncate, wrap_file_references, wrap_styled_lines, wrap_text},
    types::{
        AssistantContentKind, AssistantSegment, AssistantSegmentPhase, CanonicalReflowProjection,
        CellStyle, Color, CommittedHistoryBlock, ComponentId, StyledCell, VisualRow,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentPhase {
    Streaming,
    Stable,
    Sealed,
    Committed,
}

type RenderCacheKey = (u16, u64, u64);
type RenderCache = Arc<Mutex<HashMap<RenderCacheKey, Vec<VisualRow>>>>;

#[derive(Debug, Clone)]
pub struct TranscriptBlock {
    pub id: ComponentId,
    pub item: TranscriptItem,
    pub assistant_segment: Option<AssistantSegment>,
    leading_blank: bool,
    trailing_blank: bool,
    render_cache: RenderCache,
}

impl PartialEq for TranscriptBlock {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.item == other.item
            && self.assistant_segment == other.assistant_segment
            && self.leading_blank == other.leading_blank
            && self.trailing_blank == other.trailing_blank
    }
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
        if let Some(segment) = &self.assistant_segment {
            return match segment.phase {
                AssistantSegmentPhase::Streaming => ComponentPhase::Streaming,
                AssistantSegmentPhase::Stable => ComponentPhase::Stable,
                AssistantSegmentPhase::Sealed => ComponentPhase::Sealed,
                AssistantSegmentPhase::Committed => ComponentPhase::Committed,
            };
        }
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
        let animated = matches!(
            &self.item,
            TranscriptItem::Tool(tool)
                if matches!(tool.status, ToolStatus::WaitingApproval | ToolStatus::Running)
        );
        let cache_key = (
            width,
            palette::THEME_REVISION,
            u64::from(animated) * u64::from(animation_frame),
        );
        if let Some(rows) = self
            .render_cache
            .lock()
            .expect("transcript render cache poisoned")
            .get(&cache_key)
            .cloned()
        {
            return rows;
        }
        let mut rows = if let Some(segment) = &self.assistant_segment {
            render_assistant_segment(&self.id, &self.item, segment, width)
        } else {
            render_item(&self.id, &self.item, width, animation_frame)
        };
        if self.leading_blank {
            rows.insert(0, VisualRow::blank(self.id.clone()));
        }
        if self.trailing_blank {
            rows.push(VisualRow::blank(self.id.clone()));
        }
        self.render_cache
            .lock()
            .expect("transcript render cache poisoned")
            .insert(cache_key, rows.clone());
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
    committed_row_offset: usize,
    session_epoch: u64,
    assistant_scans: HashMap<(u64, u64, AssistantContentKind), markdown::IncrementalMarkdown>,
}

impl TranscriptStore {
    pub fn sync(&mut self, state: &AppState) -> bool {
        let epoch_changed = self.session_epoch != state.session_epoch;
        if epoch_changed {
            self.session_epoch = state.session_epoch;
            self.committed_cursor = 0;
            self.committed_row_offset = 0;
            self.phases.clear();
            self.assistant_scans.clear();
        }
        let mut order = Vec::with_capacity(state.transcript.len());
        let mut components = HashMap::with_capacity(state.transcript.len());
        let mut occurrences = HashMap::<String, usize>::new();
        let mut active_scan_keys = HashSet::new();

        for (index, item) in state.transcript.iter().enumerate() {
            let leading_blank = index == 0 && matches!(item, TranscriptItem::User(_))
                || index > 0
                    && (transcript_group(&state.transcript[index - 1]) != transcript_group(item)
                        || matches!(
                            (&state.transcript[index - 1], item),
                            (TranscriptItem::Tool(_), TranscriptItem::Tool(_))
                        ));
            if let TranscriptItem::Assistant(message) = item {
                let mut projected = project_assistant(
                    message,
                    state.session_epoch,
                    u64::try_from(index).unwrap_or(u64::MAX).saturating_add(1),
                    &mut self.assistant_scans,
                    &mut active_scan_keys,
                );
                if let Some(first) = projected.first_mut() {
                    first.leading_blank = leading_blank;
                }
                for block in projected {
                    insert_projected_block(block, &self.components, &mut order, &mut components);
                }
                continue;
            }

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
            let block = TranscriptBlock {
                id: id.clone(),
                item: item.clone(),
                assistant_segment: None,
                leading_blank,
                trailing_blank: false,
                render_cache: Arc::new(Mutex::new(HashMap::new())),
            };
            insert_projected_block(block, &self.components, &mut order, &mut components);
        }
        self.assistant_scans
            .retain(|key, _| active_scan_keys.contains(key));

        let mut changed = epoch_changed
            || self.order != order
            || order.iter().any(|id| {
                self.components
                    .get(id)
                    .zip(components.get(id))
                    .is_none_or(|(old, new)| old.as_ref() != new.as_ref())
            });

        // A branch/session replacement may reuse positional IDs. Its canonical
        // projection starts over; semantic components remain available to reflow.
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
                        .is_some_and(|(old, new)| old.as_ref() == new.as_ref())
            });
        let partial_component_unchanged = self.committed_row_offset == 0
            || self
                .order
                .get(self.committed_cursor)
                .zip(order.get(self.committed_cursor))
                .is_some_and(|(old_id, new_id)| {
                    old_id == new_id
                        && self
                            .components
                            .get(old_id)
                            .zip(components.get(new_id))
                            .is_some_and(|(old, new)| old.as_ref() == new.as_ref())
                });
        if !prefix_unchanged || !partial_component_unchanged || order.len() < self.committed_cursor
        {
            self.committed_cursor = 0;
            self.committed_row_offset = 0;
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

    pub fn committed_row_offset(&self) -> usize {
        self.committed_row_offset
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

    pub fn active_rows_after_history(
        &self,
        width: u16,
        animation_frame: u8,
        pending: &[CommittedHistoryBlock],
    ) -> Vec<VisualRow> {
        let mut rows = Vec::new();
        for (component_index, id) in self.order.iter().enumerate().skip(self.committed_cursor) {
            let Some(component) = self.components.get(id) else {
                continue;
            };
            let rendered = component.render_animated(width, animation_frame);
            let mut start = if component_index == self.committed_cursor {
                self.committed_row_offset.min(rendered.len())
            } else {
                0
            };
            if let Some(block) = pending.iter().find(|block| &block.component_id == id)
                && block.row_offset == start
            {
                start = start.saturating_add(block.rows.len()).min(rendered.len());
            }
            rows.extend(rendered.into_iter().skip(start));
        }
        rows
    }

    pub fn pending_history(
        &self,
        width: u16,
        source_revision: u64,
        maximum_rows: usize,
    ) -> Vec<CommittedHistoryBlock> {
        self.pending_history_budget(width, source_revision, maximum_rows, usize::MAX)
    }

    pub fn pending_history_budget(
        &self,
        width: u16,
        source_revision: u64,
        maximum_rows: usize,
        maximum_bytes: usize,
    ) -> Vec<CommittedHistoryBlock> {
        if maximum_rows == 0 || maximum_bytes == 0 {
            return Vec::new();
        }
        let mut blocks = Vec::new();
        let mut remaining_rows = maximum_rows;
        let mut remaining_bytes = maximum_bytes;
        for (component_index, id) in self.order.iter().enumerate().skip(self.committed_cursor) {
            if !matches!(
                self.phase(id),
                Some(ComponentPhase::Stable | ComponentPhase::Sealed)
            ) {
                break;
            }
            let Some(component) = self.components.get(id) else {
                break;
            };
            let rendered = component.render(width.max(1));
            let row_offset = if component_index == self.committed_cursor {
                self.committed_row_offset.min(rendered.len())
            } else {
                0
            };
            let mut selected = Vec::new();
            for row in rendered.iter().skip(row_offset) {
                if remaining_rows == 0 {
                    break;
                }
                let row_bytes = row
                    .cells
                    .iter()
                    .map(|cell| cell.symbol.len())
                    .sum::<usize>();
                if row_bytes > remaining_bytes {
                    if selected.is_empty() && blocks.is_empty() {
                        // Always make progress for one oversized physical row.
                        selected.push(row.clone());
                        remaining_rows = remaining_rows.saturating_sub(1);
                        remaining_bytes = 0;
                    }
                    break;
                }
                selected.push(row.clone());
                remaining_rows -= 1;
                remaining_bytes = remaining_bytes.saturating_sub(row_bytes);
            }
            if selected.is_empty() {
                break;
            }
            blocks.push(CommittedHistoryBlock {
                component_id: id.clone(),
                source_revision,
                row_offset,
                total_rows: rendered.len(),
                rows: selected,
            });
            if remaining_rows == 0 || remaining_bytes == 0 {
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
            if block.row_offset != self.committed_row_offset || block.rows.is_empty() {
                break;
            }
            let acknowledged = block.row_offset.saturating_add(block.rows.len());
            if acknowledged < block.total_rows {
                self.committed_row_offset = acknowledged;
                break;
            }
            if acknowledged == block.total_rows {
                self.phases
                    .insert(block.component_id.clone(), ComponentPhase::Committed);
                self.committed_cursor += 1;
                self.committed_row_offset = 0;
            } else {
                break;
            }
        }
    }

    pub fn reset_projection(&mut self) {
        self.committed_cursor = 0;
        self.committed_row_offset = 0;
        self.phases.clear();
        self.refresh_phases();
    }

    pub fn render_canonical_history(&self, width: u16) -> Vec<VisualRow> {
        self.order
            .iter()
            .filter_map(|id| self.components.get(id))
            .flat_map(|component| component.render(width.max(1)))
            .collect()
    }

    pub fn rebuild_projection(&mut self, width: u16) -> Vec<VisualRow> {
        self.reset_projection();
        self.render_canonical_history(width)
    }

    pub fn invalidate_render_caches(&self) {
        for component in self.components.values() {
            component
                .render_cache
                .lock()
                .expect("transcript render cache poisoned")
                .clear();
        }
    }

    pub fn canonical_reflow_projection(
        &self,
        width: u16,
        source_revision: u64,
        maximum_rows: usize,
    ) -> CanonicalReflowProjection {
        let width = width.max(1);
        let history_end_cursor = self
            .order
            .iter()
            .position(|id| {
                !matches!(
                    self.phase(id),
                    Some(
                        ComponentPhase::Stable | ComponentPhase::Sealed | ComponentPhase::Committed
                    )
                )
            })
            .unwrap_or(self.order.len());
        let rendered = self.order[..history_end_cursor]
            .iter()
            .filter_map(|id| {
                self.components
                    .get(id)
                    .map(|component| (id.clone(), component.render(width)))
            })
            .collect::<Vec<_>>();

        let mut selected_start = rendered.len();
        let mut selected_rows = 0usize;
        for (index, (_, rows)) in rendered.iter().enumerate().rev() {
            if maximum_rows > 0
                && selected_start < rendered.len()
                && selected_rows.saturating_add(rows.len()) > maximum_rows
            {
                break;
            }
            selected_start = index;
            selected_rows = selected_rows.saturating_add(rows.len());
            if maximum_rows > 0 && selected_rows >= maximum_rows {
                break;
            }
        }

        let history_blocks = rendered[selected_start..]
            .iter()
            .map(|(component_id, rows)| CommittedHistoryBlock {
                component_id: component_id.clone(),
                source_revision,
                row_offset: 0,
                total_rows: rows.len(),
                rows: rows.clone(),
            })
            .collect::<Vec<_>>();
        let active_rows = self.order[history_end_cursor..]
            .iter()
            .filter_map(|id| self.components.get(id))
            .flat_map(|component| component.render(width))
            .collect();

        CanonicalReflowProjection {
            canonical_revision: self.revision,
            source_revision,
            width,
            omitted_components: selected_start,
            history_end_cursor,
            history_blocks,
            active_rows,
        }
    }

    pub fn apply_reflow_projection(&mut self, projection: &CanonicalReflowProjection) -> bool {
        if projection.canonical_revision != self.revision
            || projection.history_end_cursor > self.order.len()
            || projection.omitted_components > projection.history_end_cursor
        {
            return false;
        }
        let projected_ids =
            &self.order[projection.omitted_components..projection.history_end_cursor];
        if projected_ids.len() != projection.history_blocks.len()
            || projected_ids
                .iter()
                .zip(&projection.history_blocks)
                .any(|(id, block)| id != &block.component_id)
        {
            return false;
        }

        self.committed_cursor = projection.history_end_cursor;
        self.committed_row_offset = 0;
        self.phases.clear();
        self.refresh_phases();
        for id in &self.order[..self.committed_cursor] {
            self.phases.insert(id.clone(), ComponentPhase::Committed);
        }
        true
    }
}

fn insert_projected_block(
    block: TranscriptBlock,
    previous: &HashMap<ComponentId, Arc<TranscriptBlock>>,
    order: &mut Vec<ComponentId>,
    components: &mut HashMap<ComponentId, Arc<TranscriptBlock>>,
) {
    let id = block.id.clone();
    let block = previous
        .get(&id)
        .filter(|previous| previous.as_ref() == &block)
        .cloned()
        .unwrap_or_else(|| Arc::new(block));
    order.push(id.clone());
    components.insert(id, block);
}

fn project_assistant(
    message: &crate::state::AssistantMessage,
    state_epoch: u64,
    fallback_message_id: u64,
    scans: &mut HashMap<(u64, u64, AssistantContentKind), markdown::IncrementalMarkdown>,
    active_keys: &mut HashSet<(u64, u64, AssistantContentKind)>,
) -> Vec<TranscriptBlock> {
    let epoch = if message.session_epoch == 0 {
        state_epoch
    } else {
        message.session_epoch
    };
    let message_id = if message.id == 0 {
        fallback_message_id
    } else {
        message.id
    };
    let mut blocks = Vec::new();
    project_assistant_content(
        epoch,
        message_id,
        AssistantContentKind::Thinking,
        &message.thinking,
        message.thinking_revision,
        message.complete || !message.text.is_empty(),
        scans,
        active_keys,
        &mut blocks,
    );
    project_assistant_content(
        epoch,
        message_id,
        AssistantContentKind::Text,
        &message.text,
        message.text_revision,
        message.complete,
        scans,
        active_keys,
        &mut blocks,
    );
    let block_count = blocks.len();
    for (index, block) in blocks.iter_mut().enumerate() {
        block.trailing_blank |= index + 1 < block_count;
    }
    blocks
}

#[allow(clippy::too_many_arguments)]
fn project_assistant_content(
    epoch: u64,
    message_id: u64,
    content_kind: AssistantContentKind,
    source: &str,
    content_revision: u64,
    finished: bool,
    scans: &mut HashMap<(u64, u64, AssistantContentKind), markdown::IncrementalMarkdown>,
    active_keys: &mut HashSet<(u64, u64, AssistantContentKind)>,
    output: &mut Vec<TranscriptBlock>,
) {
    if source.is_empty() {
        return;
    }
    let key = (epoch, message_id, content_kind);
    active_keys.insert(key);
    let scanner = scans.entry(key).or_default();
    let previously_stable = scanner.stable_prefix_bytes();
    let scan = scanner.update(source, finished);
    for (segment_index, block) in scan.blocks.into_iter().enumerate() {
        if block.start >= block.end || !source.is_char_boundary(block.start) {
            continue;
        }
        let first_in_message = output.is_empty();
        let phase = if block.complete {
            if block.end <= previously_stable {
                AssistantSegmentPhase::Stable
            } else if finished {
                AssistantSegmentPhase::Sealed
            } else {
                AssistantSegmentPhase::Stable
            }
        } else {
            AssistantSegmentPhase::Streaming
        };
        let segment_revision = if block.complete {
            u64::try_from(block.end).unwrap_or(u64::MAX)
        } else {
            content_revision
        };
        let content = source[block.start..block.end].to_owned();
        let mut projected_message = crate::state::AssistantMessage {
            id: message_id,
            session_epoch: epoch,
            complete: block.complete,
            ..crate::state::AssistantMessage::default()
        };
        match content_kind {
            AssistantContentKind::Thinking => {
                projected_message.thinking = content;
                projected_message.thinking_revision = segment_revision;
            }
            AssistantContentKind::Text => {
                projected_message.text = content;
                projected_message.text_revision = segment_revision;
            }
        }
        let kind = match content_kind {
            AssistantContentKind::Thinking => "thinking",
            AssistantContentKind::Text => "text",
        };
        let id = format!("assistant:{epoch}:{message_id}:{kind}:segment:{segment_index}");
        output.push(TranscriptBlock {
            id,
            item: TranscriptItem::Assistant(projected_message),
            assistant_segment: Some(AssistantSegment {
                message_id,
                session_epoch: epoch,
                segment_index,
                first_in_message,
                content_kind,
                byte_start: block.start,
                byte_end: block.end,
                content_revision: segment_revision,
                phase,
            }),
            leading_blank: false,
            trailing_blank: block.complete
                && source
                    .get(block.end..)
                    .is_some_and(|tail| tail.starts_with('\n')),
            render_cache: Arc::new(Mutex::new(HashMap::new())),
        });
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

fn render_assistant_segment(
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
    fn stable_prefix_segments_commit_before_the_streaming_tail() {
        let mut state = state();
        state
            .transcript
            .push(TranscriptItem::Assistant(AssistantMessage {
                id: 7,
                text: "first paragraph\n\nsecond paragraph\n\nmutable".to_owned(),
                text_revision: 1,
                complete: false,
                ..AssistantMessage::default()
            }));
        let mut store = TranscriptStore::default();
        store.sync(&state);

        assert_eq!(store.order.len(), 3);
        assert_eq!(store.phase(&store.order[0]), Some(ComponentPhase::Stable));
        assert_eq!(store.phase(&store.order[1]), Some(ComponentPhase::Stable));
        assert_eq!(
            store.phase(&store.order[2]),
            Some(ComponentPhase::Streaming)
        );
        let stable_ids = store.order[..2].to_vec();
        let pending = store.pending_history(80, 1, 100);
        assert_eq!(
            pending
                .iter()
                .map(|block| block.component_id.clone())
                .collect::<Vec<_>>(),
            stable_ids
        );
        store.acknowledge_history(&pending);
        assert_eq!(store.committed_cursor(), 2);
        assert_eq!(store.active_components().count(), 1);

        let TranscriptItem::Assistant(message) = &mut state.transcript[0] else {
            unreachable!()
        };
        message.text.push_str(" tail");
        message.text_revision += 1;
        store.sync(&state);
        assert_eq!(&store.order[..2], stable_ids.as_slice());
        assert_eq!(store.committed_cursor(), 2);
    }

    #[test]
    fn fenced_code_and_tables_remain_streaming_until_structurally_complete() {
        for source in [
            "```rust\nfn main() {}",
            "| key | value |\n|---|---|\n| one | two |",
        ] {
            let mut state = state();
            state
                .transcript
                .push(TranscriptItem::Assistant(AssistantMessage {
                    id: 9,
                    text: source.to_owned(),
                    text_revision: 1,
                    complete: false,
                    ..AssistantMessage::default()
                }));
            let mut store = TranscriptStore::default();
            store.sync(&state);
            assert!(store.pending_history(80, 1, 100).is_empty());
            assert_eq!(
                store.phase(&store.order[0]),
                Some(ComponentPhase::Streaming)
            );
        }
    }

    #[test]
    fn history_batches_acknowledge_rows_before_the_whole_segment() {
        let mut state = state();
        let body = (0..64)
            .map(|index| format!("line {index}\n"))
            .collect::<String>();
        state
            .transcript
            .push(TranscriptItem::Assistant(AssistantMessage {
                id: 11,
                text: format!("```text\n{body}```"),
                text_revision: 1,
                complete: true,
                ..AssistantMessage::default()
            }));
        let mut store = TranscriptStore::default();
        store.sync(&state);

        let first = store.pending_history(24, 1, 7);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].row_offset, 0);
        assert_eq!(first[0].rows.len(), 7);
        assert!(first[0].total_rows > first[0].rows.len());
        store.acknowledge_history(&first);
        assert_eq!(store.committed_cursor(), 0);
        assert_eq!(store.committed_row_offset(), 7);

        let second = store.pending_history(24, 2, 7);
        assert_eq!(second[0].row_offset, 7);
    }

    #[test]
    fn history_offsets_are_usize_beyond_u16() {
        let id = "large".to_owned();
        let block = Arc::new(TranscriptBlock {
            id: id.clone(),
            item: TranscriptItem::Notice("large".to_owned()),
            assistant_segment: None,
            leading_blank: false,
            trailing_blank: false,
            render_cache: Arc::new(Mutex::new(HashMap::new())),
        });
        let mut store = TranscriptStore::default();
        store.order.push(id.clone());
        store.components.insert(id.clone(), block);
        store.phases.insert(id.clone(), ComponentPhase::Sealed);
        store.acknowledge_history(&[CommittedHistoryBlock {
            component_id: id,
            source_revision: 1,
            row_offset: 0,
            total_rows: 70_000,
            rows: vec![VisualRow::blank("large"); 65_536],
        }]);
        assert_eq!(store.committed_cursor(), 0);
        assert_eq!(store.committed_row_offset(), 65_536);
    }

    #[test]
    fn stable_render_rows_are_cached_per_width() {
        let mut state = state();
        state
            .transcript
            .push(TranscriptItem::Assistant(AssistantMessage {
                id: 12,
                text: "cached paragraph".to_owned(),
                text_revision: 1,
                complete: true,
                ..AssistantMessage::default()
            }));
        let mut store = TranscriptStore::default();
        store.sync(&state);
        let block = store.active_components().next().unwrap();
        block.render(40);
        block.render(40);
        assert_eq!(block.render_cache.lock().unwrap().len(), 1);
        block.render(20);
        assert_eq!(block.render_cache.lock().unwrap().len(), 2);
    }

    #[test]
    fn projection_reset_reflows_canonical_segments_at_the_new_width() {
        let mut state = state();
        state
            .transcript
            .push(TranscriptItem::Assistant(AssistantMessage {
                id: 13,
                text: "This canonical sentence is intentionally long enough to wrap differently."
                    .to_owned(),
                text_revision: 1,
                complete: true,
                ..AssistantMessage::default()
            }));
        let mut store = TranscriptStore::default();
        store.sync(&state);
        let ids = store.order.clone();
        let wide = store.pending_history(80, 1, 100);
        let wide_rows = wide.iter().map(|block| block.rows.len()).sum::<usize>();
        store.acknowledge_history(&wide);
        assert_eq!(store.committed_cursor(), store.order.len());

        store.reset_projection();
        let narrow = store.pending_history(20, 2, 100);
        let narrow_rows = narrow.iter().map(|block| block.rows.len()).sum::<usize>();
        assert_eq!(store.order, ids);
        assert!(narrow_rows > wide_rows);
        assert_eq!(store.committed_cursor(), 0);
    }

    #[test]
    fn canonical_resize_reflow_preserves_ids_and_separates_active_tail() {
        let mut state = state();
        state.transcript = vec![
            TranscriptItem::User(UserMessage {
                text: "A user message that wraps at narrow widths".to_owned(),
                status: UserMessageStatus::Accepted,
            }),
            TranscriptItem::Notice("canonical notice".to_owned()),
            TranscriptItem::Assistant(AssistantMessage {
                id: 14,
                text: "mutable streaming tail".to_owned(),
                text_revision: 1,
                complete: false,
                ..AssistantMessage::default()
            }),
        ];
        let mut store = TranscriptStore::default();
        store.sync(&state);
        let ids = store.order.clone();

        let wide = store.canonical_reflow_projection(80, 1, 0);
        let narrow = store.canonical_reflow_projection(20, 2, 0);
        let wide_rows = wide
            .history_blocks
            .iter()
            .map(|block| block.rows.len())
            .sum::<usize>();
        let narrow_rows = narrow
            .history_blocks
            .iter()
            .map(|block| block.rows.len())
            .sum::<usize>();

        assert_eq!(store.order, ids);
        assert_eq!(
            wide.history_blocks
                .iter()
                .map(|block| &block.component_id)
                .collect::<Vec<_>>(),
            narrow
                .history_blocks
                .iter()
                .map(|block| &block.component_id)
                .collect::<Vec<_>>()
        );
        assert!(narrow_rows > wide_rows);
        assert!(
            narrow
                .active_rows
                .iter()
                .any(|row| row.plain_text().contains("mutable"))
        );
    }

    #[test]
    fn resize_reflow_limit_keeps_recent_complete_components() {
        let mut state = state();
        state.transcript = vec![
            TranscriptItem::Notice("one".to_owned()),
            TranscriptItem::Notice("two".to_owned()),
            TranscriptItem::Notice("three".to_owned()),
        ];
        let mut store = TranscriptStore::default();
        store.sync(&state);

        let limited = store.canonical_reflow_projection(80, 1, 2);
        assert_eq!(limited.omitted_components, 1);
        assert_eq!(limited.history_blocks.len(), 2);
        assert_eq!(
            limited
                .history_blocks
                .iter()
                .map(|block| block.rows[0].plain_text())
                .collect::<Vec<_>>(),
            vec!["! Notice · two", "! Notice · three"]
        );

        let unlimited = store.canonical_reflow_projection(80, 2, 0);
        assert_eq!(unlimited.omitted_components, 0);
        assert_eq!(unlimited.history_blocks.len(), 3);
    }

    #[test]
    fn resize_reflow_never_splits_an_oversized_component() {
        let mut state = state();
        state.transcript.push(TranscriptItem::User(UserMessage {
            text: "word ".repeat(200),
            status: UserMessageStatus::Accepted,
        }));
        let mut store = TranscriptStore::default();
        store.sync(&state);

        let projection = store.canonical_reflow_projection(12, 1, 3);
        assert_eq!(projection.history_blocks.len(), 1);
        assert!(projection.history_blocks[0].rows.len() > 3);
        assert_eq!(
            projection.history_blocks[0].rows.len(),
            projection.history_blocks[0].total_rows
        );
        assert!(store.apply_reflow_projection(&projection));
        assert_eq!(store.committed_cursor(), store.order.len());
        assert_eq!(
            store.render_canonical_history(12).len(),
            projection.history_blocks[0].rows.len()
        );
    }

    #[test]
    fn streaming_resize_then_completion_matches_uninterrupted_rendering() {
        let initial = "Stable paragraph.\n\n```rust\nfn main() {";
        let completed = concat!(
            "Stable paragraph.\n\n",
            "```rust\nfn main() {}\n```\n\n",
            "| a | b |\n|---|---|\n| 1 | 2 |\n"
        );
        let mut resized_state = state();
        resized_state
            .transcript
            .push(TranscriptItem::Assistant(AssistantMessage {
                id: 44,
                text: initial.to_owned(),
                text_revision: 1,
                complete: false,
                ..AssistantMessage::default()
            }));
        let mut resized = TranscriptStore::default();
        resized.sync(&resized_state);
        let projection = resized.canonical_reflow_projection(24, 1, 0);
        assert!(resized.apply_reflow_projection(&projection));

        let TranscriptItem::Assistant(message) = resized_state.transcript.last_mut().unwrap()
        else {
            unreachable!()
        };
        message.text = completed.to_owned();
        message.text_revision = 2;
        message.complete = true;
        resized.sync(&resized_state);

        let mut uninterrupted = TranscriptStore::default();
        uninterrupted.sync(&resized_state);
        assert_eq!(
            resized
                .render_canonical_history(24)
                .into_iter()
                .map(|row| row.plain_text())
                .collect::<Vec<_>>(),
            uninterrupted
                .render_canonical_history(24)
                .into_iter()
                .map(|row| row.plain_text())
                .collect::<Vec<_>>()
        );
        let mut unique_ids = resized.order.clone();
        unique_ids.sort();
        unique_ids.dedup();
        assert_eq!(unique_ids.len(), resized.order.len());
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
            assistant_segment: None,
            leading_blank: false,
            trailing_blank: false,
            render_cache: Arc::new(Mutex::new(HashMap::new())),
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
                ..AssistantMessage::default()
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
        let first_tool = blocks
            .iter()
            .find(|block| block.id == "tool:tool-1")
            .expect("first tool");
        let second_tool = blocks
            .iter()
            .find(|block| block.id == "tool:tool-2")
            .expect("second tool");
        assert!(first_tool.render(48)[0].plain_text().is_empty());
        assert!(second_tool.render(48)[0].plain_text().is_empty());

        let assistant = blocks
            .iter()
            .take_while(|block| block.assistant_segment.is_some())
            .flat_map(|block| block.render(48))
            .collect::<Vec<_>>();
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
