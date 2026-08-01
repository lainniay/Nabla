use crate::{
    command::{CommandCatalog, CommandSpec, DiscoveredCommand},
    rpc::PiState,
    selection::{next_wrapped, previous_wrapped},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::ui_types::{ComposerViewport, UiLayoutMetrics};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Idle,
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
    AgentPicker,
    Transcript,
    Question,
    Auth,
    Approval,
    Integration,
    GoalApproval,
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
    pub search_query: String,
    pub search_active: bool,
    pub search_matches: Vec<usize>,
    pub current_match: Option<usize>,
    pub opened_item_count: usize,
}

impl TranscriptViewerState {
    pub fn new(mode: TranscriptViewMode, transcript: &[TranscriptItem]) -> Self {
        Self {
            mode,
            selected_item: transcript
                .iter()
                .enumerate()
                .rev()
                .find_map(|(index, item)| matches!(item, TranscriptItem::Tool(_)).then_some(index)),
            scroll_from_bottom: 0,
            follow_tail: true,
            scroll_to_selected: false,
            tool_expansion_overrides: HashMap::new(),
            search_query: String::new(),
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
            Self::Submitting
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
    pub text: String,
    pub thinking: String,
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolExecution {
    pub id: String,
    pub name: String,
    pub args: Value,
    pub output: String,
    pub status: ToolStatus,
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContextUsageState {
    Actual,
    Estimated,
    Recalculating,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContextCategory {
    User,
    Assistant,
    ToolResult,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextCategoryEstimate {
    pub category: ContextCategory,
    pub message_count: u64,
    pub estimated_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextConsumer {
    pub category: ContextCategory,
    pub label: String,
    pub estimated_tokens: u64,
    #[serde(default)]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PruneReason {
    HardLimit,
    HistoryBudget,
    Superseded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextPruneEstimate {
    pub reason: PruneReason,
    pub count: u64,
    pub estimated_tokens_saved: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextPolicy {
    pub enabled: bool,
    pub recent_tool_result_tokens: u64,
    pub minimum_batch_savings_tokens: u64,
    pub minimum_tool_result_tokens: u64,
    pub success_tool_result_limit_tokens: u64,
    pub search_tool_result_limit_tokens: u64,
    pub error_tool_result_limit_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionRecord {
    pub reason: String,
    pub first_kept_entry_id: String,
    pub tokens_before: u64,
    pub estimated_tokens_after: Option<u64>,
    pub tokens_saved: Option<u64>,
    pub saved_percent: Option<f64>,
    pub file_count: u64,
    pub read_file_count: u64,
    pub modified_file_count: u64,
}

impl CompactionRecord {
    pub fn file_count(&self) -> u64 {
        self.file_count
    }

    pub fn deduplication_key(&self) -> String {
        format!(
            "{}\0{}\0{}",
            self.reason, self.first_kept_entry_id, self.tokens_before
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    #[serde(default)]
    pub revision: u64,
    pub usage_state: ContextUsageState,
    pub actual_tokens: Option<u64>,
    pub actual_percent: Option<f64>,
    pub context_window: Option<u64>,
    pub estimated_unfiltered_tokens: u64,
    pub estimated_next_request_tokens: u64,
    pub categories: Vec<ContextCategoryEstimate>,
    pub estimated_system_tool_other_tokens: Option<u64>,
    pub estimated_pruned_this_request_tokens: u64,
    pub estimated_currently_prunable_tokens: u64,
    pub estimated_cumulative_avoided_tokens: u64,
    pub pruning: Vec<ContextPruneEstimate>,
    pub top_consumers: Vec<ContextConsumer>,
    pub compaction_count: u64,
    pub recent_compactions: Vec<CompactionRecord>,
    pub policy: ContextPolicy,
    pub epoch: u64,
}

impl Default for ContextSnapshot {
    fn default() -> Self {
        Self {
            scope_id: None,
            revision: 0,
            usage_state: ContextUsageState::Estimated,
            actual_tokens: None,
            actual_percent: None,
            context_window: None,
            estimated_unfiltered_tokens: 0,
            estimated_next_request_tokens: 0,
            categories: Vec::new(),
            estimated_system_tool_other_tokens: None,
            estimated_pruned_this_request_tokens: 0,
            estimated_currently_prunable_tokens: 0,
            estimated_cumulative_avoided_tokens: 0,
            pruning: Vec::new(),
            top_consumers: Vec::new(),
            compaction_count: 0,
            recent_compactions: Vec::new(),
            policy: ContextPolicy {
                enabled: true,
                recent_tool_result_tokens: 40_000,
                minimum_batch_savings_tokens: 20_000,
                minimum_tool_result_tokens: 50,
                success_tool_result_limit_tokens: 12_000,
                search_tool_result_limit_tokens: 6_000,
                error_tool_result_limit_tokens: 8_000,
            },
            epoch: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalState {
    pub approval_id: String,
    pub tool_call_id: String,
    pub tool_name: String,
    pub input: Value,
    pub agent_id: Option<String>,
    pub agent_profile: Option<String>,
    pub model: Option<String>,
    pub goal_id: Option<String>,
    pub reason: Option<String>,
    pub risk: Option<String>,
    pub selected: usize,
    pub replying: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceDescriptor {
    pub name: String,
    pub path: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceDiagnostic {
    #[serde(rename = "type")]
    pub kind: String,
    pub message: String,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ResourceSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    pub trusted: bool,
    pub context_files: Vec<String>,
    pub skills: Vec<ResourceDescriptor>,
    pub prompts: Vec<ResourceDescriptor>,
    pub extensions: Vec<String>,
    #[serde(default)]
    pub commands: Vec<DiscoveredCommand>,
    pub diagnostics: Vec<ResourceDiagnostic>,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalTask {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub profile: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub allowed_paths: Vec<String>,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    pub status: String,
    #[serde(default)]
    pub result: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalSpec {
    pub revision: u64,
    pub summary: String,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub allowed_paths: Vec<String>,
    #[serde(default)]
    pub allowed_commands: Vec<String>,
    #[serde(default)]
    pub source_plan: Option<Value>,
    #[serde(default)]
    pub tasks: Vec<GoalSpecTask>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalSpecTask {
    pub id: String,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub allowed_paths: Vec<String>,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalRecord {
    #[serde(default)]
    pub schema_version: u8,
    pub id: String,
    #[serde(default)]
    pub workspace: String,
    pub session_id: String,
    pub objective: String,
    pub stage: String,
    #[serde(default)]
    pub previous_stage: Option<String>,
    pub revision: u64,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub source_plan: Option<Value>,
    #[serde(default)]
    pub spec: Option<GoalSpec>,
    #[serde(default)]
    pub tasks: Vec<GoalTask>,
    #[serde(default)]
    pub lease: Option<Value>,
    #[serde(default)]
    pub reviews: Vec<Value>,
    #[serde(default)]
    pub verification: Vec<Value>,
    #[serde(default)]
    pub repair_cycles: u64,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    pub goal: Option<GoalRecord>,
    pub state_path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalsSnapshot {
    pub goals: Vec<GoalRecord>,
    pub state_directory: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfileSnapshot {
    pub name: String,
    pub description: String,
    pub source: String,
    pub model: Option<String>,
    pub thinking_level: Option<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    pub tools: Vec<String>,
    pub permission: String,
    pub max_parallel: u64,
    pub max_turns: u64,
    #[serde(default)]
    pub isolation: AgentIsolationSnapshot,
    pub disabled: bool,
    #[serde(default)]
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentIsolationSnapshot {
    pub mode: String,
    pub integration: String,
}

impl Default for AgentIsolationSnapshot {
    fn default() -> Self {
        Self {
            mode: "none".to_owned(),
            integration: "source".to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveAgentSnapshot {
    pub id: String,
    pub profile: String,
    pub task: String,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub goal_id: Option<String>,
    pub lifecycle: String,
    pub started_at: String,
    pub turns: u64,
    pub max_turns: u64,
    pub model: String,
    pub origin_session_id: String,
    #[serde(default = "default_shared_backend")]
    pub isolation_backend: String,
    #[serde(default = "default_integration_status")]
    pub integration_status: String,
    #[serde(default)]
    pub isolation_warning: Option<String>,
}

fn default_shared_backend() -> String {
    "shared".to_owned()
}

fn default_integration_status() -> String {
    "none".to_owned()
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeIntegrationSnapshot {
    pub backend: String,
    pub status: String,
    #[serde(default)]
    pub warning: Option<String>,
    #[serde(default)]
    pub artifact_id: Option<String>,
    #[serde(default)]
    pub changed_paths: Vec<String>,
    #[serde(default)]
    pub patch_bytes: u64,
    #[serde(default)]
    pub excluded_paths: Vec<String>,
    #[serde(default = "default_true")]
    pub resolver_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationPromptState {
    pub agent: ActiveAgentSnapshot,
    pub integration: WorktreeIntegrationSnapshot,
    pub selected: usize,
    pub submitting: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentConfigDiagnostic {
    #[serde(rename = "type")]
    pub kind: String,
    pub message: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub profile: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentsSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    #[serde(default)]
    pub revision: u64,
    pub max_parallel: u64,
    pub profiles: Vec<AgentProfileSnapshot>,
    pub active: Vec<ActiveAgentSnapshot>,
    #[serde(default)]
    pub pending: Vec<ActiveAgentSnapshot>,
    #[serde(default)]
    pub diagnostics: Vec<AgentConfigDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentTranscript {
    pub event: String,
    pub agent: ActiveAgentSnapshot,
    pub result: Option<Value>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentPickerState {
    pub profiles: Vec<AgentProfileSnapshot>,
    pub selected: usize,
}

impl AgentPickerState {
    pub fn new(snapshot: &AgentsSnapshot) -> Self {
        let profiles = snapshot
            .profiles
            .iter()
            .filter(|profile| !profile.disabled && profile.unavailable_reason.is_none())
            .cloned()
            .collect();
        Self {
            profiles,
            selected: 0,
        }
    }

    pub fn selected_profile(&self) -> Option<&AgentProfileSnapshot> {
        self.profiles.get(self.selected)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlanStatus {
    #[serde(alias = "ready")]
    Submitted,
    Executing,
    Completed,
}

impl PlanStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Submitted => "submitted",
            Self::Executing => "executing",
            Self::Completed => "completed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanArtifact {
    pub schema_version: u8,
    pub id: String,
    pub revision: u64,
    pub status: PlanStatus,
    pub title: String,
    pub summary: String,
    pub body_markdown: String,
    pub assumptions: Vec<String>,
    pub test_plan: Vec<String>,
    pub source_session_id: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub last_execution_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanExecutionTarget {
    Current,
    Fresh,
}

impl PlanExecutionTarget {
    pub fn label(self) -> &'static str {
        match self {
            Self::Current => "current context",
            Self::Fresh => "fresh context",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanReviewState {
    Menu {
        selected: usize,
    },
    Confirm {
        target: PlanExecutionTarget,
        selected: usize,
        submitting: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalApprovalState {
    pub selected: usize,
    pub submitting: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionOption {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanQuestion {
    pub id: String,
    pub prompt: String,
    pub options: Vec<QuestionOption>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionAnswer {
    pub question_id: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub option_id: Option<String>,
}

#[derive(Clone)]
pub struct QuestionFlowState {
    pub request_id: String,
    pub questions: Vec<PlanQuestion>,
    pub current: usize,
    pub selected: usize,
    pub custom_answer: bool,
    pub editor: EditorState,
    pub answers: Vec<QuestionAnswer>,
    pub replying: bool,
}

impl QuestionFlowState {
    pub fn current_question(&self) -> Option<&PlanQuestion> {
        self.questions.get(self.current)
    }

    pub fn choice_count(&self) -> usize {
        self.current_question()
            .map_or(0, |question| question.options.len() + 1)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthChoice {
    pub provider_id: String,
    pub provider_name: String,
    pub auth_type: String,
    pub label: String,
    pub configured: bool,
}

pub fn matching_auth_choice_indices(choices: &[AuthChoice], query: &str) -> Vec<usize> {
    let terms = query
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    choices
        .iter()
        .enumerate()
        .filter_map(|(index, choice)| {
            let searchable = format!(
                "{} {} {} {}",
                choice.provider_name, choice.provider_id, choice.label, choice.auth_type
            )
            .to_lowercase();
            terms
                .iter()
                .all(|term| searchable.contains(term))
                .then_some(index)
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthPromptKind {
    Text,
    Secret,
    Select,
    ManualCode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthPromptOption {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
}

#[derive(Clone)]
pub struct AuthPromptState {
    pub id: String,
    pub kind: AuthPromptKind,
    pub message: String,
    pub placeholder: Option<String>,
    pub options: Vec<AuthPromptOption>,
    pub selected: usize,
    pub editor: EditorState,
}

#[derive(Clone)]
pub struct AuthFlowState {
    pub id: String,
    pub provider_name: String,
    pub status: String,
    pub url: Option<String>,
    pub device_code: Option<String>,
    pub prompt: Option<AuthPromptState>,
}

#[derive(Clone)]
pub enum AuthState {
    Inactive,
    LoadingProviders,
    Selecting {
        choices: Vec<AuthChoice>,
        selected: usize,
        filter: EditorState,
    },
    Running(Box<AuthFlowState>),
}

impl AuthState {
    pub fn is_active(&self) -> bool {
        !matches!(self, Self::Inactive)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionBrowserState {
    pub browser_id: Option<String>,
    pub current_cwd: String,
    pub scope: SessionScope,
    pub sort_mode: SessionSortMode,
    pub named_only: bool,
    pub show_path: bool,
    pub query: EditorState,
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
            show_path: false,
            query: EditorState::default(),
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
    pub folded_entry_ids: HashSet<String>,
    pub show_label_timestamps: bool,
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
            folded_entry_ids: HashSet::new(),
            show_label_timestamps: false,
            phase: TreePhase::Browse,
            loading: true,
            generation: 0,
        }
    }

    pub fn selected_item(&self) -> Option<&TreeItem> {
        self.items.get(self.selected)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TranscriptItem {
    User(UserMessage),
    Assistant(AssistantMessage),
    Tool(ToolExecution),
    Plan(PlanArtifact),
    Context(ContextSnapshot),
    Resources(ResourceSnapshot),
    Goal(Box<GoalSnapshot>),
    Goals(GoalsSnapshot),
    Agents(AgentsSnapshot),
    Subagent(SubagentTranscript),
    Compaction(CompactionRecord),
    BranchSummary(String),
    SessionBoundary {
        action: String,
        label: String,
        cwd: String,
    },
    Notice(String),
    Error(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditorState {
    text: String,
    /// Cursor position in extended grapheme clusters, never bytes or scalar values.
    cursor: usize,
    preferred_visual_column: Option<usize>,
}

impl EditorState {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn insert_char(&mut self, character: char) {
        let byte_index = self.byte_index();
        self.text.insert(byte_index, character);
        self.cursor = self.grapheme_index_after_byte(byte_index + character.len_utf8());
        self.preferred_visual_column = None;
    }

    pub fn insert_text(&mut self, text: &str) {
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        let byte_index = self.byte_index();
        self.text.insert_str(byte_index, &normalized);
        self.cursor = self.grapheme_index_after_byte(byte_index + normalized.len());
        self.preferred_visual_column = None;
    }

    pub fn insert_newline(&mut self) {
        self.insert_text("\n");
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let end = self.byte_index();
        self.cursor -= 1;
        let start = self.byte_index();
        self.text.replace_range(start..end, "");
        self.preferred_visual_column = None;
    }

    pub fn delete(&mut self) {
        if self.cursor == self.grapheme_count() {
            return;
        }
        let start = self.byte_index();
        self.cursor += 1;
        let end = self.byte_index();
        self.cursor -= 1;
        self.text.replace_range(start..end, "");
        self.preferred_visual_column = None;
    }

    pub fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
        self.preferred_visual_column = None;
    }

    pub fn move_right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.grapheme_count());
        self.preferred_visual_column = None;
    }

    pub fn move_home(&mut self) {
        let byte = self.byte_index();
        let line_start = self.text[..byte].rfind('\n').map_or(0, |index| index + 1);
        self.cursor = self.grapheme_index_after_byte(line_start);
        self.preferred_visual_column = None;
    }

    pub fn move_end(&mut self) {
        let byte = self.byte_index();
        let line_end = self.text[byte..]
            .find('\n')
            .map_or(self.text.len(), |index| byte + index);
        self.cursor = self.grapheme_index_after_byte(line_end);
        self.preferred_visual_column = None;
    }

    pub fn move_up(&mut self, width: usize) {
        self.move_vertical(width, -1);
    }

    pub fn move_down(&mut self, width: usize) {
        self.move_vertical(width, 1);
    }

    pub fn composer_viewport(&self, width: usize, maximum_rows: u16) -> ComposerViewport {
        let width = width.max(1);
        let positions = self.visual_positions(width);
        let (cursor_visual_row, cursor_visual_column) =
            positions.get(self.cursor).copied().unwrap_or_default();
        let total_visual_rows = positions
            .last()
            .map_or(1, |(row, _)| row + 1)
            .max(cursor_visual_row + 1);
        let visible_rows = (total_visual_rows as u16).clamp(1, maximum_rows.max(1));
        let first_visual_row =
            cursor_visual_row.saturating_sub(visible_rows.saturating_sub(1) as usize);
        ComposerViewport {
            first_visual_row,
            visible_rows,
            total_visual_rows,
            cursor_visual_row,
            cursor_visual_column,
        }
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.preferred_visual_column = None;
    }

    pub(crate) fn replace(&mut self, text: String) {
        self.cursor = text.graphemes(true).count();
        self.text = text;
        self.preferred_visual_column = None;
    }

    pub(crate) fn take(&mut self) -> String {
        self.cursor = 0;
        self.preferred_visual_column = None;
        std::mem::take(&mut self.text)
    }

    fn byte_index(&self) -> usize {
        self.text
            .grapheme_indices(true)
            .nth(self.cursor)
            .map_or(self.text.len(), |(index, _)| index)
    }

    fn grapheme_count(&self) -> usize {
        self.text.graphemes(true).count()
    }

    fn grapheme_index_after_byte(&self, byte: usize) -> usize {
        self.text[..byte.min(self.text.len())]
            .graphemes(true)
            .count()
    }

    fn visual_positions(&self, width: usize) -> Vec<(usize, usize)> {
        let width = width.max(1);
        let mut row = 0usize;
        let mut column = 0usize;
        let mut positions = vec![(row, column)];
        for grapheme in self.text.graphemes(true) {
            if grapheme == "\n" {
                row += 1;
                column = 0;
            } else {
                let grapheme_width = UnicodeWidthStr::width(grapheme);
                if column > 0 && column.saturating_add(grapheme_width) > width {
                    row += 1;
                    column = 0;
                }
                column = column.saturating_add(grapheme_width);
                if column >= width {
                    row += column / width;
                    column %= width;
                }
            }
            positions.push((row, column));
        }
        positions
    }

    fn move_vertical(&mut self, width: usize, delta: isize) {
        let positions = self.visual_positions(width);
        let (row, column) = positions.get(self.cursor).copied().unwrap_or_default();
        let preferred = self.preferred_visual_column.unwrap_or(column);
        self.preferred_visual_column = Some(preferred);
        let target_row = if delta < 0 {
            row.saturating_sub(delta.unsigned_abs())
        } else {
            row.saturating_add(delta as usize)
        };
        if target_row == row {
            return;
        }
        let candidates = positions
            .iter()
            .enumerate()
            .filter(|(_, (candidate_row, _))| *candidate_row == target_row);
        if let Some((index, _)) =
            candidates.min_by_key(|(_, (_, candidate_column))| candidate_column.abs_diff(preferred))
        {
            self.cursor = index;
        }
    }
}

/// Complete state consumed by the UI. It contains data only; event handling
/// and I/O remain in `App` and the runtime.
pub struct AppState {
    pub editor: EditorState,
    pub transcript: Vec<TranscriptItem>,
    pub session: PiState,
    pub run_state: RunState,
    pub connection_state: ConnectionState,
    pub last_error: Option<String>,
    pub command_catalog: CommandCatalog,
    pub auth_state: AuthState,
    pub plan_mode_active: bool,
    pub pending_plan_mode: Option<bool>,
    pub approval: Option<ApprovalState>,
    pub question: Option<QuestionFlowState>,
    pub plan: Option<PlanArtifact>,
    pub plan_review: Option<PlanReviewState>,
    pub goal_approval: Option<GoalApprovalState>,
    pub session_browser: Option<SessionBrowserState>,
    pub tree_browser: Option<TreeBrowserState>,
    pub transcript_viewer: Option<TranscriptViewerState>,
    pub transcript_view_mode: TranscriptViewMode,
    pub context: ContextSnapshot,
    pub resources: ResourceSnapshot,
    pub goal: Option<GoalSnapshot>,
    pub agents: AgentsSnapshot,
    pub agent_picker: Option<AgentPickerState>,
    pub integration_prompt: Option<IntegrationPromptState>,
    pub integration_prompt_queue: VecDeque<IntegrationPromptState>,
    pub open_agent_picker_on_agents: bool,
    pub next_auth_flow_id: u64,
    pub layout_metrics: UiLayoutMetrics,
    pub selector_page_rows: usize,
    pub(crate) seen_compactions: HashSet<String>,
    pub(crate) compact_lifecycle_finished: bool,
    command_menu_selected: usize,
    command_menu_dismissed: bool,
    redraw_requested: bool,
}

impl AppState {
    pub fn new(session: PiState) -> Self {
        Self::with_commands(session, Vec::new())
    }

    pub fn with_commands(session: PiState, commands: Vec<DiscoveredCommand>) -> Self {
        let run_state = if session.is_compacting {
            RunState::Compacting
        } else if session.is_streaming {
            RunState::Running
        } else {
            RunState::Idle
        };

        Self {
            editor: EditorState::default(),
            transcript: Vec::new(),
            session,
            run_state,
            connection_state: ConnectionState::Connected,
            last_error: None,
            command_catalog: CommandCatalog::new(commands),
            auth_state: AuthState::Inactive,
            plan_mode_active: false,
            pending_plan_mode: None,
            approval: None,
            question: None,
            plan: None,
            plan_review: None,
            goal_approval: None,
            session_browser: None,
            tree_browser: None,
            transcript_viewer: None,
            transcript_view_mode: TranscriptViewMode::Normal,
            context: ContextSnapshot::default(),
            resources: ResourceSnapshot::default(),
            goal: None,
            agents: AgentsSnapshot::default(),
            agent_picker: None,
            integration_prompt: None,
            integration_prompt_queue: VecDeque::new(),
            open_agent_picker_on_agents: false,
            next_auth_flow_id: 1,
            layout_metrics: UiLayoutMetrics {
                desired_height: 14,
                body_height: 8,
                ..UiLayoutMetrics::default()
            },
            selector_page_rows: 8,
            seen_compactions: HashSet::new(),
            compact_lifecycle_finished: false,
            command_menu_selected: 0,
            command_menu_dismissed: false,
            redraw_requested: true,
        }
    }

    pub fn model_label(&self) -> String {
        let Some(model) = self.session.model.as_ref() else {
            return "no model".to_owned();
        };
        let provider = model
            .get("provider")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        let name = model
            .get("name")
            .or_else(|| model.get("id"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        format!("{provider}/{name}")
    }

    pub fn can_submit(&self) -> bool {
        self.connection_state == ConnectionState::Connected
            && !self.run_state.is_busy()
            && self.active_modal_kind().is_none()
            && self.pending_plan_mode.is_none()
    }

    pub fn can_abort(&self) -> bool {
        self.connection_state == ConnectionState::Connected && self.run_state.is_busy()
    }

    pub fn can_toggle_plan_mode(&self) -> bool {
        self.connection_state == ConnectionState::Connected
            && !self.run_state.is_busy()
            && self.active_modal_kind().is_none()
            && self.pending_plan_mode.is_none()
    }

    pub fn active_modal_kind(&self) -> Option<UiModalKind> {
        if self.session_browser.is_some() {
            Some(UiModalKind::SessionBrowser)
        } else if self.tree_browser.is_some() {
            Some(UiModalKind::TreeBrowser)
        } else if self.agent_picker.is_some() {
            Some(UiModalKind::AgentPicker)
        } else if self.question.is_some() {
            Some(UiModalKind::Question)
        } else if self.auth_state.is_active() {
            Some(UiModalKind::Auth)
        } else if self.approval.is_some() {
            Some(UiModalKind::Approval)
        } else if self.integration_prompt.is_some() {
            Some(UiModalKind::Integration)
        } else if self.goal_approval.is_some() {
            Some(UiModalKind::GoalApproval)
        } else if self.plan_review.is_some() {
            Some(UiModalKind::PlanReview)
        } else if self.transcript_viewer.is_some() {
            Some(UiModalKind::Transcript)
        } else {
            None
        }
    }

    pub fn command_candidates(&self) -> Vec<&CommandSpec> {
        if self.command_menu_dismissed {
            return Vec::new();
        }
        self.command_catalog.candidates(
            self.editor.text(),
            self.editor.cursor(),
            self.command_catalog.commands().len(),
        )
    }

    pub fn selected_command(&self) -> Option<&CommandSpec> {
        self.command_candidates()
            .get(self.command_menu_selected)
            .copied()
    }

    pub fn command_menu_selected(&self) -> usize {
        self.command_menu_selected
    }

    pub(crate) fn command_completion(&self) -> Option<String> {
        let command = self.selected_command()?;
        Some(format!("/{} ", command.name))
    }

    pub(crate) fn command_needs_completion(&self) -> bool {
        let Some(command) = self.selected_command() else {
            return false;
        };
        self.editor.text().trim() != format!("/{}", command.name)
    }

    pub(crate) fn select_previous_command(&mut self) {
        let count = self.command_candidates().len();
        if count == 0 {
            return;
        }
        self.command_menu_selected = previous_wrapped(self.command_menu_selected, count);
    }

    pub(crate) fn select_next_command(&mut self) {
        let count = self.command_candidates().len();
        if count == 0 {
            return;
        }
        self.command_menu_selected = next_wrapped(self.command_menu_selected, count);
    }

    pub(crate) fn select_command(&mut self, index: usize) {
        let count = self.command_candidates().len();
        if count > 0 {
            self.command_menu_selected = index.min(count - 1);
        }
    }

    pub(crate) fn reset_command_menu(&mut self) {
        self.command_menu_selected = 0;
        self.command_menu_dismissed = false;
    }

    pub(crate) fn dismiss_command_menu(&mut self) {
        self.command_menu_dismissed = true;
    }

    pub(crate) fn request_redraw(&mut self) {
        self.redraw_requested = true;
    }

    pub(crate) fn take_redraw_request(&mut self) -> bool {
        std::mem::take(&mut self.redraw_requested)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn session(is_streaming: bool, is_compacting: bool) -> PiState {
        PiState {
            model: Some(json!({"provider": "test", "id": "model-1"})),
            thinking_level: "off".to_owned(),
            is_streaming,
            is_compacting,
            steering_mode: "one-at-a-time".to_owned(),
            follow_up_mode: "one-at-a-time".to_owned(),
            session_file: None,
            session_id: "session-1".to_owned(),
            session_name: None,
            auto_compaction_enabled: true,
            message_count: 0,
            pending_message_count: 0,
        }
    }

    #[test]
    fn editor_preserves_paste_newlines_and_deletes_whole_graphemes() {
        let mut editor = EditorState::default();
        editor.insert_text("a\r\n你e\u{301}\r🙂");
        assert_eq!(editor.text(), "a\n你e\u{301}\n🙂");

        editor.backspace();
        assert_eq!(editor.text(), "a\n你e\u{301}\n");
        editor.backspace();
        editor.backspace();
        assert_eq!(editor.text(), "a\n你");
    }

    #[test]
    fn composer_viewport_grows_to_eight_rows_then_follows_cursor() {
        let mut editor = EditorState::default();
        editor.insert_text(
            &(1..=12)
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        );
        let viewport = editor.composer_viewport(20, 8);

        assert_eq!(viewport.visible_rows, 8);
        assert!(viewport.total_visual_rows >= 12);
        assert!(viewport.first_visual_row > 0);
        assert!(viewport.cursor_visual_row >= viewport.first_visual_row);
    }

    #[test]
    fn maps_pi_session_flags_to_initial_run_state() {
        assert_eq!(
            AppState::new(session(false, false)).run_state,
            RunState::Idle
        );
        assert_eq!(
            AppState::new(session(true, false)).run_state,
            RunState::Running
        );
        assert_eq!(
            AppState::new(session(true, true)).run_state,
            RunState::Compacting
        );
    }

    #[test]
    fn exposes_a_stable_model_label_for_the_ui() {
        let state = AppState::new(session(false, false));

        assert_eq!(state.model_label(), "test/model-1");
        assert_eq!(state.connection_state, ConnectionState::Connected);
    }

    #[test]
    fn context_snapshot_uses_the_camel_case_host_protocol() {
        let snapshot: ContextSnapshot = serde_json::from_value(json!({
            "usageState": "actual",
            "actualTokens": 47_000,
            "actualPercent": 47.0,
            "contextWindow": 100_000,
            "estimatedUnfilteredTokens": 55_000,
            "estimatedNextRequestTokens": 43_000,
            "categories": [{
                "category": "toolResult",
                "messageCount": 2,
                "estimatedTokens": 40_000
            }],
            "estimatedSystemToolOtherTokens": 8_000,
            "estimatedPrunedThisRequestTokens": 12_000,
            "estimatedCurrentlyPrunableTokens": 3_000,
            "estimatedCumulativeAvoidedTokens": 24_000,
            "pruning": [{
                "reason": "hard_limit",
                "count": 1,
                "estimatedTokensSaved": 12_000
            }],
            "topConsumers": [{
                "category": "toolResult",
                "label": "read result",
                "estimatedTokens": 40_000,
                "toolCallId": "call-1"
            }],
            "compactionCount": 1,
            "recentCompactions": [{
                "reason": "manual",
                "firstKeptEntryId": "entry-1",
                "tokensBefore": 82_000,
                "estimatedTokensAfter": 31_000,
                "tokensSaved": 51_000,
                "savedPercent": 62.2,
                "fileCount": 3,
                "readFileCount": 2,
                "modifiedFileCount": 2
            }],
            "policy": {
                "enabled": true,
                "recentToolResultTokens": 40_000,
                "minimumBatchSavingsTokens": 20_000,
                "minimumToolResultTokens": 50,
                "successToolResultLimitTokens": 12_000,
                "searchToolResultLimitTokens": 6_000,
                "errorToolResultLimitTokens": 8_000
            },
            "epoch": 2
        }))
        .unwrap();

        assert_eq!(snapshot.usage_state, ContextUsageState::Actual);
        assert_eq!(snapshot.categories[0].category, ContextCategory::ToolResult);
        assert_eq!(snapshot.pruning[0].reason, PruneReason::HardLimit);
        assert_eq!(snapshot.recent_compactions[0].file_count(), 3);
        let encoded = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(encoded["estimatedNextRequestTokens"], 43_000);
        assert!(encoded.get("estimated_next_request_tokens").is_none());
    }

    #[test]
    fn agents_snapshot_uses_config_and_runtime_fields() {
        let snapshot: AgentsSnapshot = serde_json::from_value(json!({
            "maxParallel": 3,
            "profiles": [{
                "name": "reviewer",
                "description": "Review changes",
                "source": "builtin",
                "model": null,
                "thinkingLevel": "high",
                "skills": [],
                "tools": ["read"],
                "permission": "read:allow",
                "maxParallel": 1,
                "maxTurns": 12,
                "disabled": false,
                "unavailableReason": null
            }],
            "active": [{
                "id": "agent-1",
                "profile": "reviewer",
                "task": "Review",
                "lifecycle": "running",
                "startedAt": "2026-01-01T00:00:00Z",
                "turns": 2,
                "maxTurns": 12,
                "model": "test/model",
                "originSessionId": "session-1"
            }],
            "pending": [{
                "id": "agent-2",
                "profile": "worker",
                "task": "Implement",
                "lifecycle": "awaiting_integration",
                "startedAt": "2026-01-01T00:00:00Z",
                "turns": 3,
                "maxTurns": 32,
                "model": "test/model",
                "originSessionId": "session-1",
                "integrationStatus": "pending"
            }],
            "diagnostics": [{
                "type": "warning",
                "message": "example"
            }]
        }))
        .unwrap();

        assert_eq!(snapshot.profiles[0].max_turns, 12);
        assert_eq!(snapshot.active[0].origin_session_id, "session-1");
        assert_eq!(snapshot.pending[0].integration_status, "pending");
        assert_eq!(snapshot.diagnostics[0].kind, "warning");
    }

    #[test]
    fn auth_choice_search_matches_provider_method_and_multiple_terms() {
        let choices = vec![
            AuthChoice {
                provider_id: "openai-codex".to_owned(),
                provider_name: "OpenAI Codex".to_owned(),
                auth_type: "oauth".to_owned(),
                label: "ChatGPT Plus/Pro".to_owned(),
                configured: false,
            },
            AuthChoice {
                provider_id: "github-copilot".to_owned(),
                provider_name: "GitHub Copilot".to_owned(),
                auth_type: "oauth".to_owned(),
                label: "Device login".to_owned(),
                configured: false,
            },
        ];

        assert_eq!(matching_auth_choice_indices(&choices, ""), vec![0, 1]);
        assert_eq!(
            matching_auth_choice_indices(&choices, "OPENAI plus"),
            vec![0]
        );
        assert_eq!(
            matching_auth_choice_indices(&choices, "github device"),
            vec![1]
        );
        assert!(matching_auth_choice_indices(&choices, "missing").is_empty());
    }
}
