use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use crate::state::{AppState, TranscriptItem};
use crate::ui::{
    markdown,
    types::{
        AssistantContentKind, AssistantSegment, AssistantSegmentPhase, ComponentId,
        TranscriptSyncOutcome,
    },
};

pub(crate) mod cache;
pub(crate) mod history;
pub(crate) mod model;
pub(crate) mod projection;

pub use model::{ComponentPhase, HistorySink, TranscriptBlock, TranscriptComponent};

#[derive(Debug, Clone, Default)]
pub struct TranscriptStore {
    pub order: Vec<ComponentId>,
    pub components: HashMap<ComponentId, Arc<TranscriptBlock>>,
    pub revision: u64,
    pub(crate) phases: HashMap<ComponentId, ComponentPhase>,
    pub(crate) scrollback_cursor: usize,
    pub(crate) scrollback_row_offset: usize,
    pub(crate) session_epoch: u64,
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
    pub(crate) fn refresh_phases(&mut self) {
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
