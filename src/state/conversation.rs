use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Idle,
    PreparingReferences,
    Submitting,
    Running,
    Aborting,
    Compacting,
    Authenticating,
    SwitchingSession,
    NavigatingTree,
    SummarizingBranch,
    AuthRequired,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiModalKind {
    SessionBrowser,
    TreeBrowser,
    Selection,
    AgentPicker,
    Transcript,
    Question,
    Auth,
    Approval,
    Permissions,
    Integration,
    PlanReview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TranscriptViewMode {
    #[default]
    Normal,
    Verbose,
    Summary,
}

impl TranscriptViewMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Verbose => "Verbose",
            Self::Summary => "Summary",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptViewerState {
    pub mode: TranscriptViewMode,
    pub selected_item: Option<usize>,
    pub scroll_from_bottom: usize,
    pub follow_tail: bool,
    pub scroll_to_selected: bool,
    pub tool_expansion_overrides: HashMap<String, bool>,
    pub search_query: EditorState,
    pub search_active: bool,
    pub search_matches: Vec<usize>,
    pub current_match: Option<usize>,
    pub opened_item_count: usize,
}

impl TranscriptViewerState {
    pub fn new(mode: TranscriptViewMode, transcript: &[TranscriptItem]) -> Self {
        let selected_item = transcript
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, item)| matches!(item, TranscriptItem::Tool(_)).then_some(index));
        Self {
            mode,
            selected_item,
            scroll_from_bottom: 0,
            follow_tail: selected_item.is_none(),
            scroll_to_selected: selected_item.is_some(),
            tool_expansion_overrides: HashMap::new(),
            search_query: EditorState::default(),
            search_active: false,
            search_matches: Vec::new(),
            current_match: None,
            opened_item_count: transcript.len(),
        }
    }

    pub fn unseen_items(&self, transcript_len: usize) -> usize {
        if self.follow_tail {
            0
        } else {
            transcript_len.saturating_sub(self.opened_item_count)
        }
    }
}

impl RunState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::PreparingReferences => "preparing references",
            Self::Submitting => "submitting",
            Self::Running => "running",
            Self::Aborting => "aborting",
            Self::Compacting => "compacting",
            Self::Authenticating => "authenticating",
            Self::SwitchingSession => "switching session",
            Self::NavigatingTree => "navigating tree",
            Self::SummarizingBranch => "summarizing branch",
            Self::AuthRequired => "login required",
            Self::Error => "error",
        }
    }

    pub fn is_busy(self) -> bool {
        matches!(
            self,
            Self::PreparingReferences
                | Self::Submitting
                | Self::Running
                | Self::Aborting
                | Self::Compacting
                | Self::Authenticating
                | Self::SwitchingSession
                | Self::NavigatingTree
                | Self::SummarizingBranch
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Connected,
    Disconnected,
}

impl ConnectionState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::Disconnected => "disconnected",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserMessageStatus {
    Pending,
    Accepted,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    WaitingApproval,
    Running,
    Succeeded,
    Failed,
    Denied,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserMessage {
    pub text: String,
    pub status: UserMessageStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AssistantMessage {
    /// Stable inside one application session epoch.
    pub id: u64,
    pub session_epoch: u64,
    pub text: String,
    pub thinking: String,
    pub text_revision: u64,
    pub thinking_revision: u64,
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolExecution {
    pub id: String,
    pub name: String,
    pub args: Value,
    pub output: String,
    pub diff: Option<ToolDiff>,
    pub status: ToolStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnSeparator {
    pub turn_id: String,
    pub started_at: String,
    pub ended_at: String,
    pub duration_ms: u64,
    #[serde(default)]
    pub estimated: bool,
}
