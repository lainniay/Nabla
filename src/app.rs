use crossterm::event::{Event as TerminalEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use serde_json::Value;
use std::{
    fmt,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use crate::{
    browser::is_safe_web_url,
    command::{CommandRoute, DiscoveredCommand, LocalCommand},
    event::{CommandEvent, RuntimeEvent},
    file_references::{
        FileCompletionState, PreparedPrompt, PromptDelivery, completion_text, reference_tokens,
        token_at_cursor,
    },
    host::{ApprovalDecision, BootstrapStateData, SessionActivationData},
    rpc::{PiState, RpcEvent},
    selection::{next_wrapped, page_backward, page_forward, previous_wrapped},
};

pub use crate::event::AppEvent;
pub use crate::state::{
    ActiveAgentSnapshot, AgentPickerState, AgentsSnapshot, AppState, ApprovalRulesSnapshot,
    ApprovalState, AssistantMessage, AuthChoice, AuthFlowState, AuthPromptKind, AuthPromptOption,
    AuthPromptState, AuthState, CompactionRecord, ConnectionState, ContextCategory,
    ContextCategoryEstimate, ContextConsumer, ContextPolicy, ContextPruneEstimate, ContextSnapshot,
    ContextUsageState, EditorState, IntegrationPromptState, PermissionManagerState, PlanArtifact,
    PlanExecutionContext, PlanQuestion, PlanReviewState, PruneReason, QuestionAnswer,
    QuestionFlowState, QuestionOption, ResourceSnapshot, RunState, SelectionPanelAction,
    SelectionPanelKind, SelectionPanelOption, SelectionPanelState, SessionBrowserSnapshot,
    SessionBrowserState, SessionHistoryItem, SessionScope, SessionSortMode, SessionSummary,
    SubagentTranscript, THINKING_LEVELS, ToolExecution, ToolStatus, TranscriptItem,
    TranscriptViewMode, TranscriptViewerState, TreeBrowserState, TreeFilterMode, TreeItem,
    TreePhase, TreeSnapshot, TurnSeparator, UiModalKind, UserMessage, UserMessageStatus,
    WorktreeIntegrationSnapshot, matching_auth_choice_indices, parse_tool_diff,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppEffect {
    Prompt(String),
    Steer(String),
    FollowUp(String),
    SearchFiles {
        query: String,
        generation: u64,
    },
    PrepareReferences {
        message: String,
        delivery: PromptDelivery,
    },
    DeliverPrepared {
        prompt: PreparedPrompt,
        delivery: PromptDelivery,
    },
    Abort,
    ClearQueue,
    AbortAndClearQueue,
    Compact(Option<String>),
    GetContextState,
    GetResources,
    ReloadResources,
    SetWorkspaceTrust(bool),
    GetApprovalRules,
    RevokeApprovalRule(String),
    ClearApprovalRules,
    ListModels,
    SetModel {
        provider: String,
        model_id: String,
    },
    SetThinking(String),
    GetAgents,
    ReloadAgents,
    StartSubagent {
        profile: String,
        task: String,
    },
    CancelSubagent(String),
    IntegrateSubagent {
        agent_id: String,
        action: String,
    },
    OpenSessionBrowser,
    QuerySessionBrowser {
        browser_id: String,
        scope: SessionScope,
        query: String,
        sort_mode: SessionSortMode,
        named_only: bool,
        offset: usize,
        generation: u64,
    },
    CloseSessionBrowser {
        browser_id: String,
    },
    NewSession,
    ResumeSession {
        session_path: String,
        cwd_override: Option<String>,
    },
    GetTreeState {
        filter_mode: TreeFilterMode,
        query: String,
        folded_entry_ids: Vec<String>,
        generation: u64,
    },
    SetTreeLabel {
        entry_id: String,
        label: Option<String>,
    },
    CopyTreeEntry {
        entry_id: String,
    },
    NavigateTree {
        entry_id: String,
        summarize: bool,
        custom_instructions: Option<String>,
    },
    AbortTreeNavigation,
    AuthList,
    AuthLogin {
        flow_id: String,
        provider_id: String,
        auth_type: String,
    },
    AuthReply {
        flow_id: String,
        prompt_id: String,
        value: AuthResponse,
    },
    AuthCancel {
        flow_id: String,
    },
    OpenUrl(String),
    SetPlanMode(bool),
    ReplyApproval {
        approval_id: String,
        decision: ApprovalDecision,
    },
    ReplyQuestions {
        request_id: String,
        answers: Vec<QuestionAnswer>,
    },
    ExecutePlan(PlanExecutionContext),
    Quit,
    ExitWithError(String),
}

#[derive(Clone, PartialEq, Eq)]
pub struct AuthResponse(String);

impl AuthResponse {
    fn new(value: String) -> Self {
        Self(value)
    }

    pub fn into_inner(self) -> String {
        self.0
    }

    #[cfg(test)]
    fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for AuthResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthResponse([REDACTED])")
    }
}

// INFO: `App` is the reducer boundary for core state transitions. Transport
// adapters feed it events and execute the returned effects.
pub struct App {
    state: AppState,
    local_command_timing: Option<LocalCommandTiming>,
    next_local_turn_id: u64,
    pi_turn: PiTurnState,
    next_pi_turn_id: u64,
}

struct LocalCommandTiming {
    turn_id: String,
    started_at: String,
    started: Instant,
    completion: LocalCommandCompletion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PiTurnState {
    Inactive,
    Active {
        turn_id: String,
        started_at: String,
        started: Instant,
    },
    /// Attached while Pi was already streaming; the in-flight turn has no
    /// known start time, so its settled boundary is an estimated marker.
    AttachedUnknown,
}

#[derive(Clone, Copy)]
enum LocalCommandCompletion {
    Compact,
    Context,
    Resources,
    ResourceReload,
    WorkspaceTrust,
    ModelSet,
    ThinkingSet,
    Agents,
    AgentsReload,
}

impl LocalCommandCompletion {
    fn matches(self, event: &CommandEvent) -> bool {
        matches!(
            (self, event),
            (Self::Compact, CommandEvent::CompactFinished(_))
                | (Self::Context, CommandEvent::ContextStateFinished(_))
                | (Self::Resources, CommandEvent::ResourcesFinished(_))
                | (
                    Self::ResourceReload,
                    CommandEvent::ResourceReloadFinished(_)
                )
                | (
                    Self::WorkspaceTrust,
                    CommandEvent::WorkspaceTrustFinished(_)
                )
                | (Self::ModelSet, CommandEvent::ModelSetFinished(_))
                | (Self::ThinkingSet, CommandEvent::ThinkingSetFinished(_))
                | (Self::Agents, CommandEvent::AgentsFinished(_))
                | (Self::AgentsReload, CommandEvent::AgentsReloaded(_))
        )
    }
}

impl App {
    pub fn new(session: PiState) -> Self {
        let pi_turn = if session.is_streaming {
            PiTurnState::AttachedUnknown
        } else {
            PiTurnState::Inactive
        };
        Self {
            state: AppState::new(session),
            local_command_timing: None,
            next_local_turn_id: 1,
            pi_turn,
            next_pi_turn_id: 1,
        }
    }

    pub fn with_commands(session: PiState, commands: Vec<DiscoveredCommand>) -> Self {
        let pi_turn = if session.is_streaming {
            PiTurnState::AttachedUnknown
        } else {
            PiTurnState::Inactive
        };
        Self {
            state: AppState::with_commands(session, commands),
            local_command_timing: None,
            next_local_turn_id: 1,
            pi_turn,
            next_pi_turn_id: 1,
        }
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    pub fn set_selection_page_size(&mut self, page_size: usize) {
        self.state.selection_page_size = page_size.max(1);
    }

    pub fn set_initial_bootstrap_state(&mut self, bootstrap: BootstrapStateData) {
        self.state.command_catalog =
            crate::command::CommandCatalog::new(bootstrap.resources.commands.clone());
        self.state.plan_mode_active = bootstrap.plan_mode.active;
        self.state.sandbox_status = bootstrap.sandbox;
        self.state.plan = bootstrap.plan.artifact;
        self.state.resources = bootstrap.resources;
        self.state.agents = bootstrap.agents;
        self.state.context = bootstrap.context;
        if !self.state.resources.trusted {
            self.state.question = Some(QuestionFlowState {
                request_id: "workspace-trust".to_owned(),
                questions: vec![PlanQuestion {
                    id: "trust".to_owned(),
                    prompt: "This workspace is not trusted. Trust it to load project configuration and agent files?"
                        .to_owned(),
                    options: vec![
                        QuestionOption {
                            id: "trust".to_owned(),
                            label: "Trust workspace".to_owned(),
                            description: Some(
                                "Loads project .nabla config and agents files; saved to ~/.nabla/config.json"
                                    .to_owned(),
                            ),
                        },
                        QuestionOption {
                            id: "deny".to_owned(),
                            label: "Don't trust".to_owned(),
                            description: Some(
                                "Keeps project config and agents files disabled for this workspace"
                                    .to_owned(),
                            ),
                        },
                    ],
                }],
                current: 0,
                selected: 0,
                custom_answer: false,
                editor: EditorState::default(),
                answers: Vec::new(),
                replying: false,
                workspace_trust_prompt: true,
            });
        }
        for pending in bootstrap.pending_integrations {
            self.enqueue_integration_prompt(IntegrationPromptState {
                agent: pending.agent,
                selected: if pending.integration.status == "conflicted" {
                    if pending.integration.resolver_available {
                        1
                    } else {
                        2
                    }
                } else if pending.integration.status == "needs_reconciliation" {
                    2
                } else {
                    0
                },
                integration: pending.integration,
                submitting: false,
            });
        }
        for warning in bootstrap.warnings {
            self.state.transcript.push(TranscriptItem::Error(warning));
        }
    }

    pub fn update(&mut self, event: AppEvent) -> Vec<AppEffect> {
        let completes_local_command = match &event {
            AppEvent::Pi(event) if event.kind == "compaction_end" => self
                .local_command_timing
                .as_ref()
                .is_some_and(|timing| matches!(timing.completion, LocalCommandCompletion::Compact)),
            AppEvent::Command(event) => self
                .local_command_timing
                .as_ref()
                .is_some_and(|timing| timing.completion.matches(event)),
            _ => false,
        };
        // INFO: All asynchronous results re-enter through this single match so
        // stale-snapshot checks and lifecycle transitions remain deterministic.
        let effects = match event {
            AppEvent::Terminal(event) => self.update_terminal(event),
            AppEvent::Pi(event) => {
                self.update_pi(event);
                Vec::new()
            }
            AppEvent::Host(event) => self.update_host(event),
            AppEvent::Command(event) => self.update_command(event),
            AppEvent::Runtime(event) => self.update_runtime(event),
        };
        if completes_local_command {
            self.finish_local_command_timing();
        }
        if let Some(viewer) = self.state.transcript_viewer.as_mut()
            && viewer.follow_tail
        {
            viewer.opened_item_count = self.state.transcript.len();
        }
        effects
    }

    fn begin_local_command_timing(&mut self, completion: LocalCommandCompletion) {
        if self.local_command_timing.is_some() {
            return;
        }
        let wall_clock_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let turn_id = format!("local-{}", self.next_local_turn_id);
        self.next_local_turn_id = self.next_local_turn_id.saturating_add(1);
        self.local_command_timing = Some(LocalCommandTiming {
            turn_id,
            started_at: format!("unix-ms:{wall_clock_ms}"),
            started: Instant::now(),
            completion,
        });
    }

    fn finish_local_command_timing(&mut self) {
        let Some(timing) = self.local_command_timing.take() else {
            return;
        };
        let duration_ms = u64::try_from(timing.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.push_local_turn_separator(timing.turn_id, timing.started_at, duration_ms);
    }

    fn record_immediate_local_command_timing(&mut self) {
        let wall_clock_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let turn_id = format!("local-{}", self.next_local_turn_id);
        self.next_local_turn_id = self.next_local_turn_id.saturating_add(1);
        self.push_local_turn_separator(turn_id, format!("unix-ms:{wall_clock_ms}"), 0);
    }

    fn push_local_turn_separator(&mut self, turn_id: String, started_at: String, duration_ms: u64) {
        self.push_turn_separator(turn_id, started_at, duration_ms, false);
    }

    fn begin_pi_turn_timing(&mut self) {
        let wall_clock_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let turn_id = format!("pi-agent-{}", self.next_pi_turn_id);
        self.next_pi_turn_id = self.next_pi_turn_id.saturating_add(1);
        self.pi_turn = PiTurnState::Active {
            turn_id,
            started_at: format!("unix-ms:{wall_clock_ms}"),
            started: Instant::now(),
        };
    }

    fn push_turn_separator(
        &mut self,
        turn_id: String,
        started_at: String,
        duration_ms: u64,
        estimated: bool,
    ) {
        let ended_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        self.state
            .transcript
            .push(TranscriptItem::TurnSeparator(TurnSeparator {
                turn_id,
                started_at,
                ended_at: format!("unix-ms:{ended_at_ms}"),
                duration_ms,
                estimated,
            }));
    }

    fn update_terminal(&mut self, event: TerminalEvent) -> Vec<AppEffect> {
        match event {
            TerminalEvent::Key(key) if key.kind != KeyEventKind::Release => self.update_key(key),
            TerminalEvent::Paste(text) => {
                if self.state.run_state == RunState::PreparingReferences {
                    return Vec::new();
                }
                match self.state.active_modal_kind() {
                    Some(UiModalKind::Question) => {
                        if let Some(question) = self.state.question.as_mut()
                            && question.custom_answer
                            && !question.replying
                        {
                            question.editor.insert_text(&text);
                        }
                    }
                    Some(UiModalKind::SessionBrowser) => {
                        if let Some(browser) = self.state.session_browser.as_mut()
                            && !browser.switching
                            && browser.confirm_missing_cwd.is_none()
                            && browser.search_active
                        {
                            browser.query.insert_text(&text);
                            if let Some(effect) = self.refresh_session_browser_effect() {
                                return vec![effect];
                            }
                        }
                    }
                    Some(UiModalKind::TreeBrowser) => {
                        if let Some(browser) = self.state.tree_browser.as_mut() {
                            match &mut browser.phase {
                                TreePhase::Browse if browser.search_active => {
                                    browser.query.insert_text(&text);
                                    if let Some(effect) = self.refresh_tree_effect() {
                                        return vec![effect];
                                    }
                                }
                                TreePhase::EditLabel { editor, .. }
                                | TreePhase::CustomSummary { editor, .. } => {
                                    editor.insert_text(&text);
                                }
                                _ => {}
                            }
                        }
                    }
                    Some(UiModalKind::Auth) => match &mut self.state.auth_state {
                        AuthState::Selecting {
                            selected,
                            filter,
                            search_active: true,
                            ..
                        } => {
                            filter.insert_text(&text);
                            *selected = 0;
                        }
                        AuthState::Running(flow)
                            if flow
                                .prompt
                                .as_ref()
                                .is_some_and(|prompt| prompt.kind != AuthPromptKind::Select) =>
                        {
                            if let Some(prompt) = flow.prompt.as_mut() {
                                prompt.editor.insert_text(&text);
                            }
                        }
                        _ => {}
                    },
                    Some(UiModalKind::Transcript) => {
                        if let Some(viewer) = self.state.transcript_viewer.as_mut()
                            && viewer.search_active
                        {
                            viewer.search_query.insert_text(&text);
                            self.refresh_transcript_search();
                        }
                    }
                    None => {
                        self.state.editor.insert_text(&text);
                        self.state.reset_command_menu();
                        return self.refresh_file_completion().into_iter().collect();
                    }
                    _ => {}
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }
}

mod actions;
mod command_events;
mod file_references;
mod host_events;
mod input;
mod modal_input;
mod pi_events;
mod runtime_events;
mod session_flow;
mod support;
mod workflow_input;

use support::*;

#[cfg(test)]
mod tests;
