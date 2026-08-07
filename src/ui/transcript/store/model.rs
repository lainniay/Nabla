use std::io;

use crate::state::{ToolStatus, TranscriptItem, UserMessageStatus};
use crate::ui::transcript::render::{render_assistant_segment, render_item};
use crate::ui::{
    palette,
    types::{
        AssistantSegment, AssistantSegmentPhase, CommittedHistoryBlock, ComponentId, VisualRow,
    },
};

use super::cache::{RENDER_CACHE_MAX_ENTRIES, RenderCache};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentPhase {
    Streaming,
    Stable,
    Sealed,
    Committed,
}
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
