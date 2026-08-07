use crate::ui::types::{CommittedHistoryBlock, VisualRow};

use super::{
    TranscriptStore,
    model::{ComponentPhase, TranscriptComponent},
};

impl TranscriptStore {
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
    pub fn render_canonical_history(&self, width: u16) -> Vec<VisualRow> {
        self.order
            .iter()
            .filter_map(|id| self.components.get(id))
            .flat_map(|component| component.render(width.max(1)))
            .collect()
    }
}
