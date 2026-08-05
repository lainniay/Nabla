use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use crate::state::AppState;

use super::{
    transcript::TranscriptStore,
    types::{ComponentId, SurfaceKind, TerminalSize},
};

pub type OverlayId = u64;
pub type ModalId = u64;
pub type InputSessionId = u64;
pub type RequestId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusTarget {
    Prompt,
    Overlay(OverlayId),
    Transcript,
    Modal(ModalId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayPlacement {
    AboveComposer,
    Centered,
    FullHeight,
    Anchored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeightPolicy {
    Content {
        min: u16,
        max: u16,
    },
    Fixed(u16),
    Fraction {
        numerator: u16,
        denominator: u16,
        min: u16,
        max: u16,
    },
    Available,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayInstance {
    pub id: OverlayId,
    pub placement: OverlayPlacement,
    pub height: HeightPolicy,
    pub anchor: Option<ComponentId>,
    pub title: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OverlayStack {
    pub entries: Vec<OverlayInstance>,
    pub focused: Option<OverlayId>,
}

impl OverlayStack {
    pub fn open(&mut self, overlay: OverlayInstance) {
        self.entries.retain(|entry| entry.id != overlay.id);
        self.focused = Some(overlay.id);
        self.entries.push(overlay);
    }

    pub fn close(&mut self, id: OverlayId) {
        self.entries.retain(|entry| entry.id != id);
        self.focused = self.entries.last().map(|entry| entry.id);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputRole {
    Prompt,
    Search,
    InlineCompletion,
    Secret,
    ConfirmationReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitPolicy {
    Enter,
    ModifiedEnter,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryPolicy {
    Record,
    Ephemeral,
    Secret,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionPolicy {
    None,
    Command,
    File,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditorCore {
    pub text: String,
    pub cursor: usize,
    pub selection: Option<(usize, usize)>,
    undo: Vec<String>,
}

impl EditorCore {
    pub fn replace(&mut self, text: String) {
        self.undo.push(std::mem::replace(&mut self.text, text));
        self.cursor =
            unicode_segmentation::UnicodeSegmentation::graphemes(self.text.as_str(), true).count();
        self.selection = None;
    }

    pub fn undo(&mut self) {
        if let Some(previous) = self.undo.pop() {
            self.text = previous;
            self.cursor =
                unicode_segmentation::UnicodeSegmentation::graphemes(self.text.as_str(), true)
                    .count();
            self.selection = None;
        }
    }

    fn zeroize(&mut self) {
        let mut text = std::mem::take(&mut self.text).into_bytes();
        text.fill(0);
        for value in &mut self.undo {
            let mut bytes = std::mem::take(value).into_bytes();
            bytes.fill(0);
        }
        self.undo.clear();
        self.cursor = 0;
        self.selection = None;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputSession {
    pub id: InputSessionId,
    pub role: InputRole,
    pub editor: EditorCore,
    pub submit_policy: SubmitPolicy,
    pub history_policy: HistoryPolicy,
    pub completion_policy: CompletionPolicy,
}

impl InputSession {
    pub fn close(mut self) {
        if self.role == InputRole::Secret || self.history_policy == HistoryPolicy::Secret {
            self.editor.zeroize();
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InputSessions {
    pub active: Option<InputSessionId>,
    pub sessions: HashMap<InputSessionId, InputSession>,
}

impl InputSessions {
    pub fn insert(&mut self, session: InputSession) {
        self.active = Some(session.id);
        self.sessions.insert(session.id, session);
    }

    pub fn close(&mut self, id: InputSessionId) {
        if let Some(session) = self.sessions.remove(&id) {
            session.close();
        }
        if self.active == Some(id) {
            self.active = None;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalUiState {
    pub size: TerminalSize,
    pub surface: SurfaceKind,
    pub terminal_invalid: bool,
    pub projection_reflow_pending: bool,
}

impl Default for TerminalUiState {
    fn default() -> Self {
        Self {
            size: TerminalSize::new(1, 1),
            surface: SurfaceKind::Primary,
            terminal_invalid: true,
            projection_reflow_pending: false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionUiState {
    pub transcript_scroll_from_bottom: usize,
    pub follow_tail: bool,
}

#[derive(Debug, Clone)]
pub struct UiState {
    pub revision: u64,
    pub animation_frame: u8,
    pub transcript: TranscriptStore,
    pub overlays: OverlayStack,
    pub focus: FocusTarget,
    pub inputs: InputSessions,
    pub session_ui: SessionUiState,
    pub terminal: TerminalUiState,
    last_tick: Option<Instant>,
}

impl UiState {
    pub fn new(size: TerminalSize) -> Self {
        Self {
            revision: 1,
            animation_frame: 0,
            transcript: TranscriptStore::default(),
            overlays: OverlayStack::default(),
            focus: FocusTarget::Prompt,
            inputs: InputSessions::default(),
            session_ui: SessionUiState {
                transcript_scroll_from_bottom: 0,
                follow_tail: true,
            },
            terminal: TerminalUiState {
                size,
                ..TerminalUiState::default()
            },
            last_tick: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum UiEvent {
    DomainChanged,
    Resize(TerminalSize),
    ProjectionInvalidated,
    ProjectionRebuilt,
    Tick {
        now: Instant,
        animate: bool,
    },
    OpenOverlay(OverlayInstance),
    CloseOverlay(OverlayId),
    Focus(FocusTarget),
    EnterAlternate,
    LeaveAlternate,
    TerminalFailed,
    TerminalRecovered,
    AsyncResult {
        request_id: RequestId,
        entity_revision: u64,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Invalidation {
    pub scene: bool,
    pub layout: bool,
    pub terminal: bool,
    pub full_redraw: bool,
}

impl Invalidation {
    pub const fn all() -> Self {
        Self {
            scene: true,
            layout: true,
            terminal: true,
            full_redraw: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReduceResult {
    pub changed: bool,
    pub invalidation: Invalidation,
}

#[derive(Debug, Clone)]
pub struct UiStore {
    state: UiState,
}

impl UiStore {
    pub fn new(size: TerminalSize) -> Self {
        Self {
            state: UiState::new(size),
        }
    }

    pub fn state(&self) -> &UiState {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut UiState {
        &mut self.state
    }

    pub fn synchronize(&mut self, domain: &AppState) -> ReduceResult {
        if self.state.transcript.sync(domain) {
            self.bump();
            ReduceResult {
                changed: true,
                invalidation: Invalidation {
                    scene: true,
                    layout: true,
                    terminal: true,
                    full_redraw: false,
                },
            }
        } else {
            ReduceResult {
                changed: false,
                invalidation: Invalidation::default(),
            }
        }
    }

    pub fn reduce(&mut self, event: UiEvent) -> ReduceResult {
        let mut invalidation = Invalidation::default();
        let changed = match event {
            UiEvent::DomainChanged => true,
            UiEvent::Resize(size) if size != self.state.terminal.size => {
                let width_changed = size.width != self.state.terminal.size.width;
                self.state.terminal.size = size;
                self.state.terminal.terminal_invalid = true;
                if width_changed {
                    self.state.terminal.projection_reflow_pending = true;
                }
                invalidation = Invalidation::all();
                true
            }
            UiEvent::Resize(_) => false,
            UiEvent::ProjectionInvalidated => {
                self.state.terminal.terminal_invalid = true;
                self.state.terminal.projection_reflow_pending = true;
                invalidation = Invalidation::all();
                true
            }
            UiEvent::ProjectionRebuilt => {
                let changed = self.state.terminal.projection_reflow_pending
                    || self.state.terminal.terminal_invalid;
                self.state.terminal.projection_reflow_pending = false;
                self.state.terminal.terminal_invalid = false;
                changed
            }
            UiEvent::Tick { now, animate } => {
                let changed = animate
                    && self
                        .state
                        .last_tick
                        .is_none_or(|last| now.duration_since(last) >= Duration::from_millis(80));
                self.state.last_tick = Some(now);
                if changed {
                    self.state.animation_frame = self.state.animation_frame.wrapping_add(1);
                    invalidation.scene = true;
                    invalidation.terminal = true;
                }
                changed
            }
            UiEvent::OpenOverlay(overlay) => {
                let id = overlay.id;
                self.state.overlays.open(overlay);
                self.state.focus = FocusTarget::Overlay(id);
                invalidation = Invalidation::all();
                true
            }
            UiEvent::CloseOverlay(id) => {
                let existed = self
                    .state
                    .overlays
                    .entries
                    .iter()
                    .any(|entry| entry.id == id);
                self.state.overlays.close(id);
                self.state.focus = self
                    .state
                    .overlays
                    .focused
                    .map_or(FocusTarget::Prompt, FocusTarget::Overlay);
                invalidation = Invalidation::all();
                existed
            }
            UiEvent::Focus(focus) if focus != self.state.focus => {
                self.state.focus = focus;
                invalidation.scene = true;
                invalidation.terminal = true;
                true
            }
            UiEvent::Focus(_) => false,
            UiEvent::EnterAlternate if self.state.terminal.surface != SurfaceKind::Alternate => {
                self.state.terminal.surface = SurfaceKind::Alternate;
                invalidation = Invalidation::all();
                true
            }
            UiEvent::LeaveAlternate if self.state.terminal.surface != SurfaceKind::Primary => {
                self.state.terminal.surface = SurfaceKind::Primary;
                invalidation = Invalidation::all();
                true
            }
            UiEvent::EnterAlternate | UiEvent::LeaveAlternate => false,
            UiEvent::TerminalFailed => {
                self.state.terminal.terminal_invalid = true;
                invalidation.full_redraw = true;
                true
            }
            UiEvent::TerminalRecovered => {
                let changed = self.state.terminal.terminal_invalid;
                self.state.terminal.terminal_invalid = false;
                changed
            }
            UiEvent::AsyncResult {
                entity_revision, ..
            } => entity_revision >= self.state.revision,
        };

        if changed {
            self.bump();
            invalidation.scene = true;
            invalidation.terminal = true;
        }
        ReduceResult {
            changed,
            invalidation,
        }
    }

    fn bump(&mut self) {
        self.state.revision = self.state.revision.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_derives_the_full_viewport_and_invalidates_old_geometry() {
        let mut store = UiStore::new(TerminalSize::new(120, 40));
        let result = store.reduce(UiEvent::Resize(TerminalSize::new(80, 24)));
        assert!(result.changed);
        assert!(result.invalidation.full_redraw);
        assert_eq!(store.state().terminal.size, TerminalSize::new(80, 24));
    }

    #[test]
    fn prompt_search_and_secret_sessions_never_share_buffers() {
        let mut sessions = InputSessions::default();
        for (id, role, text) in [
            (1, InputRole::Prompt, "prompt"),
            (2, InputRole::Search, "query"),
            (3, InputRole::Secret, "token"),
        ] {
            sessions.insert(InputSession {
                id,
                role,
                editor: EditorCore {
                    text: text.to_owned(),
                    ..EditorCore::default()
                },
                submit_policy: SubmitPolicy::Enter,
                history_policy: if role == InputRole::Secret {
                    HistoryPolicy::Secret
                } else {
                    HistoryPolicy::Ephemeral
                },
                completion_policy: CompletionPolicy::None,
            });
        }
        sessions
            .sessions
            .get_mut(&1)
            .unwrap()
            .editor
            .replace("changed".to_owned());
        assert_eq!(sessions.sessions[&2].editor.text, "query");
        assert_eq!(sessions.sessions[&3].editor.text, "token");
        sessions.close(3);
        assert!(!sessions.sessions.contains_key(&3));
    }

    #[test]
    fn stale_async_results_do_not_advance_revision() {
        let mut store = UiStore::new(TerminalSize::new(80, 24));
        let revision = store.state().revision;
        let result = store.reduce(UiEvent::AsyncResult {
            request_id: 7,
            entity_revision: revision.saturating_sub(1),
        });
        assert!(!result.changed);
        assert_eq!(store.state().revision, revision);
    }

    #[test]
    fn ticks_only_advance_transient_animation_when_requested() {
        let mut store = UiStore::new(TerminalSize::new(80, 24));
        let now = Instant::now();
        let idle = store.reduce(UiEvent::Tick {
            now,
            animate: false,
        });
        assert!(!idle.changed);
        assert_eq!(store.state().animation_frame, 0);

        let animated = store.reduce(UiEvent::Tick {
            now: now + Duration::from_millis(100),
            animate: true,
        });
        assert!(animated.changed);
        assert_eq!(store.state().animation_frame, 1);
        assert!(animated.invalidation.terminal);
    }
}
