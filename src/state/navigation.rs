use super::*;

pub const THINKING_LEVELS: [&str; 7] = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionPanelKind {
    Model,
    Thinking,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionPanelAction {
    SetModel { provider: String, model_id: String },
    SetThinking(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionPanelOption {
    pub label: String,
    pub description: String,
    pub action: SelectionPanelAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionPanelState {
    pub kind: SelectionPanelKind,
    pub title: String,
    pub options: Vec<SelectionPanelOption>,
    pub selected: usize,
    pub loading: bool,
}

impl SelectionPanelState {
    pub fn loading_models() -> Self {
        Self {
            kind: SelectionPanelKind::Model,
            title: "Select model".to_owned(),
            options: Vec::new(),
            selected: 0,
            loading: true,
        }
    }

    pub fn models(options: Vec<SelectionPanelOption>, selected: usize) -> Self {
        Self {
            kind: SelectionPanelKind::Model,
            title: "Select model".to_owned(),
            selected: selected.min(options.len().saturating_sub(1)),
            options,
            loading: false,
        }
    }

    pub fn thinking(current: &str) -> Self {
        let options = THINKING_LEVELS
            .iter()
            .map(|level| SelectionPanelOption {
                label: (*level).to_owned(),
                description: if *level == current {
                    "Current".to_owned()
                } else {
                    String::new()
                },
                action: SelectionPanelAction::SetThinking((*level).to_owned()),
            })
            .collect::<Vec<_>>();
        let selected = THINKING_LEVELS
            .iter()
            .position(|level| *level == current)
            .unwrap_or(0);
        Self {
            kind: SelectionPanelKind::Thinking,
            title: "Select thinking level".to_owned(),
            options,
            selected,
            loading: false,
        }
    }

    pub fn selected_action(&self) -> Option<&SelectionPanelAction> {
        self.options.get(self.selected).map(|option| &option.action)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionBrowserState {
    pub browser_id: Option<String>,
    pub current_cwd: String,
    pub scope: SessionScope,
    pub sort_mode: SessionSortMode,
    pub named_only: bool,
    pub query: EditorState,
    pub search_active: bool,
    pub sessions: Vec<SessionSummary>,
    pub total: usize,
    pub next_offset: Option<usize>,
    pub truncated: bool,
    pub selected: usize,
    pub loading: bool,
    pub loaded: Option<(u64, u64)>,
    pub generation: u64,
    pub switching: bool,
    pub confirm_missing_cwd: Option<SessionSummary>,
}

impl SessionBrowserState {
    pub fn loading() -> Self {
        Self {
            browser_id: None,
            current_cwd: String::new(),
            scope: SessionScope::Current,
            sort_mode: SessionSortMode::Threaded,
            named_only: false,
            query: EditorState::default(),
            search_active: false,
            sessions: Vec::new(),
            total: 0,
            next_offset: None,
            truncated: false,
            selected: 0,
            loading: true,
            loaded: None,
            generation: 0,
            switching: false,
            confirm_missing_cwd: None,
        }
    }

    pub fn selected_session(&self) -> Option<&SessionSummary> {
        self.sessions.get(self.selected)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TreePhase {
    Browse,
    EditLabel {
        entry_id: String,
        editor: EditorState,
    },
    ChooseSummary {
        entry_id: String,
        selected: usize,
    },
    CustomSummary {
        entry_id: String,
        editor: EditorState,
    },
    Navigating {
        entry_id: String,
        summarizing: bool,
        aborting: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct TreeBrowserState {
    pub items: Vec<TreeItem>,
    pub leaf_id: Option<String>,
    pub selected: usize,
    pub selected_entry_id: Option<String>,
    pub filter_mode: TreeFilterMode,
    pub query: EditorState,
    pub search_active: bool,
    pub folded_entry_ids: HashSet<String>,
    pub phase: TreePhase,
    pub loading: bool,
    pub generation: u64,
}

impl TreeBrowserState {
    pub fn loading() -> Self {
        Self {
            items: Vec::new(),
            leaf_id: None,
            selected: 0,
            selected_entry_id: None,
            filter_mode: TreeFilterMode::Default,
            query: EditorState::default(),
            search_active: false,
            folded_entry_ids: HashSet::new(),
            phase: TreePhase::Browse,
            loading: true,
            generation: 0,
        }
    }

    pub fn selected_item(&self) -> Option<&TreeItem> {
        self.items.get(self.selected)
    }
}
