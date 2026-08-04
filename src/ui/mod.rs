//! Full-height terminal UI infrastructure.
//!
//! The domain [`crate::app::App`] remains responsible for coding-agent
//! behaviour.  This module owns the presentation pipeline:
//!
//! `UiStore -> Scene -> VisualFrame -> TerminalCommitPlan -> TerminalDriver`.

pub mod input;
pub mod layout;
pub mod markdown;
pub mod palette;
pub mod panel;
pub mod scene;
pub mod selector;
pub mod shell;
pub mod store;
pub mod surface;
pub mod terminal;
pub mod test_support;
pub mod text;
pub mod tool;
pub mod transcript;
pub mod types;

pub use input::{InputRouter, RoutedInput};
pub use layout::{LayoutEngine, LayoutRequest};
pub use panel::PanelRequest;
pub use scene::{SceneBuilder, animation_active};
pub use selector::{
    SelectionNavigation, SelectorModel, SelectorPolicy, VirtualList, selection_navigation,
};
pub use store::{Invalidation, ReduceResult, UiEvent, UiState, UiStore};
pub use surface::SurfaceManager;
pub use terminal::{FrameCoordinator, TerminalCapabilities, TerminalDriver};
pub use tool::ToolSnapshotStore;
pub use transcript::{ComponentPhase, TranscriptStore};
pub use types::*;
