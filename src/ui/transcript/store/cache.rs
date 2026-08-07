use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use crate::ui::types::VisualRow;

use super::TranscriptStore;

pub(crate) type RenderCacheKey = (u16, u64, u64);
pub(crate) type RenderCache = Arc<Mutex<HashMap<RenderCacheKey, Vec<VisualRow>>>>;
pub(crate) const RENDER_CACHE_MAX_ENTRIES: usize = 64;

impl TranscriptStore {
    pub fn invalidate_render_caches(&self) {
        for component in self.components.values() {
            component
                .render_cache
                .lock()
                .expect("transcript render cache poisoned")
                .clear();
        }
    }
}
