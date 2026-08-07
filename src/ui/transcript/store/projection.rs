use crate::ui::types::{
    CanonicalReflowProjection, CommittedHistoryBlock, ComponentId, PrimaryTranscriptProjection,
    VisualRow,
};

use super::{
    TranscriptStore,
    model::{ComponentPhase, TranscriptComponent},
};

impl TranscriptStore {
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

    pub fn reset_projection(&mut self) {
        self.scrollback_cursor = 0;
        self.scrollback_row_offset = 0;
        self.phases.clear();
        self.refresh_phases();
    }
    pub fn rebuild_projection(&mut self, width: u16) -> Vec<VisualRow> {
        self.reset_projection();
        self.render_canonical_history(width)
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
