use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionScope {
    Current,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionSortMode {
    Threaded,
    Recent,
    Relevance,
}

impl SessionSortMode {
    pub fn next(self) -> Self {
        match self {
            Self::Threaded => Self::Recent,
            Self::Recent => Self::Relevance,
            Self::Relevance => Self::Threaded,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Threaded => "threaded",
            Self::Recent => "recent",
            Self::Relevance => "relevance",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub path: String,
    pub id: String,
    pub cwd: String,
    pub cwd_available: bool,
    pub name: Option<String>,
    pub parent_session_path: Option<String>,
    pub created_at: String,
    pub modified_at: String,
    pub message_count: u64,
    pub first_message: String,
    pub depth: usize,
    pub is_last: bool,
    pub current: bool,
}

impl SessionSummary {
    pub fn label(&self) -> &str {
        self.name
            .as_deref()
            .filter(|name| !name.is_empty())
            .unwrap_or(&self.first_message)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionBrowserSnapshot {
    pub browser_id: String,
    pub current_cwd: String,
    pub scope: SessionScope,
    pub query: String,
    pub sort_mode: SessionSortMode,
    pub named_only: bool,
    pub sessions: Vec<SessionSummary>,
    pub total: usize,
    #[serde(default)]
    pub offset: usize,
    #[serde(default)]
    pub next_offset: Option<usize>,
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SessionHistoryItem {
    User {
        text: String,
    },
    Assistant {
        text: String,
        thinking: String,
    },
    ToolCall {
        id: String,
        name: String,
        args: Value,
    },
    ToolResult {
        id: String,
        name: String,
        output: String,
        #[serde(default)]
        details: Option<Value>,
        is_error: bool,
    },
    Notice {
        text: String,
    },
    Compaction {
        first_kept_entry_id: String,
        tokens_before: u64,
        file_count: u64,
    },
    TurnBoundary {
        turn_id: String,
        started_at: String,
        ended_at: String,
        duration_ms: u64,
        #[serde(default)]
        estimated: bool,
    },
    BranchSummary {
        summary: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TreeFilterMode {
    Default,
    NoTools,
    UserOnly,
    LabeledOnly,
    All,
}

impl TreeFilterMode {
    pub fn next(self) -> Self {
        match self {
            Self::Default => Self::NoTools,
            Self::NoTools => Self::UserOnly,
            Self::UserOnly => Self::LabeledOnly,
            Self::LabeledOnly => Self::All,
            Self::All => Self::Default,
        }
    }

    pub fn previous(self) -> Self {
        match self {
            Self::Default => Self::All,
            Self::NoTools => Self::Default,
            Self::UserOnly => Self::NoTools,
            Self::LabeledOnly => Self::UserOnly,
            Self::All => Self::LabeledOnly,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::NoTools => "no-tools",
            Self::UserOnly => "user-only",
            Self::LabeledOnly => "labeled-only",
            Self::All => "all",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeItem {
    pub entry_id: String,
    pub parent_id: Option<String>,
    pub kind: String,
    pub role: Option<String>,
    pub preview: String,
    pub label: Option<String>,
    pub label_timestamp: Option<String>,
    pub visual_depth: usize,
    pub show_connector: bool,
    pub gutter_positions: Vec<usize>,
    pub is_last: bool,
    pub is_active_path: bool,
    pub is_leaf: bool,
    pub foldable: bool,
    pub folded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeSnapshot {
    pub items: Vec<TreeItem>,
    pub leaf_id: Option<String>,
    pub filter_mode: TreeFilterMode,
    pub query: String,
}
