use std::{
    collections::{HashMap, HashSet},
    io,
    sync::{Arc, Mutex},
};

use crate::state::{AppState, ToolStatus, TranscriptItem, UserMessageStatus};
use crate::ui::transcript::render::{render_assistant_segment, render_item};
use crate::ui::{
    markdown, palette,
    types::{
        AssistantContentKind, AssistantSegment, AssistantSegmentPhase, CanonicalReflowProjection,
        CommittedHistoryBlock, ComponentId, PrimaryTranscriptProjection, TranscriptSyncOutcome,
        VisualRow,
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
pub(crate) const RENDER_CACHE_MAX_ENTRIES: usize = 64;

#[derive(Debug, Clone)]
pub struct TranscriptBlock {
    pub id: ComponentId,
    pub item: TranscriptItem,
    pub assistant_segment: Option<AssistantSegment>,
    pub(crate) leading_blank: bool,
    pub(crate) trailing_blank: bool,
    pub(crate) render_cache: RenderCache,
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
        let mut cache = self
            .render_cache
            .lock()
            .expect("transcript render cache poisoned");
        if cache.len() >= RENDER_CACHE_MAX_ENTRIES {
            cache.clear();
        }
        cache.insert(cache_key, rows.clone());
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
    pub(crate) phases: HashMap<ComponentId, ComponentPhase>,
    scrollback_cursor: usize,
    scrollback_row_offset: usize,
    session_epoch: u64,
    assistant_scans: HashMap<(u64, u64, AssistantContentKind), markdown::IncrementalMarkdown>,
}

impl TranscriptStore {
    pub fn sync(&mut self, state: &AppState) -> TranscriptSyncOutcome {
        let epoch_changed = self.session_epoch != state.session_epoch;
        if epoch_changed {
            self.session_epoch = state.session_epoch;
            self.scrollback_cursor = 0;
            self.scrollback_row_offset = 0;
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
            .take(self.scrollback_cursor)
            .all(|(old_id, new_id)| {
                old_id == new_id
                    && self
                        .components
                        .get(old_id)
                        .zip(components.get(new_id))
                        .is_some_and(|(old, new)| old.as_ref() == new.as_ref())
            });
        let partial_component_unchanged = self.scrollback_row_offset == 0
            || self
                .order
                .get(self.scrollback_cursor)
                .zip(order.get(self.scrollback_cursor))
                .is_some_and(|(old_id, new_id)| {
                    old_id == new_id
                        && self
                            .components
                            .get(old_id)
                            .zip(components.get(new_id))
                            .is_some_and(|(old, new)| old.as_ref() == new.as_ref())
                });
        let projection_invalidated = epoch_changed
            || !prefix_unchanged
            || !partial_component_unchanged
            || order.len() < self.scrollback_cursor;
        if projection_invalidated {
            self.scrollback_cursor = 0;
            self.scrollback_row_offset = 0;
            self.phases.clear();
            changed = true;
        }

        if changed {
            self.order = order;
            self.components = components;
            self.revision = self.revision.saturating_add(1);
            self.refresh_phases();
        }
        if projection_invalidated {
            TranscriptSyncOutcome::ProjectionInvalidated
        } else if changed {
            TranscriptSyncOutcome::AppendOnly
        } else {
            TranscriptSyncOutcome::Unchanged
        }
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

    pub fn scrollback_cursor(&self) -> usize {
        self.scrollback_cursor
    }

    pub fn scrollback_row_offset(&self) -> usize {
        self.scrollback_row_offset
    }

    pub fn uncommitted_components(&self) -> impl Iterator<Item = &Arc<TranscriptBlock>> {
        self.order[self.scrollback_cursor.min(self.order.len())..]
            .iter()
            .filter_map(|id| self.components.get(id))
    }

    pub fn project_primary(
        &self,
        width: u16,
        resident_capacity: usize,
        source_revision: u64,
        maximum_rows: usize,
        maximum_bytes: usize,
        animation_frame: u8,
    ) -> PrimaryTranscriptProjection {
        struct ProjectedRow {
            component_id: ComponentId,
            row_offset: usize,
            total_rows: usize,
            phase: ComponentPhase,
            row: VisualRow,
        }

        let width = width.max(1);
        let mut projected = Vec::new();
        for (component_index, id) in self.order.iter().enumerate().skip(self.scrollback_cursor) {
            let Some(component) = self.components.get(id) else {
                continue;
            };
            let phase = self.phase(id).unwrap_or_else(|| component.phase());
            let rows = component.render_animated(width, animation_frame);
            let start = if component_index == self.scrollback_cursor {
                self.scrollback_row_offset.min(rows.len())
            } else {
                0
            };
            let total_rows = rows.len();
            projected.extend(
                rows.into_iter()
                    .enumerate()
                    .skip(start)
                    .map(|(row_offset, row)| ProjectedRow {
                        component_id: id.clone(),
                        row_offset,
                        total_rows,
                        phase,
                        row,
                    }),
            );
        }

        let overflow_needed = projected.len().saturating_sub(resident_capacity);
        let eligible_rows = projected
            .iter()
            .take_while(|row| matches!(row.phase, ComponentPhase::Stable | ComponentPhase::Sealed))
            .count();
        let mut selected_rows = 0usize;
        let mut selected_bytes = 0usize;
        for row in projected
            .iter()
            .take(overflow_needed.min(eligible_rows).min(maximum_rows))
        {
            let row_bytes = row
                .row
                .cells
                .iter()
                .map(|cell| cell.symbol.len())
                .sum::<usize>();
            if selected_rows > 0 && selected_bytes.saturating_add(row_bytes) > maximum_bytes {
                break;
            }
            if selected_rows == 0 && row_bytes > maximum_bytes {
                if maximum_bytes == 0 {
                    break;
                }
                selected_rows = 1;
                break;
            }
            selected_rows += 1;
            selected_bytes = selected_bytes.saturating_add(row_bytes);
        }

        let mut overflow_blocks = Vec::<CommittedHistoryBlock>::new();
        for entry in projected.iter().take(selected_rows) {
            if let Some(block) = overflow_blocks.last_mut()
                && block.component_id == entry.component_id
                && block.row_offset.saturating_add(block.rows.len()) == entry.row_offset
            {
                block.rows.push(entry.row.clone());
            } else {
                overflow_blocks.push(CommittedHistoryBlock {
                    component_id: entry.component_id.clone(),
                    source_revision,
                    row_offset: entry.row_offset,
                    total_rows: entry.total_rows,
                    rows: vec![entry.row.clone()],
                });
            }
        }

        let remaining = &projected[selected_rows..];
        let resident_start = remaining.len().saturating_sub(resident_capacity);
        let resident_rows = remaining[resident_start..]
            .iter()
            .map(|entry| entry.row.clone())
            .collect::<Vec<_>>();
        let bootstrap_padding_rows = resident_capacity.saturating_sub(resident_rows.len());

        PrimaryTranscriptProjection {
            overflow_blocks,
            resident_rows,
            bootstrap_padding_rows,
            resident_capacity,
            scrollback_cursor: self.scrollback_cursor,
            scrollback_row_offset: self.scrollback_row_offset,
            canonical_revision: self.revision,
        }
    }

    /// Advances history only after a successful terminal commit.
    pub fn acknowledge_overflow(&mut self, blocks: &[CommittedHistoryBlock]) {
        for block in blocks {
            let expected = self.order.get(self.scrollback_cursor);
            if expected != Some(&block.component_id) {
                break;
            }
            if block.row_offset != self.scrollback_row_offset || block.rows.is_empty() {
                break;
            }
            let acknowledged = block.row_offset.saturating_add(block.rows.len());
            if acknowledged < block.total_rows {
                self.scrollback_row_offset = acknowledged;
                break;
            }
            if acknowledged == block.total_rows {
                self.phases
                    .insert(block.component_id.clone(), ComponentPhase::Committed);
                self.scrollback_cursor += 1;
                self.scrollback_row_offset = 0;
            } else {
                break;
            }
        }
    }

    pub fn reset_projection(&mut self) {
        self.scrollback_cursor = 0;
        self.scrollback_row_offset = 0;
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
        resident_capacity: usize,
        source_revision: u64,
        _maximum_rows: usize,
    ) -> CanonicalReflowProjection {
        let width = width.max(1);
        let mut canonical = self.clone();
        canonical.reset_projection();
        let primary = canonical.project_primary(
            width,
            resident_capacity,
            source_revision,
            usize::MAX,
            usize::MAX,
            0,
        );
        let mut scrollback_cursor = 0usize;
        let mut scrollback_row_offset = 0usize;
        for block in &primary.overflow_blocks {
            let Some(id) = canonical.order.get(scrollback_cursor) else {
                break;
            };
            if id != &block.component_id || block.row_offset != scrollback_row_offset {
                break;
            }
            let acknowledged = block.row_offset.saturating_add(block.rows.len());
            if acknowledged == block.total_rows {
                scrollback_cursor = scrollback_cursor.saturating_add(1);
                scrollback_row_offset = 0;
            } else {
                scrollback_row_offset = acknowledged;
            }
        }
        CanonicalReflowProjection {
            canonical_revision: self.revision,
            session_epoch: self.session_epoch,
            source_revision,
            width,
            scrollback_cursor,
            scrollback_row_offset,
            history_blocks: primary.overflow_blocks,
            resident_rows: primary.resident_rows,
            bootstrap_padding_rows: primary.bootstrap_padding_rows,
            resident_capacity,
        }
    }

    pub fn apply_reflow_projection(&mut self, projection: &CanonicalReflowProjection) -> bool {
        if !self.reflow_projection_is_compatible(projection) {
            return false;
        }

        self.scrollback_cursor = projection.scrollback_cursor;
        self.scrollback_row_offset = projection.scrollback_row_offset;
        self.phases.clear();
        self.refresh_phases();
        for id in &self.order[..self.scrollback_cursor.min(self.order.len())] {
            self.phases.insert(id.clone(), ComponentPhase::Committed);
        }
        true
    }

    pub fn reflow_projection_is_compatible(&self, projection: &CanonicalReflowProjection) -> bool {
        if projection.session_epoch != self.session_epoch
            || projection.canonical_revision != self.revision
            || projection.scrollback_cursor > self.order.len()
        {
            return false;
        }
        if projection.history_blocks.iter().any(|block| {
            self.components
                .get(&block.component_id)
                .is_none_or(|component| {
                    let rows = component.render(projection.width);
                    rows.len() != block.total_rows
                        || rows.get(
                            block.row_offset..block.row_offset.saturating_add(block.rows.len()),
                        ) != Some(block.rows.as_slice())
                })
        }) {
            return false;
        }
        true
    }

    pub fn canonical_reflow_batches(
        projection: &CanonicalReflowProjection,
        maximum_rows: usize,
        maximum_bytes: usize,
    ) -> Vec<Vec<CommittedHistoryBlock>> {
        let maximum_rows = maximum_rows.max(1);
        let maximum_bytes = maximum_bytes.max(1);
        let mut batches = Vec::<Vec<CommittedHistoryBlock>>::new();
        let mut current = Vec::<CommittedHistoryBlock>::new();
        let mut current_rows = 0usize;
        let mut current_bytes = 0usize;

        for block in &projection.history_blocks {
            let mut row_offset = 0usize;
            while row_offset < block.rows.len() {
                let row = &block.rows[row_offset];
                let row_bytes = row
                    .cells
                    .iter()
                    .map(|cell| cell.symbol.len())
                    .sum::<usize>();
                if !current.is_empty()
                    && (current_rows >= maximum_rows
                        || current_bytes.saturating_add(row_bytes) > maximum_bytes)
                {
                    batches.push(std::mem::take(&mut current));
                    current_rows = 0;
                    current_bytes = 0;
                }

                let physical_offset = row_offset;
                let mut selected = Vec::new();
                while row_offset < block.rows.len() && current_rows < maximum_rows {
                    let candidate = &block.rows[row_offset];
                    let candidate_bytes = candidate
                        .cells
                        .iter()
                        .map(|cell| cell.symbol.len())
                        .sum::<usize>();
                    if !selected.is_empty()
                        && current_bytes.saturating_add(candidate_bytes) > maximum_bytes
                    {
                        break;
                    }
                    selected.push(candidate.clone());
                    row_offset += 1;
                    current_rows += 1;
                    current_bytes = current_bytes.saturating_add(candidate_bytes);
                    if current_bytes >= maximum_bytes {
                        break;
                    }
                }
                current.push(CommittedHistoryBlock {
                    component_id: block.component_id.clone(),
                    source_revision: block.source_revision,
                    row_offset: physical_offset,
                    total_rows: block.total_rows,
                    rows: selected,
                });
                if current_rows >= maximum_rows || current_bytes >= maximum_bytes {
                    batches.push(std::mem::take(&mut current));
                    current_rows = 0;
                    current_bytes = 0;
                }
            }
        }
        if !current.is_empty() {
            batches.push(current);
        }
        batches
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
