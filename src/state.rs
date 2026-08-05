use crate::{
    command::{CommandCatalog, CommandSpec, DiscoveredCommand},
    file_references::FileCompletionState,
    host::ApprovalDecision,
    rpc::PiState,
    selection::{next_wrapped, previous_wrapped},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

// INFO: State is grouped by domain so protocol data, workflows, and editor logic
// can evolve independently while callers keep using the stable state facade.
mod agents;
mod app_state;
mod auth;
mod context;
mod conversation;
mod goals;
mod navigation;
mod planning;
mod resources;
mod sessions;
mod tool_diff;
mod transcript;

pub use agents::*;
pub use app_state::*;
pub use auth::*;
pub use context::*;
pub use conversation::*;
pub use goals::*;
pub use navigation::*;
pub use planning::*;
pub use resources::*;
pub use sessions::*;
pub use tool_diff::*;
pub use transcript::*;

#[cfg(test)]
mod tests;
