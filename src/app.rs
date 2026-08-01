use crossterm::event::{Event as TerminalEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use serde_json::Value;
use std::fmt;

use crate::{
    browser::is_safe_web_url,
    command::{CommandRoute, DiscoveredCommand, LocalCommand},
    event::{CommandEvent, RuntimeEvent},
    host::{ApprovalDecision, BootstrapStateData, SessionActivationData},
    rpc::{PiState, RpcEvent},
    selection::{next_wrapped, page_backward, page_forward, previous_wrapped},
    ui_types::{UiHitTarget, UiInputEvent, UiLayoutMetrics},
};

pub use crate::event::AppEvent;
pub use crate::state::{
    ActiveAgentSnapshot, AgentPickerState, AgentsSnapshot, AppState, ApprovalState,
    AssistantMessage, AuthChoice, AuthFlowState, AuthPromptKind, AuthPromptOption, AuthPromptState,
    AuthState, CompactionRecord, ConnectionState, ContextCategory, ContextCategoryEstimate,
    ContextConsumer, ContextPolicy, ContextPruneEstimate, ContextSnapshot, ContextUsageState,
    EditorState, GoalApprovalState, GoalSnapshot, IntegrationPromptState, PlanArtifact,
    PlanExecutionTarget, PlanQuestion, PlanReviewState, PlanStatus, PruneReason, QuestionAnswer,
    QuestionFlowState, ResourceSnapshot, RunState, SessionBrowserSnapshot, SessionBrowserState,
    SessionHistoryItem, SessionScope, SessionSortMode, SessionSummary, SubagentTranscript,
    ToolExecution, ToolStatus, TranscriptItem, TranscriptViewMode, TranscriptViewerState,
    TreeBrowserState, TreeFilterMode, TreeItem, TreePhase, TreeSnapshot, UiModalKind, UserMessage,
    UserMessageStatus, WorktreeIntegrationSnapshot, matching_auth_choice_indices,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppEffect {
    Prompt(String),
    Steer(String),
    FollowUp(String),
    Abort,
    ClearQueue,
    AbortAndClearQueue,
    Compact(Option<String>),
    GetContextState,
    GetResources,
    ReloadResources,
    SetWorkspaceTrust(bool),
    GetGoal,
    GetGoals,
    StartGoal {
        objective: Option<String>,
        from_plan: bool,
    },
    GoalAction(String),
    ApproveGoal,
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
    ExecutePlan(PlanExecutionTarget),
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

pub struct App {
    state: AppState,
    last_ui_click: Option<UiHitTarget>,
}

impl App {
    pub fn new(session: PiState) -> Self {
        Self {
            state: AppState::new(session),
            last_ui_click: None,
        }
    }

    pub fn with_commands(session: PiState, commands: Vec<DiscoveredCommand>) -> Self {
        Self {
            state: AppState::with_commands(session, commands),
            last_ui_click: None,
        }
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    pub fn set_inline_viewport_height(&mut self, height: u16) {
        self.state.layout_metrics.desired_height = height;
        self.update_selector_page_rows(height);
    }

    pub fn set_layout_metrics(&mut self, metrics: UiLayoutMetrics) {
        self.state.layout_metrics = metrics;
        self.update_selector_page_rows(metrics.body_height.max(1));
    }

    pub fn set_initial_bootstrap_state(&mut self, bootstrap: BootstrapStateData) {
        self.state.command_catalog =
            crate::command::CommandCatalog::new(bootstrap.resources.commands.clone());
        self.state.plan_mode_active = bootstrap.plan_mode.active;
        self.state.plan = bootstrap.plan.artifact;
        self.state.resources = bootstrap.resources;
        self.state.goal = Some(bootstrap.goal);
        self.state.agents = bootstrap.agents;
        self.state.context = bootstrap.context;
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
        if event.is_tick() {
            return Vec::new();
        }

        self.state.request_redraw();
        if !matches!(&event, AppEvent::UiInput(UiInputEvent::Click(_))) {
            self.last_ui_click = None;
        }
        let effects = match event {
            AppEvent::Terminal(event) => self.update_terminal(event),
            AppEvent::Pi(event) => {
                self.update_pi(event);
                Vec::new()
            }
            AppEvent::Host(event) => self.update_host(event),
            AppEvent::Command(event) => self.update_command(event),
            AppEvent::Runtime(event) => self.update_runtime(event),
            AppEvent::UiInput(event) => self.update_ui_input(event),
            AppEvent::Tick => Vec::new(),
        };
        if let Some(viewer) = self.state.transcript_viewer.as_mut()
            && viewer.follow_tail
        {
            viewer.opened_item_count = self.state.transcript.len();
        }
        effects
    }

    pub fn take_redraw_request(&mut self) -> bool {
        self.state.take_redraw_request()
    }

    fn update_terminal(&mut self, event: TerminalEvent) -> Vec<AppEffect> {
        match event {
            TerminalEvent::Key(key) if key.kind != KeyEventKind::Release => self.update_key(key),
            TerminalEvent::Paste(text) => {
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
                                TreePhase::Browse => {
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
                            selected, filter, ..
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
                    None => {
                        self.state.editor.insert_text(&text);
                        self.state.reset_command_menu();
                    }
                    _ => {}
                }
                Vec::new()
            }
            TerminalEvent::Resize(_, rows) => {
                self.state.layout_metrics.terminal_rows = rows;
                self.update_selector_page_rows(
                    rows.min(self.state.layout_metrics.desired_height.max(1)),
                );
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn update_selector_page_rows(&mut self, viewport_height: u16) {
        self.state.selector_page_rows = viewport_height
            .saturating_sub(crate::ui::SELECTOR_CHROME_HEIGHT)
            .max(1) as usize;
    }

    fn update_command(&mut self, event: CommandEvent) -> Vec<AppEffect> {
        match event {
            CommandEvent::PromptFinished(Ok(())) => {
                if let Some(TranscriptItem::User(message)) = self
                    .state
                    .transcript
                    .iter_mut()
                    .rev()
                    .find(|item| matches!(item, TranscriptItem::User(_)))
                {
                    message.status = UserMessageStatus::Accepted;
                }
            }
            CommandEvent::PromptFinished(Err(error)) => {
                self.fail_pending_user();
                self.set_pi_error(error);
            }
            CommandEvent::AbortFinished(Ok(())) => {}
            CommandEvent::AbortFinished(Err(error)) => self.set_pi_error(error),
            CommandEvent::QueueCleared(result) => match result {
                Ok(data) => {
                    if !data.restored_text.is_empty() {
                        self.state.editor.replace(data.restored_text);
                    }
                }
                Err(error) => self.set_error(format!("Unable to clear queued input: {error}")),
            },
            CommandEvent::AbortAndQueueCleared(result) => match result {
                Ok(data) => {
                    if !data.restored_text.is_empty() {
                        self.state.editor.replace(data.restored_text);
                    }
                }
                Err(error) => self.set_pi_error(error),
            },
            CommandEvent::CompactFinished(Err(error)) => {
                if !self.state.compact_lifecycle_finished {
                    self.set_pi_error(error);
                }
                self.state.compact_lifecycle_finished = false;
            }
            CommandEvent::CompactFinished(Ok(_)) => {
                self.state.compact_lifecycle_finished = false;
            }
            CommandEvent::ContextStateFinished(result) => match result {
                Ok(snapshot) => {
                    if !self.snapshot_scope_matches(snapshot.scope_id.as_deref())
                        || snapshot.revision < self.state.context.revision
                    {
                        return Vec::new();
                    }
                    self.state.context = *snapshot;
                    self.state
                        .transcript
                        .push(TranscriptItem::Context(self.state.context.clone()));
                }
                Err(error) => {
                    self.set_error(format!("Unable to inspect context: {error}"));
                }
            },
            CommandEvent::ResourcesFinished(result)
            | CommandEvent::ResourceReloadFinished(result)
            | CommandEvent::WorkspaceTrustFinished(result) => match result {
                Ok(snapshot) => {
                    if !self.snapshot_scope_matches(snapshot.scope_id.as_deref())
                        || snapshot.revision < self.state.resources.revision
                    {
                        return Vec::new();
                    }
                    self.state.command_catalog =
                        crate::command::CommandCatalog::new(snapshot.commands.clone());
                    self.state.resources = *snapshot;
                    self.state
                        .transcript
                        .push(TranscriptItem::Resources(self.state.resources.clone()));
                }
                Err(error) => self.set_error(format!("Unable to update resources: {error}")),
            },
            CommandEvent::GoalStateFinished(result) | CommandEvent::GoalActionFinished(result) => {
                match result {
                    Ok(snapshot) => {
                        self.receive_goal(*snapshot, true);
                    }
                    Err(error) => self.set_error(format!("Unable to update Goal: {error}")),
                }
            }
            CommandEvent::GoalStarted(result) => match result {
                Ok(snapshot) => {
                    self.receive_goal(*snapshot, true);
                }
                Err(error) => self.set_error(format!("Unable to start Goal: {error}")),
            },
            CommandEvent::GoalApproved(result) => match result {
                Ok(snapshot) => {
                    if self.receive_goal(*snapshot, true)
                        && self
                            .state
                            .goal
                            .as_ref()
                            .and_then(|snapshot| snapshot.goal.as_ref())
                            .is_some_and(|goal| goal.stage == "executing")
                    {
                        self.state.goal_approval = None;
                    }
                }
                Err(error) => {
                    if let Some(approval) = self.state.goal_approval.as_mut() {
                        approval.submitting = false;
                    }
                    self.set_error(format!("Unable to approve Goal: {error}"));
                }
            },
            CommandEvent::GoalsFinished(result) => match result {
                Ok(snapshot) => self.state.transcript.push(TranscriptItem::Goals(*snapshot)),
                Err(error) => self.set_error(format!("Unable to list Goals: {error}")),
            },
            CommandEvent::ModelsFinished(result) => match result {
                Ok(data) => {
                    let current = data
                        .current
                        .as_ref()
                        .and_then(|value| {
                            Some(format!(
                                "{}/{}",
                                value.get("provider")?.as_str()?,
                                value.get("id")?.as_str()?
                            ))
                        })
                        .unwrap_or_else(|| "none".to_owned());
                    let models = data
                        .models
                        .iter()
                        .map(|model| {
                            format!(
                                "{}/{}  {}  ctx {}{}",
                                model.provider,
                                model.id,
                                model.name,
                                model.context_window,
                                if model.reasoning { "  reasoning" } else { "" }
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    self.state.transcript.push(TranscriptItem::Notice(format!(
                        "Current model: {current}\n{models}\n\nUse /model provider/id to select."
                    )));
                }
                Err(error) => self.set_error(format!("Unable to list models: {error}")),
            },
            CommandEvent::ModelSetFinished(result) => match result {
                Ok(model) => {
                    self.state.session.model = Some(model);
                    self.state
                        .transcript
                        .push(TranscriptItem::Notice("Model updated.".to_owned()));
                }
                Err(error) => self.set_error(format!("Unable to set model: {error}")),
            },
            CommandEvent::ThinkingSetFinished(result) => match result {
                Ok(data) => {
                    if let Some(level) = data.get("level").and_then(Value::as_str) {
                        self.state.session.thinking_level = level.to_owned();
                    }
                    self.state.transcript.push(TranscriptItem::Notice(format!(
                        "Thinking level: {}",
                        self.state.session.thinking_level
                    )));
                }
                Err(error) => self.set_error(format!("Unable to set thinking level: {error}")),
            },
            CommandEvent::AgentsFinished(result) => match result {
                Ok(snapshot) => {
                    if !self.snapshot_scope_matches(snapshot.scope_id.as_deref())
                        || snapshot.revision < self.state.agents.revision
                    {
                        self.state.open_agent_picker_on_agents = false;
                        return Vec::new();
                    }
                    self.state.agents = *snapshot;
                    if self.state.open_agent_picker_on_agents {
                        self.state.open_agent_picker_on_agents = false;
                        self.state.agent_picker = Some(AgentPickerState::new(&self.state.agents));
                    } else {
                        self.state
                            .transcript
                            .push(TranscriptItem::Agents(self.state.agents.clone()));
                    }
                }
                Err(error) => {
                    self.state.open_agent_picker_on_agents = false;
                    self.set_error(format!("Unable to inspect agents: {error}"));
                }
            },
            CommandEvent::AgentsReloaded(result) => match result {
                Ok(snapshot) => {
                    if !self.snapshot_scope_matches(snapshot.scope_id.as_deref())
                        || snapshot.revision < self.state.agents.revision
                    {
                        return Vec::new();
                    }
                    self.state.agents = *snapshot;
                    self.state
                        .transcript
                        .push(TranscriptItem::Agents(self.state.agents.clone()));
                }
                Err(error) => self.set_error(format!("Unable to reload agents: {error}")),
            },
            CommandEvent::SubagentStarted(result) => match result {
                Ok(data) => {
                    if !self
                        .state
                        .agents
                        .active
                        .iter()
                        .any(|agent| agent.id == data.agent.id)
                    {
                        self.state.agents.active.push(data.agent);
                    }
                }
                Err(error) => self.set_error(format!("Unable to start subagent: {error}")),
            },
            CommandEvent::SubagentCancelled(Ok(())) => self.state.transcript.push(
                TranscriptItem::Notice("Subagent cancellation requested.".to_owned()),
            ),
            CommandEvent::SubagentCancelled(Err(error)) => {
                self.set_error(format!("Unable to cancel subagent: {error}"));
            }
            CommandEvent::SubagentIntegrated(result) => match result {
                Ok(value) => {
                    let status = value
                        .get("status")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("updated");
                    if matches!(status, "conflicted" | "needs_reconciliation") {
                        if let Some(prompt) = self.state.integration_prompt.as_mut() {
                            prompt.submitting = false;
                            prompt.integration.status = status.to_owned();
                        }
                    } else {
                        self.finish_current_integration_prompt();
                    }
                    self.state.transcript.push(TranscriptItem::Notice(format!(
                        "Subagent integration {status}."
                    )));
                }
                Err(error) => {
                    if let Some(prompt) = self.state.integration_prompt.as_mut() {
                        prompt.submitting = false;
                    }
                    self.set_error(format!("Unable to integrate subagent: {error}"));
                }
            },
            CommandEvent::SessionBrowserOpened(result) => match result {
                Ok(snapshot) => self.apply_session_browser_snapshot(*snapshot),
                Err(error) => {
                    self.state.session_browser = None;
                    self.set_error(format!("Unable to list sessions: {error}"));
                }
            },
            CommandEvent::SessionBrowserQueryFinished { generation, result } => {
                let current_generation = self
                    .state
                    .session_browser
                    .as_ref()
                    .map(|browser| browser.generation);
                if current_generation == Some(generation) {
                    match result {
                        Ok(snapshot) => self.apply_session_browser_snapshot(*snapshot),
                        Err(error) => {
                            if let Some(browser) = self.state.session_browser.as_mut() {
                                browser.loading = false;
                            }
                            self.set_error(format!("Unable to update session list: {error}"));
                        }
                    }
                }
            }
            CommandEvent::SessionBrowserClosed(Ok(())) => {}
            CommandEvent::SessionBrowserClosed(Err(error)) => {
                self.set_error(format!("Unable to close session browser: {error}"));
            }
            CommandEvent::NewSessionFinished(result) => match result {
                Ok(data) if data.cancelled => {
                    self.state.run_state = RunState::Idle;
                    self.state.transcript.push(TranscriptItem::Notice(
                        "New session was cancelled.".to_owned(),
                    ));
                }
                Ok(data) => {
                    let Some(activation) = data.activation else {
                        self.state.run_state = RunState::Idle;
                        self.set_error("New session response omitted activation state.".to_owned());
                        return Vec::new();
                    };
                    self.apply_activation("new session", activation);
                }
                Err(error) => {
                    self.state.run_state = RunState::Idle;
                    self.set_error(format!("Unable to create a new session: {error}"));
                }
            },
            CommandEvent::ResumeSessionFinished(result) => match result {
                Ok(data) if data.cancelled => {
                    self.state.run_state = RunState::Idle;
                    if let Some(browser) = self.state.session_browser.as_mut() {
                        browser.switching = false;
                    }
                }
                Ok(data) => {
                    let Some(activation) = data.activation else {
                        self.state.run_state = RunState::Idle;
                        self.set_error("Resume response omitted activation state.".to_owned());
                        return Vec::new();
                    };
                    self.state.session_browser = None;
                    self.apply_activation("resumed", activation);
                }
                Err(error) => {
                    self.state.run_state = RunState::Idle;
                    if let Some(browser) = self.state.session_browser.as_mut() {
                        browser.switching = false;
                    }
                    self.set_error(format!("Unable to resume session: {error}"));
                }
            },
            CommandEvent::TreeStateFinished { generation, result } => {
                let current_generation = self
                    .state
                    .tree_browser
                    .as_ref()
                    .map(|browser| browser.generation);
                if current_generation == Some(generation) {
                    match result {
                        Ok(snapshot) => self.apply_tree_snapshot(*snapshot),
                        Err(error) => {
                            if let Some(browser) = self.state.tree_browser.as_mut() {
                                browser.loading = false;
                            }
                            self.set_error(format!("Unable to load session tree: {error}"));
                        }
                    }
                }
            }
            CommandEvent::TreeLabelFinished(Ok(())) => {
                if let Some(effect) = self.refresh_tree_effect() {
                    return vec![effect];
                }
            }
            CommandEvent::TreeLabelFinished(Err(error)) => {
                if let Some(browser) = self.state.tree_browser.as_mut() {
                    browser.phase = TreePhase::Browse;
                }
                self.set_error(format!("Unable to update tree label: {error}"));
            }
            CommandEvent::TreeCopyFinished(Ok(())) => {
                self.state
                    .transcript
                    .push(TranscriptItem::Notice("Copied tree entry.".to_owned()));
            }
            CommandEvent::TreeCopyFinished(Err(error)) => {
                self.set_error(format!("Unable to copy tree entry: {error}"));
            }
            CommandEvent::TreeNavigateFinished(result) => match result {
                Ok(data) if data.cancelled => {
                    self.state.run_state = RunState::Idle;
                    if data.aborted {
                        if let Some(browser) = self.state.tree_browser.as_mut() {
                            browser.phase = TreePhase::Browse;
                        }
                        self.state.transcript.push(TranscriptItem::Notice(
                            "Branch summarization was cancelled.".to_owned(),
                        ));
                        if let Some(effect) = self.refresh_tree_effect() {
                            return vec![effect];
                        }
                    } else {
                        self.state.tree_browser = None;
                        self.state.transcript.push(TranscriptItem::Notice(
                            "Tree navigation was cancelled.".to_owned(),
                        ));
                    }
                }
                Ok(data) => {
                    let Some(activation) = data.activation else {
                        self.state.run_state = RunState::Idle;
                        self.set_error(
                            "Tree navigation response omitted activation state.".to_owned(),
                        );
                        return Vec::new();
                    };
                    let editor_text = data.editor_text;
                    self.state.tree_browser = None;
                    self.apply_activation("tree navigation", activation);
                    if let Some(text) = editor_text.filter(|text| !text.is_empty()) {
                        self.state.editor.replace(text);
                    }
                }
                Err(error) => {
                    self.state.run_state = RunState::Idle;
                    if let Some(browser) = self.state.tree_browser.as_mut() {
                        browser.phase = TreePhase::Browse;
                    }
                    self.set_error(format!("Unable to navigate session tree: {error}"));
                }
            },
            CommandEvent::TreeAbortFinished(Ok(())) => {}
            CommandEvent::TreeAbortFinished(Err(error)) => {
                self.set_error(format!("Unable to abort branch summary: {error}"));
            }
            CommandEvent::AuthProvidersFinished(Ok(providers)) => {
                let choices = providers
                    .into_iter()
                    .flat_map(|provider| {
                        provider
                            .methods
                            .into_iter()
                            .filter(|method| method.available)
                            .map(move |method| AuthChoice {
                                provider_id: provider.id.clone(),
                                provider_name: provider.name.clone(),
                                auth_type: method.kind,
                                label: method.label,
                                configured: provider.configured,
                            })
                    })
                    .collect::<Vec<_>>();
                if choices.is_empty() {
                    self.state.auth_state = AuthState::Inactive;
                    self.set_error("No providers support in-app authentication.".to_owned());
                } else {
                    self.state.auth_state = AuthState::Selecting {
                        choices,
                        selected: 0,
                        filter: EditorState::default(),
                    };
                }
            }
            CommandEvent::AuthProvidersFinished(Err(error)) => {
                self.state.auth_state = AuthState::Inactive;
                self.set_error(format!("Unable to load authentication providers: {error}"));
            }
            CommandEvent::AuthLoginFinished(Ok(result)) => {
                self.state.auth_state = AuthState::Inactive;
                self.state.run_state = RunState::Idle;
                self.state.last_error = None;
                if let Some(model) = result.selected_model {
                    self.state.session.model = Some(model);
                }
                self.state.transcript.push(TranscriptItem::Notice(format!(
                    "Authenticated {} with {}.",
                    result.provider_id, result.credential_type
                )));
            }
            CommandEvent::AuthLoginFinished(Err(error)) => {
                if !error.to_ascii_lowercase().contains("cancel") {
                    self.state.auth_state = AuthState::Inactive;
                    self.set_error(format!("Login failed: {error}"));
                }
            }
            CommandEvent::AuthReplyFinished(Ok(())) => {}
            CommandEvent::AuthReplyFinished(Err(error)) => {
                self.set_auth_error(format!("Unable to submit authentication response: {error}"));
            }
            CommandEvent::AuthCancelFinished(Ok(())) => {
                self.state.auth_state = AuthState::Inactive;
                self.state.run_state = self.run_state_after_auth();
            }
            CommandEvent::AuthCancelFinished(Err(error)) => {
                self.set_auth_error(format!("Unable to cancel login: {error}"));
            }
            CommandEvent::OpenUrlFinished(Ok(())) => {}
            CommandEvent::OpenUrlFinished(Err(error)) => {
                if let AuthState::Running(flow) = &mut self.state.auth_state {
                    flow.status = format!(
                        "Unable to open the browser: {error}. Open the link below manually."
                    );
                } else {
                    self.state.transcript.push(TranscriptItem::Notice(format!(
                        "Unable to open the browser: {error}"
                    )));
                }
            }
            CommandEvent::SetPlanModeFinished { requested, result } => {
                self.state.pending_plan_mode = None;
                match result {
                    Ok(mode) if mode.active == requested => {
                        self.state.plan_mode_active = mode.active;
                    }
                    Ok(mode) => self.set_error(format!(
                        "Host returned Plan mode active={} while active={} was requested",
                        mode.active, requested
                    )),
                    Err(error) => self.set_error(format!(
                        "Unable to {} Plan mode: {error}",
                        if requested { "enter" } else { "exit" }
                    )),
                }
            }
            CommandEvent::ApprovalReplyFinished {
                approval_id,
                decision,
                result,
            } => {
                let tool_call_id = self
                    .state
                    .approval
                    .as_ref()
                    .filter(|approval| approval.approval_id == approval_id)
                    .map(|approval| approval.tool_call_id.clone());
                if let Some(tool_call_id) = tool_call_id {
                    match result {
                        Ok(()) => {
                            self.state.approval = None;
                            if let Some(tool) = self.find_tool_mut(Some(&tool_call_id)) {
                                tool.status = match decision {
                                    ApprovalDecision::Allow | ApprovalDecision::AllowGoal => {
                                        ToolStatus::Running
                                    }
                                    ApprovalDecision::Deny => ToolStatus::Denied,
                                };
                            }
                        }
                        Err(error) => {
                            if let Some(approval) = &mut self.state.approval {
                                approval.replying = false;
                            }
                            self.set_error(format!("Unable to answer approval request: {error}"));
                        }
                    }
                }
            }
            CommandEvent::PlanStateFinished(result) => match result {
                Ok(data) => {
                    if !self.snapshot_scope_matches(data.scope_id.as_deref()) {
                        return Vec::new();
                    }
                    if let Some(artifact) = data.artifact {
                        self.receive_plan(artifact, true);
                    }
                }
                Err(error) => self.set_error(format!("Unable to restore plan state: {error}")),
            },
            CommandEvent::QuestionReplyFinished(Ok(())) => {
                self.state.question = None;
            }
            CommandEvent::QuestionReplyFinished(Err(error)) => {
                self.state.question = None;
                self.set_error(format!("Unable to submit clarification answers: {error}"));
                return vec![AppEffect::Abort];
            }
            CommandEvent::PlanExecutionFinished { target, result } => match result {
                Ok(execution) => {
                    self.state.plan_mode_active = false;
                    self.state.pending_plan_mode = None;
                    self.state.plan = Some(execution.artifact.clone());
                    self.state.plan_review = None;
                    self.state.session.session_id = execution.session_id;
                    if execution.fresh {
                        self.state.seen_compactions.clear();
                    }
                    self.state.transcript.push(TranscriptItem::Notice(format!(
                        "Executing plan {} r{} in {}.",
                        execution.artifact.id,
                        execution.artifact.revision,
                        target.label()
                    )));
                }
                Err(error) => {
                    self.state.run_state = RunState::Idle;
                    if let Some(PlanReviewState::Confirm { submitting, .. }) =
                        self.state.plan_review.as_mut()
                    {
                        *submitting = false;
                    }
                    self.set_error(format!(
                        "Unable to execute plan in {}: {error}",
                        target.label()
                    ));
                }
            },
        }
        Vec::new()
    }

    fn update_runtime(&mut self, event: RuntimeEvent) -> Vec<AppEffect> {
        match event {
            RuntimeEvent::PiStderr(line) => self
                .state
                .transcript
                .push(TranscriptItem::Notice(format!("Pi: {line}"))),
            RuntimeEvent::PiRpcError(error) => self.set_error(error),
            RuntimeEvent::PiDisconnected => {
                self.state.connection_state = ConnectionState::Disconnected;
                self.set_error("Pi process disconnected".to_owned());
            }
            RuntimeEvent::HostDisconnected => {
                self.state.approval = None;
                self.state.question = None;
                self.state.plan_review = None;
                self.state.goal_approval = None;
                self.state.session_browser = None;
                self.state.tree_browser = None;
                self.state.agent_picker = None;
                self.state.integration_prompt = None;
                self.state.integration_prompt_queue.clear();
                self.state.open_agent_picker_on_agents = false;
                self.state.pending_plan_mode = None;
                self.state.auth_state = AuthState::Inactive;
                self.set_error("Host control service disconnected".to_owned());
            }
            RuntimeEvent::TerminalError(error) => {
                let error = format!("terminal input failed: {error}");
                self.set_error(error.clone());
                return vec![AppEffect::ExitWithError(error)];
            }
            RuntimeEvent::TerminalClosed => return vec![AppEffect::Quit],
        }
        Vec::new()
    }

    fn update_key(&mut self, key: KeyEvent) -> Vec<AppEffect> {
        if let Some(modal) = self.state.active_modal_kind() {
            return match modal {
                UiModalKind::SessionBrowser => self.update_session_browser_key(key),
                UiModalKind::TreeBrowser => self.update_tree_browser_key(key),
                UiModalKind::AgentPicker => self.update_agent_picker_key(key),
                UiModalKind::Transcript => self.update_transcript_viewer_key(key),
                UiModalKind::Question => self.update_question_key(key),
                UiModalKind::Auth => self.update_auth_key(key),
                UiModalKind::Approval => self.update_approval_key(key),
                UiModalKind::Integration => self.update_integration_prompt_key(key),
                UiModalKind::GoalApproval => self.update_goal_approval_key(key),
                UiModalKind::PlanReview => self.update_plan_review_key(key),
            };
        }

        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('o' | 'O'))
        {
            self.state.transcript_viewer = Some(TranscriptViewerState::new(
                self.state.transcript_view_mode,
                &self.state.transcript,
            ));
            return Vec::new();
        }

        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c' | 'C'))
        {
            if self.state.can_abort() {
                self.state.run_state = RunState::Aborting;
                return vec![AppEffect::AbortAndClearQueue];
            }
            return vec![AppEffect::Quit];
        }

        match key.code {
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.state.editor.insert_newline();
                self.state.reset_command_menu();
                Vec::new()
            }
            KeyCode::Char('j' | 'J') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.state.editor.insert_newline();
                self.state.reset_command_menu();
                Vec::new()
            }
            KeyCode::Esc if !self.state.command_candidates().is_empty() => {
                self.state.dismiss_command_menu();
                Vec::new()
            }
            KeyCode::Esc if self.state.can_abort() => {
                self.state.run_state = RunState::Aborting;
                vec![AppEffect::AbortAndClearQueue]
            }
            KeyCode::Up
                if key.modifiers.contains(KeyModifiers::ALT)
                    && self.state.connection_state == ConnectionState::Connected =>
            {
                vec![AppEffect::ClearQueue]
            }
            KeyCode::Enter
                if self.state.session.is_streaming
                    && self.state.connection_state == ConnectionState::Connected =>
            {
                let message = self.state.editor.take();
                self.state.reset_command_menu();
                if message.trim().is_empty() {
                    return Vec::new();
                }
                self.push_user(message.clone(), UserMessageStatus::Pending);
                if key.modifiers.contains(KeyModifiers::ALT) {
                    vec![AppEffect::FollowUp(message)]
                } else {
                    vec![AppEffect::Steer(message)]
                }
            }
            KeyCode::Enter if self.state.can_submit() => {
                if self.state.command_needs_completion() {
                    if let Some(completion) = self.state.command_completion() {
                        self.state.editor.replace(completion);
                        self.state.reset_command_menu();
                    }
                    return Vec::new();
                }
                let message = self.state.editor.take();
                self.state.reset_command_menu();
                if message.trim().is_empty() {
                    return Vec::new();
                }
                self.submit(message)
            }
            KeyCode::Up if !self.state.command_candidates().is_empty() => {
                self.state.select_previous_command();
                Vec::new()
            }
            KeyCode::Down if !self.state.command_candidates().is_empty() => {
                self.state.select_next_command();
                Vec::new()
            }
            KeyCode::Tab if !self.state.command_candidates().is_empty() => {
                self.state.select_next_command();
                Vec::new()
            }
            KeyCode::BackTab if !self.state.command_candidates().is_empty() => {
                self.state.select_previous_command();
                Vec::new()
            }
            KeyCode::BackTab => self.toggle_plan_mode(!self.state.plan_mode_active),
            KeyCode::Up => {
                let width = self
                    .state
                    .layout_metrics
                    .terminal_columns
                    .saturating_sub(4)
                    .max(1) as usize;
                self.state.editor.move_up(width);
                Vec::new()
            }
            KeyCode::Down => {
                let width = self
                    .state
                    .layout_metrics
                    .terminal_columns
                    .saturating_sub(4)
                    .max(1) as usize;
                self.state.editor.move_down(width);
                Vec::new()
            }
            KeyCode::Char('n' | 'N')
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && !self.state.command_candidates().is_empty() =>
            {
                self.state.select_next_command();
                Vec::new()
            }
            KeyCode::Char('p' | 'P')
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && !self.state.command_candidates().is_empty() =>
            {
                self.state.select_previous_command();
                Vec::new()
            }
            KeyCode::Backspace => {
                self.state.editor.backspace();
                self.state.reset_command_menu();
                Vec::new()
            }
            KeyCode::Delete => {
                self.state.editor.delete();
                self.state.reset_command_menu();
                Vec::new()
            }
            KeyCode::Left => {
                self.state.editor.move_left();
                self.state.reset_command_menu();
                Vec::new()
            }
            KeyCode::Right => {
                self.state.editor.move_right();
                self.state.reset_command_menu();
                Vec::new()
            }
            KeyCode::Home => {
                self.state.editor.move_home();
                self.state.reset_command_menu();
                Vec::new()
            }
            KeyCode::End => {
                self.state.editor.move_end();
                self.state.reset_command_menu();
                Vec::new()
            }
            KeyCode::Char('u' | 'U') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.state.editor.clear();
                self.state.reset_command_menu();
                Vec::new()
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::SUPER) =>
            {
                self.state.editor.insert_char(character);
                self.state.reset_command_menu();
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn update_ui_input(&mut self, event: UiInputEvent) -> Vec<AppEffect> {
        match event {
            UiInputEvent::ScrollUp { lines } => {
                if let Some(viewer) = self.state.transcript_viewer.as_mut() {
                    viewer.follow_tail = false;
                    viewer.scroll_to_selected = false;
                    viewer.scroll_from_bottom = viewer.scroll_from_bottom.saturating_add(lines);
                    return Vec::new();
                }
                return self.repeat_surface_key(KeyCode::Up, lines);
            }
            UiInputEvent::ScrollDown { lines } => {
                if let Some(viewer) = self.state.transcript_viewer.as_mut() {
                    viewer.scroll_to_selected = false;
                    viewer.scroll_from_bottom = viewer.scroll_from_bottom.saturating_sub(lines);
                    viewer.follow_tail = viewer.scroll_from_bottom == 0;
                    return Vec::new();
                }
                return self.repeat_surface_key(KeyCode::Down, lines);
            }
            UiInputEvent::Click(UiHitTarget::CommandCandidate(index)) => {
                self.state.select_command(index);
                if let Some(completion) = self.state.command_completion() {
                    self.state.editor.replace(completion);
                    self.state.reset_command_menu();
                }
            }
            UiInputEvent::Click(UiHitTarget::ChoiceOption(index)) => {
                let target = UiHitTarget::ChoiceOption(index);
                let repeated = self.last_ui_click.as_ref() == Some(&target);
                self.last_ui_click = Some(target);
                if self.select_active_choice(index) && repeated {
                    return self.update_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                }
            }
            UiInputEvent::Click(UiHitTarget::ListRow(index)) => {
                let target = UiHitTarget::ListRow(index);
                let repeated = self.last_ui_click.as_ref() == Some(&target);
                self.last_ui_click = Some(target);
                match self.state.active_modal_kind() {
                    Some(UiModalKind::SessionBrowser) => {
                        if let Some(browser) = self.state.session_browser.as_mut()
                            && index < browser.sessions.len()
                        {
                            browser.selected = index;
                        }
                    }
                    Some(UiModalKind::TreeBrowser) => {
                        if let Some(browser) = self.state.tree_browser.as_mut()
                            && index < browser.items.len()
                        {
                            browser.selected = index;
                            browser.selected_entry_id =
                                browser.selected_item().map(|item| item.entry_id.clone());
                        }
                    }
                    Some(UiModalKind::AgentPicker) => {
                        if let Some(picker) = self.state.agent_picker.as_mut()
                            && index < picker.profiles.len()
                        {
                            picker.selected = index;
                        }
                    }
                    _ => {}
                }
                if repeated {
                    return self.update_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                }
            }
            UiInputEvent::Click(UiHitTarget::TranscriptItem(index)) => {
                let tool_id = self
                    .state
                    .transcript
                    .get(index)
                    .and_then(|item| match item {
                        TranscriptItem::Tool(tool) => Some(tool.id.clone()),
                        _ => None,
                    });
                if let Some(viewer) = self.state.transcript_viewer.as_mut() {
                    if viewer.selected_item == Some(index) {
                        if let Some(tool_id) = tool_id {
                            let default_expanded = viewer.mode == TranscriptViewMode::Verbose;
                            let expanded = viewer
                                .tool_expansion_overrides
                                .get(&tool_id)
                                .copied()
                                .unwrap_or(default_expanded);
                            viewer.tool_expansion_overrides.insert(tool_id, !expanded);
                        }
                    } else {
                        viewer.selected_item = Some(index);
                    }
                    viewer.scroll_to_selected = true;
                    viewer.follow_tail = false;
                }
            }
            UiInputEvent::Click(UiHitTarget::SurfaceBody) => {}
        }
        Vec::new()
    }

    fn select_active_choice(&mut self, index: usize) -> bool {
        match self.state.active_modal_kind() {
            Some(UiModalKind::Question) => {
                let Some(question) = self.state.question.as_mut() else {
                    return false;
                };
                if index < question.choice_count() {
                    question.selected = index;
                    return true;
                }
            }
            Some(UiModalKind::Approval) => {
                let Some(approval) = self.state.approval.as_mut() else {
                    return false;
                };
                let count = if approval.goal_id.is_some() { 3 } else { 2 };
                if index < count {
                    approval.selected = index;
                    return true;
                }
            }
            Some(UiModalKind::GoalApproval) => {
                if index < 2
                    && let Some(approval) = self.state.goal_approval.as_mut()
                {
                    approval.selected = index;
                    return true;
                }
            }
            Some(UiModalKind::PlanReview) => {
                let Some(review) = self.state.plan_review.as_mut() else {
                    return false;
                };
                match review {
                    PlanReviewState::Menu { selected } if index < 3 => {
                        *selected = index;
                        return true;
                    }
                    PlanReviewState::Confirm { selected, .. } if index < 2 => {
                        *selected = index;
                        return true;
                    }
                    _ => {}
                }
            }
            Some(UiModalKind::Integration) => {
                let Some(prompt) = self.state.integration_prompt.as_mut() else {
                    return false;
                };
                if index < 4 && (index != 1 || prompt.integration.resolver_available) {
                    prompt.selected = index;
                    return true;
                }
            }
            Some(UiModalKind::Auth) => match &mut self.state.auth_state {
                AuthState::Selecting {
                    choices,
                    selected,
                    filter,
                } => {
                    let count = matching_auth_choice_indices(choices, filter.text()).len();
                    if index < count {
                        *selected = index;
                        return true;
                    }
                }
                AuthState::Running(flow) => {
                    if let Some(prompt) = flow.prompt.as_mut()
                        && prompt.kind == AuthPromptKind::Select
                        && index < prompt.options.len()
                    {
                        prompt.selected = index;
                        return true;
                    }
                }
                _ => {}
            },
            _ => {}
        }
        false
    }

    fn repeat_surface_key(&mut self, code: KeyCode, count: usize) -> Vec<AppEffect> {
        let mut effects = Vec::new();
        for _ in 0..count {
            effects.extend(self.update_key(KeyEvent::new(code, KeyModifiers::NONE)));
        }
        effects
    }

    fn update_integration_prompt_key(&mut self, key: KeyEvent) -> Vec<AppEffect> {
        let Some(prompt) = self.state.integration_prompt.as_mut() else {
            return Vec::new();
        };
        if prompt.submitting {
            return Vec::new();
        }
        let direct_action = match key.code {
            KeyCode::Char('a' | 'A') => Some("apply"),
            KeyCode::Char('r' | 'R') => Some("resolve"),
            KeyCode::Char('k' | 'K') => Some("keep"),
            KeyCode::Char('d' | 'D') => Some("discard"),
            _ => None,
        };
        let enabled = [true, prompt.integration.resolver_available, true, true];
        let action = if let Some(action) = direct_action {
            Some(action)
        } else {
            match update_choice_navigation(key, &mut prompt.selected, &enabled) {
                ChoiceNavAction::Handled => return Vec::new(),
                ChoiceNavAction::Cancel => {
                    self.finish_current_integration_prompt();
                    return Vec::new();
                }
                ChoiceNavAction::Confirm(selected) => Some(match selected {
                    0 => "apply",
                    1 => "resolve",
                    2 => "keep",
                    _ => "discard",
                }),
                ChoiceNavAction::Unhandled => None,
            }
        };
        let Some(action) = action else {
            return Vec::new();
        };
        if action == "resolve" && !prompt.integration.resolver_available {
            return Vec::new();
        }
        prompt.submitting = true;
        vec![AppEffect::IntegrateSubagent {
            agent_id: prompt.agent.id.clone(),
            action: action.to_owned(),
        }]
    }

    fn update_agent_picker_key(&mut self, key: KeyEvent) -> Vec<AppEffect> {
        let Some(picker) = self.state.agent_picker.as_mut() else {
            return Vec::new();
        };
        match key.code {
            KeyCode::Esc => {
                self.state.agent_picker = None;
            }
            KeyCode::Up | KeyCode::BackTab => {
                if !picker.profiles.is_empty() {
                    picker.selected = previous_wrapped(picker.selected, picker.profiles.len());
                }
            }
            KeyCode::Down | KeyCode::Tab => {
                if !picker.profiles.is_empty() {
                    picker.selected = next_wrapped(picker.selected, picker.profiles.len());
                }
            }
            KeyCode::Enter => {
                let selected = picker
                    .selected_profile()
                    .map(|profile| profile.name.clone());
                self.state.agent_picker = None;
                if let Some(name) = selected {
                    self.state.editor.replace(format!("/agent {name} "));
                    self.state.reset_command_menu();
                }
            }
            _ => {}
        }
        Vec::new()
    }

    fn update_transcript_viewer_key(&mut self, key: KeyEvent) -> Vec<AppEffect> {
        if self
            .state
            .transcript_viewer
            .as_ref()
            .is_some_and(|viewer| viewer.search_active)
        {
            let mut close_search = false;
            let mut refresh = false;
            if let Some(viewer) = self.state.transcript_viewer.as_mut() {
                match key.code {
                    KeyCode::Esc => {
                        viewer.search_active = false;
                        close_search = true;
                    }
                    KeyCode::Enter => {
                        viewer.search_active = false;
                        close_search = true;
                    }
                    KeyCode::Backspace => {
                        viewer.search_query.pop();
                        refresh = true;
                    }
                    KeyCode::Char(character)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::SUPER) =>
                    {
                        viewer.search_query.push(character);
                        refresh = true;
                    }
                    _ => {}
                }
            }
            if refresh {
                self.refresh_transcript_search();
            }
            if close_search || refresh {
                return Vec::new();
            }
        }

        if matches!(key.code, KeyCode::Esc) {
            if let Some(viewer) = self.state.transcript_viewer.take() {
                self.state.transcript_view_mode = viewer.mode;
            }
            return Vec::new();
        }

        let tool_items = self
            .state
            .transcript
            .iter()
            .enumerate()
            .filter_map(|(index, item)| matches!(item, TranscriptItem::Tool(_)).then_some(index))
            .collect::<Vec<_>>();
        let selected_tool_id = self
            .state
            .transcript_viewer
            .as_ref()
            .and_then(|viewer| viewer.selected_item)
            .and_then(|index| self.state.transcript.get(index))
            .and_then(|item| match item {
                TranscriptItem::Tool(tool) => Some(tool.id.clone()),
                _ => None,
            });
        let Some(viewer) = self.state.transcript_viewer.as_mut() else {
            return Vec::new();
        };

        match key.code {
            KeyCode::Char('/') => {
                viewer.search_active = true;
                viewer.search_query.clear();
                viewer.search_matches.clear();
                viewer.current_match = None;
            }
            KeyCode::Char('n') if !viewer.search_matches.is_empty() => {
                let next = viewer
                    .current_match
                    .map_or(0, |current| (current + 1) % viewer.search_matches.len());
                viewer.current_match = Some(next);
                viewer.selected_item = Some(viewer.search_matches[next]);
                viewer.scroll_to_selected = true;
                viewer.follow_tail = false;
            }
            KeyCode::Char('N') if !viewer.search_matches.is_empty() => {
                let previous = viewer.current_match.map_or(0, |current| {
                    current
                        .checked_sub(1)
                        .unwrap_or(viewer.search_matches.len() - 1)
                });
                viewer.current_match = Some(previous);
                viewer.selected_item = Some(viewer.search_matches[previous]);
                viewer.scroll_to_selected = true;
                viewer.follow_tail = false;
            }
            KeyCode::Char('g') => {
                viewer.follow_tail = false;
                viewer.scroll_to_selected = false;
                viewer.scroll_from_bottom = usize::MAX;
            }
            KeyCode::Char('G') => {
                viewer.follow_tail = true;
                viewer.scroll_to_selected = false;
                viewer.scroll_from_bottom = 0;
            }
            KeyCode::Char('1') => viewer.mode = TranscriptViewMode::Normal,
            KeyCode::Char('2') => viewer.mode = TranscriptViewMode::Verbose,
            KeyCode::Char('3') => viewer.mode = TranscriptViewMode::Summary,
            KeyCode::Up => {
                viewer.follow_tail = false;
                viewer.scroll_to_selected = false;
                viewer.scroll_from_bottom = viewer.scroll_from_bottom.saturating_add(1);
            }
            KeyCode::Down => {
                viewer.scroll_to_selected = false;
                viewer.scroll_from_bottom = viewer.scroll_from_bottom.saturating_sub(1);
                viewer.follow_tail = viewer.scroll_from_bottom == 0;
            }
            KeyCode::PageUp => {
                viewer.follow_tail = false;
                viewer.scroll_to_selected = false;
                viewer.scroll_from_bottom = viewer
                    .scroll_from_bottom
                    .saturating_add(self.state.selector_page_rows);
            }
            KeyCode::PageDown => {
                viewer.scroll_to_selected = false;
                viewer.scroll_from_bottom = viewer
                    .scroll_from_bottom
                    .saturating_sub(self.state.selector_page_rows);
                viewer.follow_tail = viewer.scroll_from_bottom == 0;
            }
            KeyCode::Home => {
                viewer.follow_tail = false;
                viewer.scroll_to_selected = false;
                viewer.scroll_from_bottom = usize::MAX;
            }
            KeyCode::End => {
                viewer.follow_tail = true;
                viewer.scroll_to_selected = false;
                viewer.scroll_from_bottom = 0;
            }
            KeyCode::Tab | KeyCode::BackTab if !tool_items.is_empty() => {
                let current = viewer
                    .selected_item
                    .and_then(|selected| tool_items.iter().position(|index| *index == selected))
                    .unwrap_or(0);
                let next = if matches!(key.code, KeyCode::BackTab) {
                    previous_wrapped(current, tool_items.len())
                } else {
                    next_wrapped(current, tool_items.len())
                };
                viewer.selected_item = Some(tool_items[next]);
                viewer.scroll_to_selected = true;
                viewer.follow_tail = false;
            }
            KeyCode::Enter => {
                if let Some(tool_id) = selected_tool_id {
                    let default_expanded = viewer.mode == TranscriptViewMode::Verbose;
                    let expanded = viewer
                        .tool_expansion_overrides
                        .get(&tool_id)
                        .copied()
                        .unwrap_or(default_expanded);
                    viewer.tool_expansion_overrides.insert(tool_id, !expanded);
                    viewer.scroll_to_selected = true;
                }
            }
            _ => {}
        }
        Vec::new()
    }

    fn refresh_transcript_search(&mut self) {
        let query = self
            .state
            .transcript_viewer
            .as_ref()
            .map(|viewer| viewer.search_query.to_lowercase())
            .unwrap_or_default();
        let matches = if query.is_empty() {
            Vec::new()
        } else {
            self.state
                .transcript
                .iter()
                .enumerate()
                .filter_map(|(index, item)| {
                    format!("{item:?}")
                        .to_lowercase()
                        .contains(&query)
                        .then_some(index)
                })
                .collect::<Vec<_>>()
        };
        if let Some(viewer) = self.state.transcript_viewer.as_mut() {
            viewer.search_matches = matches;
            viewer.current_match = (!viewer.search_matches.is_empty()).then_some(0);
            viewer.selected_item = viewer
                .current_match
                .and_then(|current| viewer.search_matches.get(current).copied());
            viewer.scroll_to_selected = viewer.selected_item.is_some();
            if viewer.scroll_to_selected {
                viewer.follow_tail = false;
            }
        }
    }

    fn update_session_browser_key(&mut self, key: KeyEvent) -> Vec<AppEffect> {
        let page_rows = self.state.selector_page_rows;
        let Some(browser) = self.state.session_browser.as_mut() else {
            return Vec::new();
        };
        if browser.switching {
            return Vec::new();
        }

        if browser.confirm_missing_cwd.is_some() {
            match key.code {
                KeyCode::Enter | KeyCode::Char('y' | 'Y') => {
                    let session = browser
                        .confirm_missing_cwd
                        .take()
                        .expect("missing cwd confirmation existed");
                    browser.switching = true;
                    self.state.run_state = RunState::SwitchingSession;
                    return vec![AppEffect::ResumeSession {
                        session_path: session.path,
                        cwd_override: Some(browser.current_cwd.clone()),
                    }];
                }
                KeyCode::Esc | KeyCode::Char('n' | 'N') => {
                    browser.confirm_missing_cwd = None;
                }
                _ => {}
            }
            return Vec::new();
        }

        match key.code {
            KeyCode::Esc => {
                let browser_id = browser.browser_id.clone();
                self.state.session_browser = None;
                return browser_id.map_or_else(Vec::new, |browser_id| {
                    vec![AppEffect::CloseSessionBrowser { browser_id }]
                });
            }
            KeyCode::Up => {
                if !browser.sessions.is_empty() {
                    browser.selected = previous_wrapped(browser.selected, browser.sessions.len());
                }
            }
            KeyCode::Down => {
                if !browser.sessions.is_empty() {
                    if browser.selected + 1 < browser.sessions.len() {
                        browser.selected += 1;
                    } else if browser.next_offset.is_some() {
                        return self.load_more_sessions_effect().into_iter().collect();
                    } else {
                        browser.selected = 0;
                    }
                }
            }
            KeyCode::PageUp | KeyCode::Left => {
                browser.selected = page_backward(browser.selected, page_rows);
            }
            KeyCode::PageDown | KeyCode::Right => {
                if !browser.sessions.is_empty() {
                    let previous = browser.selected;
                    browser.selected =
                        page_forward(browser.selected, browser.sessions.len(), page_rows);
                    if browser.selected == previous && browser.next_offset.is_some() {
                        return self.load_more_sessions_effect().into_iter().collect();
                    }
                }
            }
            KeyCode::Home => browser.selected = 0,
            KeyCode::End => {
                browser.selected = browser.sessions.len().saturating_sub(1);
            }
            KeyCode::Tab => {
                browser.scope = match browser.scope {
                    SessionScope::Current => SessionScope::All,
                    SessionScope::All => SessionScope::Current,
                };
                browser.selected = 0;
                browser.loaded = None;
                return self.refresh_session_browser_effect().into_iter().collect();
            }
            KeyCode::Char('s' | 'S') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                browser.sort_mode = browser.sort_mode.next();
                return self.refresh_session_browser_effect().into_iter().collect();
            }
            KeyCode::Char('n' | 'N') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                browser.named_only = !browser.named_only;
                browser.selected = 0;
                return self.refresh_session_browser_effect().into_iter().collect();
            }
            KeyCode::Char('p' | 'P') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                browser.show_path = !browser.show_path;
            }
            KeyCode::Enter => {
                let Some(session) = browser.selected_session().cloned() else {
                    return Vec::new();
                };
                if session.current {
                    let browser_id = browser.browser_id.clone();
                    self.state.session_browser = None;
                    self.state.transcript.push(TranscriptItem::Notice(
                        "This session is already active.".to_owned(),
                    ));
                    return browser_id.map_or_else(Vec::new, |browser_id| {
                        vec![AppEffect::CloseSessionBrowser { browser_id }]
                    });
                }
                if !session.cwd_available {
                    browser.confirm_missing_cwd = Some(session);
                    return Vec::new();
                }
                browser.switching = true;
                self.state.run_state = RunState::SwitchingSession;
                return vec![AppEffect::ResumeSession {
                    session_path: session.path,
                    cwd_override: None,
                }];
            }
            KeyCode::Backspace => {
                browser.query.backspace();
                browser.selected = 0;
                return self.refresh_session_browser_effect().into_iter().collect();
            }
            KeyCode::Delete => {
                browser.query.delete();
                browser.selected = 0;
                return self.refresh_session_browser_effect().into_iter().collect();
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::SUPER) =>
            {
                browser.query.insert_char(character);
                browser.selected = 0;
                return self.refresh_session_browser_effect().into_iter().collect();
            }
            _ => {}
        }
        Vec::new()
    }

    fn update_tree_browser_key(&mut self, key: KeyEvent) -> Vec<AppEffect> {
        let page_rows = self.state.selector_page_rows;
        let phase = self
            .state
            .tree_browser
            .as_ref()
            .map(|browser| browser.phase.clone());
        let Some(phase) = phase else {
            return Vec::new();
        };

        match phase {
            TreePhase::EditLabel {
                entry_id,
                mut editor,
            } => {
                match key.code {
                    KeyCode::Esc => {
                        if let Some(browser) = self.state.tree_browser.as_mut() {
                            browser.phase = TreePhase::Browse;
                        }
                    }
                    KeyCode::Enter => {
                        let label = editor.text().trim().to_owned();
                        if let Some(browser) = self.state.tree_browser.as_mut() {
                            browser.phase = TreePhase::Browse;
                        }
                        return vec![AppEffect::SetTreeLabel {
                            entry_id,
                            label: (!label.is_empty()).then_some(label),
                        }];
                    }
                    KeyCode::Backspace => editor.backspace(),
                    KeyCode::Delete => editor.delete(),
                    KeyCode::Left => editor.move_left(),
                    KeyCode::Right => editor.move_right(),
                    KeyCode::Home => editor.move_home(),
                    KeyCode::End => editor.move_end(),
                    KeyCode::Char(character)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::SUPER) =>
                    {
                        editor.insert_char(character);
                    }
                    _ => {}
                }
                if let Some(browser) = self.state.tree_browser.as_mut()
                    && matches!(browser.phase, TreePhase::EditLabel { .. })
                {
                    browser.phase = TreePhase::EditLabel { entry_id, editor };
                }
                return Vec::new();
            }
            TreePhase::ChooseSummary {
                entry_id,
                mut selected,
            } => {
                match key.code {
                    KeyCode::Esc => {
                        if let Some(browser) = self.state.tree_browser.as_mut() {
                            browser.phase = TreePhase::Browse;
                        }
                        return Vec::new();
                    }
                    KeyCode::Up => selected = previous_wrapped(selected, 3),
                    KeyCode::Down | KeyCode::Tab => selected = next_wrapped(selected, 3),
                    KeyCode::Char('1' | '2' | '3') => {
                        selected = match key.code {
                            KeyCode::Char('1') => 0,
                            KeyCode::Char('2') => 1,
                            _ => 2,
                        };
                    }
                    KeyCode::Enter => {
                        if selected == 2 {
                            if let Some(browser) = self.state.tree_browser.as_mut() {
                                browser.phase = TreePhase::CustomSummary {
                                    entry_id,
                                    editor: EditorState::default(),
                                };
                            }
                            return Vec::new();
                        }
                        return self.start_tree_navigation(entry_id, selected == 1, None);
                    }
                    _ => {}
                }
                if let Some(browser) = self.state.tree_browser.as_mut() {
                    browser.phase = TreePhase::ChooseSummary { entry_id, selected };
                }
                return Vec::new();
            }
            TreePhase::CustomSummary {
                entry_id,
                mut editor,
            } => {
                match key.code {
                    KeyCode::Esc => {
                        if let Some(browser) = self.state.tree_browser.as_mut() {
                            browser.phase = TreePhase::ChooseSummary {
                                entry_id,
                                selected: 2,
                            };
                        }
                        return Vec::new();
                    }
                    KeyCode::Enter => {
                        let instructions = editor.text().trim().to_owned();
                        if instructions.is_empty() {
                            return Vec::new();
                        }
                        return self.start_tree_navigation(entry_id, true, Some(instructions));
                    }
                    KeyCode::Backspace => editor.backspace(),
                    KeyCode::Delete => editor.delete(),
                    KeyCode::Left => editor.move_left(),
                    KeyCode::Right => editor.move_right(),
                    KeyCode::Home => editor.move_home(),
                    KeyCode::End => editor.move_end(),
                    KeyCode::Char(character)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::SUPER) =>
                    {
                        editor.insert_char(character);
                    }
                    _ => {}
                }
                if let Some(browser) = self.state.tree_browser.as_mut() {
                    browser.phase = TreePhase::CustomSummary { entry_id, editor };
                }
                return Vec::new();
            }
            TreePhase::Navigating {
                entry_id,
                summarizing,
                aborting,
            } => {
                if matches!(key.code, KeyCode::Esc) && summarizing && !aborting {
                    if let Some(browser) = self.state.tree_browser.as_mut() {
                        browser.phase = TreePhase::Navigating {
                            entry_id,
                            summarizing,
                            aborting: true,
                        };
                    }
                    return vec![AppEffect::AbortTreeNavigation];
                }
                return Vec::new();
            }
            TreePhase::Browse => {}
        }

        let Some(browser) = self.state.tree_browser.as_mut() else {
            return Vec::new();
        };
        let branch_modifier = key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        match key.code {
            KeyCode::Esc => {
                if !browser.query.text().is_empty() {
                    browser.query.clear();
                    browser.selected = 0;
                    return self.refresh_tree_effect().into_iter().collect();
                }
                self.state.tree_browser = None;
            }
            KeyCode::Up => {
                if !browser.items.is_empty() {
                    browser.selected = previous_wrapped(browser.selected, browser.items.len());
                    browser.selected_entry_id =
                        browser.selected_item().map(|item| item.entry_id.clone());
                }
            }
            KeyCode::Down => {
                if !browser.items.is_empty() {
                    browser.selected = next_wrapped(browser.selected, browser.items.len());
                    browser.selected_entry_id =
                        browser.selected_item().map(|item| item.entry_id.clone());
                }
            }
            KeyCode::Left if branch_modifier => {
                let Some(item) = browser.selected_item().cloned() else {
                    return Vec::new();
                };
                if item.foldable && !browser.folded_entry_ids.contains(&item.entry_id) {
                    browser.folded_entry_ids.insert(item.entry_id);
                    return self.refresh_tree_effect().into_iter().collect();
                }
                if let Some(index) =
                    tree_branch_segment_index(&browser.items, browser.selected, false)
                {
                    browser.selected = index;
                    browser.selected_entry_id =
                        browser.selected_item().map(|item| item.entry_id.clone());
                }
            }
            KeyCode::Right if branch_modifier => {
                let Some(item) = browser.selected_item().cloned() else {
                    return Vec::new();
                };
                if browser.folded_entry_ids.remove(&item.entry_id) {
                    return self.refresh_tree_effect().into_iter().collect();
                }
                if let Some(index) =
                    tree_branch_segment_index(&browser.items, browser.selected, true)
                {
                    browser.selected = index;
                    browser.selected_entry_id =
                        browser.selected_item().map(|item| item.entry_id.clone());
                }
            }
            KeyCode::PageUp | KeyCode::Left => {
                browser.selected = page_backward(browser.selected, page_rows);
                browser.selected_entry_id =
                    browser.selected_item().map(|item| item.entry_id.clone());
            }
            KeyCode::PageDown | KeyCode::Right => {
                if !browser.items.is_empty() {
                    browser.selected =
                        page_forward(browser.selected, browser.items.len(), page_rows);
                    browser.selected_entry_id =
                        browser.selected_item().map(|item| item.entry_id.clone());
                }
            }
            KeyCode::Home => browser.selected = 0,
            KeyCode::End => browser.selected = browser.items.len().saturating_sub(1),
            KeyCode::Enter => {
                let Some(item) = browser.selected_item().cloned() else {
                    return Vec::new();
                };
                if item.is_leaf {
                    self.state.transcript.push(TranscriptItem::Notice(
                        "Already at this tree point.".to_owned(),
                    ));
                    return Vec::new();
                }
                browser.phase = TreePhase::ChooseSummary {
                    entry_id: item.entry_id,
                    selected: 0,
                };
            }
            KeyCode::Char('x' | 'X') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(entry_id) = browser.selected_item().map(|item| item.entry_id.clone()) {
                    return vec![AppEffect::CopyTreeEntry { entry_id }];
                }
            }
            KeyCode::Char('l' | 'L')
                if key.modifiers.contains(KeyModifiers::SHIFT)
                    && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                if let Some(item) = browser.selected_item().cloned() {
                    let mut editor = EditorState::default();
                    if let Some(label) = item.label {
                        editor.replace(label);
                    }
                    browser.phase = TreePhase::EditLabel {
                        entry_id: item.entry_id,
                        editor,
                    };
                }
            }
            KeyCode::Char('t' | 'T')
                if key.modifiers.contains(KeyModifiers::SHIFT)
                    && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                browser.show_label_timestamps = !browser.show_label_timestamps;
            }
            KeyCode::Char('d' | 'D') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                browser.filter_mode = TreeFilterMode::Default;
                browser.folded_entry_ids.clear();
                return self.refresh_tree_effect().into_iter().collect();
            }
            KeyCode::Char('t' | 'T') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                browser.filter_mode = if browser.filter_mode == TreeFilterMode::NoTools {
                    TreeFilterMode::Default
                } else {
                    TreeFilterMode::NoTools
                };
                browser.folded_entry_ids.clear();
                return self.refresh_tree_effect().into_iter().collect();
            }
            KeyCode::Char('u' | 'U') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                browser.filter_mode = if browser.filter_mode == TreeFilterMode::UserOnly {
                    TreeFilterMode::Default
                } else {
                    TreeFilterMode::UserOnly
                };
                browser.folded_entry_ids.clear();
                return self.refresh_tree_effect().into_iter().collect();
            }
            KeyCode::Char('l' | 'L') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                browser.filter_mode = if browser.filter_mode == TreeFilterMode::LabeledOnly {
                    TreeFilterMode::Default
                } else {
                    TreeFilterMode::LabeledOnly
                };
                browser.folded_entry_ids.clear();
                return self.refresh_tree_effect().into_iter().collect();
            }
            KeyCode::Char('a' | 'A') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                browser.filter_mode = if browser.filter_mode == TreeFilterMode::All {
                    TreeFilterMode::Default
                } else {
                    TreeFilterMode::All
                };
                browser.folded_entry_ids.clear();
                return self.refresh_tree_effect().into_iter().collect();
            }
            KeyCode::Char('o' | 'O') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                browser.filter_mode = if key.modifiers.contains(KeyModifiers::SHIFT) {
                    browser.filter_mode.previous()
                } else {
                    browser.filter_mode.next()
                };
                browser.folded_entry_ids.clear();
                return self.refresh_tree_effect().into_iter().collect();
            }
            KeyCode::Backspace => {
                browser.query.backspace();
                browser.selected = 0;
                return self.refresh_tree_effect().into_iter().collect();
            }
            KeyCode::Delete => {
                browser.query.delete();
                browser.selected = 0;
                return self.refresh_tree_effect().into_iter().collect();
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::SUPER) =>
            {
                browser.query.insert_char(character);
                browser.selected = 0;
                return self.refresh_tree_effect().into_iter().collect();
            }
            _ => {}
        }
        Vec::new()
    }

    fn refresh_session_browser_effect(&mut self) -> Option<AppEffect> {
        let browser = self.state.session_browser.as_mut()?;
        let browser_id = browser.browser_id.clone()?;
        browser.generation += 1;
        browser.loading = true;
        Some(AppEffect::QuerySessionBrowser {
            browser_id,
            scope: browser.scope,
            query: browser.query.text().to_owned(),
            sort_mode: browser.sort_mode,
            named_only: browser.named_only,
            offset: 0,
            generation: browser.generation,
        })
    }

    fn load_more_sessions_effect(&mut self) -> Option<AppEffect> {
        let browser = self.state.session_browser.as_mut()?;
        let browser_id = browser.browser_id.clone()?;
        let offset = browser.next_offset?;
        if browser.loading {
            return None;
        }
        browser.generation += 1;
        browser.loading = true;
        Some(AppEffect::QuerySessionBrowser {
            browser_id,
            scope: browser.scope,
            query: browser.query.text().to_owned(),
            sort_mode: browser.sort_mode,
            named_only: browser.named_only,
            offset,
            generation: browser.generation,
        })
    }

    fn refresh_tree_effect(&mut self) -> Option<AppEffect> {
        let browser = self.state.tree_browser.as_mut()?;
        browser.generation += 1;
        browser.loading = true;
        Some(AppEffect::GetTreeState {
            filter_mode: browser.filter_mode,
            query: browser.query.text().to_owned(),
            folded_entry_ids: browser.folded_entry_ids.iter().cloned().collect(),
            generation: browser.generation,
        })
    }

    fn start_tree_navigation(
        &mut self,
        entry_id: String,
        summarize: bool,
        custom_instructions: Option<String>,
    ) -> Vec<AppEffect> {
        if let Some(browser) = self.state.tree_browser.as_mut() {
            browser.phase = TreePhase::Navigating {
                entry_id: entry_id.clone(),
                summarizing: summarize,
                aborting: false,
            };
        }
        self.state.run_state = if summarize {
            RunState::SummarizingBranch
        } else {
            RunState::NavigatingTree
        };
        vec![AppEffect::NavigateTree {
            entry_id,
            summarize,
            custom_instructions,
        }]
    }

    fn apply_session_browser_snapshot(&mut self, snapshot: SessionBrowserSnapshot) {
        let Some(browser) = self.state.session_browser.as_mut() else {
            return;
        };
        let selected_path = browser
            .selected_session()
            .map(|session| session.path.clone());
        let previous_len = browser.sessions.len();
        let append = snapshot.offset > 0 && snapshot.offset == previous_len;
        let advance_into_page = append
            && browser.selected.saturating_add(1) >= previous_len
            && !snapshot.sessions.is_empty();
        browser.browser_id = Some(snapshot.browser_id);
        browser.current_cwd = snapshot.current_cwd;
        browser.scope = snapshot.scope;
        browser.sort_mode = snapshot.sort_mode;
        browser.named_only = snapshot.named_only;
        if append {
            browser.sessions.extend(snapshot.sessions);
        } else {
            browser.sessions = snapshot.sessions;
        }
        browser.total = snapshot.total;
        browser.next_offset = snapshot.next_offset;
        browser.truncated = snapshot.truncated;
        browser.selected = if advance_into_page {
            previous_len
        } else {
            selected_path
                .and_then(|path| {
                    browser
                        .sessions
                        .iter()
                        .position(|session| session.path == path)
                })
                .unwrap_or(0)
                .min(browser.sessions.len().saturating_sub(1))
        };
        browser.loading = false;
        browser.loaded = None;
    }

    fn apply_tree_snapshot(&mut self, snapshot: TreeSnapshot) {
        let Some(browser) = self.state.tree_browser.as_mut() else {
            return;
        };
        let selected_id = browser
            .selected_entry_id
            .clone()
            .or_else(|| snapshot.leaf_id.clone());
        browser.items = snapshot.items;
        browser.leaf_id = snapshot.leaf_id;
        browser.filter_mode = snapshot.filter_mode;
        browser.selected = selected_id
            .as_ref()
            .and_then(|entry_id| {
                browser
                    .items
                    .iter()
                    .position(|item| &item.entry_id == entry_id)
            })
            .unwrap_or_else(|| browser.items.len().saturating_sub(1));
        browser.selected_entry_id = browser.selected_item().map(|item| item.entry_id.clone());
        browser.loading = false;
        if browser.items.is_empty()
            && browser.query.text().is_empty()
            && browser.filter_mode == TreeFilterMode::Default
        {
            self.state.tree_browser = None;
            self.state.transcript.push(TranscriptItem::Notice(
                "No entries in this session.".to_owned(),
            ));
        }
    }

    fn apply_activation(&mut self, action: &str, activation: SessionActivationData) {
        let label = activation
            .state
            .session_name
            .clone()
            .unwrap_or_else(|| short_session_id(&activation.state.session_id));
        self.state.session = activation.state;
        self.state.plan_mode_active = activation.plan_mode;
        self.state.context = activation.context;
        self.state.plan = activation.plan;
        self.state.goal = Some(activation.goal);
        self.state.goal_approval = if self
            .state
            .goal
            .as_ref()
            .and_then(|snapshot| snapshot.goal.as_ref())
            .is_some_and(|goal| goal.stage == "awaiting_approval")
        {
            Some(GoalApprovalState {
                selected: 0,
                submitting: false,
            })
        } else {
            None
        };
        self.state.plan_review = self
            .state
            .plan
            .as_ref()
            .is_some_and(|plan| plan.status == PlanStatus::Submitted)
            .then_some(PlanReviewState::Menu { selected: 0 });
        self.state.seen_compactions.clear();
        self.state.compact_lifecycle_finished = false;
        self.state.run_state = RunState::Idle;
        self.state.last_error = None;
        self.state.transcript.push(TranscriptItem::SessionBoundary {
            action: action.to_owned(),
            label,
            cwd: activation.cwd,
        });
        for item in activation.history {
            self.append_history_item(item);
        }
    }

    fn append_history_item(&mut self, item: SessionHistoryItem) {
        match item {
            SessionHistoryItem::User { text } => {
                self.push_user(text, UserMessageStatus::Accepted);
            }
            SessionHistoryItem::Assistant { text, thinking } => {
                self.state
                    .transcript
                    .push(TranscriptItem::Assistant(AssistantMessage {
                        text,
                        thinking,
                        complete: true,
                    }));
            }
            SessionHistoryItem::ToolCall { id, name, args } => {
                self.state
                    .transcript
                    .push(TranscriptItem::Tool(ToolExecution {
                        id,
                        name,
                        args,
                        output: String::new(),
                        status: ToolStatus::Running,
                    }));
            }
            SessionHistoryItem::ToolResult {
                id,
                name,
                output,
                is_error,
            } => {
                if let Some(tool) = self.find_tool_mut(Some(&id)) {
                    tool.output = output;
                    tool.status = if is_error {
                        ToolStatus::Failed
                    } else {
                        ToolStatus::Succeeded
                    };
                } else {
                    self.state
                        .transcript
                        .push(TranscriptItem::Tool(ToolExecution {
                            id,
                            name,
                            args: serde_json::Value::Null,
                            output,
                            status: if is_error {
                                ToolStatus::Failed
                            } else {
                                ToolStatus::Succeeded
                            },
                        }));
                }
            }
            SessionHistoryItem::Notice { text } => {
                self.state.transcript.push(TranscriptItem::Notice(text));
            }
            SessionHistoryItem::Compaction {
                first_kept_entry_id,
                tokens_before,
                file_count,
            } => {
                let record = CompactionRecord {
                    reason: "restored".to_owned(),
                    first_kept_entry_id,
                    tokens_before,
                    estimated_tokens_after: None,
                    tokens_saved: None,
                    saved_percent: None,
                    file_count,
                    read_file_count: 0,
                    modified_file_count: 0,
                };
                self.state
                    .seen_compactions
                    .insert(record.deduplication_key());
                self.state
                    .transcript
                    .push(TranscriptItem::Compaction(record));
            }
            SessionHistoryItem::BranchSummary { summary } => {
                self.state
                    .transcript
                    .push(TranscriptItem::BranchSummary(summary));
            }
        }
    }

    fn update_question_key(&mut self, key: KeyEvent) -> Vec<AppEffect> {
        let Some(question) = self.state.question.as_mut() else {
            return Vec::new();
        };
        if question.replying {
            return Vec::new();
        }

        let interrupt = key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c' | 'C'));
        if interrupt || (matches!(key.code, KeyCode::Esc) && !question.custom_answer) {
            question.replying = true;
            self.state.run_state = RunState::Aborting;
            return vec![AppEffect::Abort];
        }

        if question.custom_answer {
            match key.code {
                KeyCode::Esc => {
                    question.custom_answer = false;
                    question.editor.clear();
                }
                KeyCode::Enter => {
                    let value = question.editor.text().trim().to_owned();
                    if value.is_empty() {
                        return Vec::new();
                    }
                    return self.answer_current_question(value, None);
                }
                KeyCode::Backspace => question.editor.backspace(),
                KeyCode::Delete => question.editor.delete(),
                KeyCode::Left => question.editor.move_left(),
                KeyCode::Right => question.editor.move_right(),
                KeyCode::Home => question.editor.move_home(),
                KeyCode::End => question.editor.move_end(),
                KeyCode::Char('u' | 'U') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    question.editor.clear();
                }
                KeyCode::Char(character)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::SUPER) =>
                {
                    question.editor.insert_char(character);
                }
                _ => {}
            }
            return Vec::new();
        }

        let enabled = vec![true; question.choice_count()];
        match update_choice_navigation(key, &mut question.selected, &enabled) {
            ChoiceNavAction::Handled => return Vec::new(),
            ChoiceNavAction::Cancel => {
                question.replying = true;
                self.state.run_state = RunState::Aborting;
                return vec![AppEffect::Abort];
            }
            ChoiceNavAction::Confirm(_) => {}
            ChoiceNavAction::Unhandled => return Vec::new(),
        }

        let Some(current) = question.current_question() else {
            return Vec::new();
        };
        if question.selected == current.options.len() {
            question.custom_answer = true;
            question.editor.clear();
            return Vec::new();
        }
        let Some(option) = current.options.get(question.selected) else {
            return Vec::new();
        };
        let value = option.label.clone();
        let option_id = option.id.clone();
        self.answer_current_question(value, Some(option_id))
    }

    fn answer_current_question(
        &mut self,
        value: String,
        option_id: Option<String>,
    ) -> Vec<AppEffect> {
        let Some(question) = self.state.question.as_mut() else {
            return Vec::new();
        };
        let Some(current) = question.current_question() else {
            return Vec::new();
        };
        question.answers.push(QuestionAnswer {
            question_id: current.id.clone(),
            value,
            option_id,
        });
        question.editor.clear();
        question.custom_answer = false;

        if question.current + 1 < question.questions.len() {
            question.current += 1;
            question.selected = 0;
            return Vec::new();
        }

        question.replying = true;
        vec![AppEffect::ReplyQuestions {
            request_id: question.request_id.clone(),
            answers: question.answers.clone(),
        }]
    }

    fn update_plan_review_key(&mut self, key: KeyEvent) -> Vec<AppEffect> {
        let Some(review) = self.state.plan_review.clone() else {
            return Vec::new();
        };
        match review {
            PlanReviewState::Menu { mut selected } => {
                match update_choice_navigation(key, &mut selected, &[true, true, true]) {
                    ChoiceNavAction::Handled => {
                        self.state.plan_review = Some(PlanReviewState::Menu { selected });
                    }
                    ChoiceNavAction::Confirm(selected) => {
                        return self.choose_plan_review(selected);
                    }
                    ChoiceNavAction::Cancel => {
                        self.state.plan_review = None;
                    }
                    ChoiceNavAction::Unhandled => {}
                }
            }
            PlanReviewState::Confirm {
                target,
                mut selected,
                submitting,
            } => {
                if submitting {
                    return Vec::new();
                }
                if matches!(key.code, KeyCode::Char('n' | 'N')) {
                    let selected = usize::from(target == PlanExecutionTarget::Fresh);
                    self.state.plan_review = Some(PlanReviewState::Menu { selected });
                    return Vec::new();
                }
                if !matches!(key.code, KeyCode::Char('y' | 'Y')) {
                    match update_choice_navigation(key, &mut selected, &[true, true]) {
                        ChoiceNavAction::Handled => {
                            self.state.plan_review = Some(PlanReviewState::Confirm {
                                target,
                                selected,
                                submitting: false,
                            });
                            return Vec::new();
                        }
                        ChoiceNavAction::Cancel | ChoiceNavAction::Confirm(1) => {
                            let selected = usize::from(target == PlanExecutionTarget::Fresh);
                            self.state.plan_review = Some(PlanReviewState::Menu { selected });
                            return Vec::new();
                        }
                        ChoiceNavAction::Confirm(0) => {}
                        ChoiceNavAction::Confirm(_) | ChoiceNavAction::Unhandled => {
                            return Vec::new();
                        }
                    }
                }
                self.state.plan_review = Some(PlanReviewState::Confirm {
                    target,
                    selected: 0,
                    submitting: true,
                });
                self.state.run_state = RunState::Submitting;
                return vec![AppEffect::ExecutePlan(target)];
            }
        }
        Vec::new()
    }

    fn update_goal_approval_key(&mut self, key: KeyEvent) -> Vec<AppEffect> {
        let Some(approval) = self.state.goal_approval.as_mut() else {
            return Vec::new();
        };
        if approval.submitting {
            return Vec::new();
        }
        if matches!(key.code, KeyCode::Char('n' | 'N')) {
            self.state.goal_approval = None;
            return Vec::new();
        }
        if matches!(key.code, KeyCode::Char('y' | 'Y')) {
            approval.selected = 0;
        } else {
            match update_choice_navigation(key, &mut approval.selected, &[true, true]) {
                ChoiceNavAction::Handled => return Vec::new(),
                ChoiceNavAction::Cancel | ChoiceNavAction::Confirm(1) => {
                    self.state.goal_approval = None;
                    return Vec::new();
                }
                ChoiceNavAction::Confirm(0) => {}
                ChoiceNavAction::Confirm(_) | ChoiceNavAction::Unhandled => return Vec::new(),
            }
        }
        approval.submitting = true;
        vec![AppEffect::ApproveGoal]
    }

    fn choose_plan_review(&mut self, selected: usize) -> Vec<AppEffect> {
        if self.state.run_state.is_busy() {
            return Vec::new();
        }
        match selected {
            0 => {
                self.state.plan_review = Some(PlanReviewState::Confirm {
                    target: PlanExecutionTarget::Current,
                    selected: 0,
                    submitting: false,
                });
            }
            1 => {
                self.state.plan_review = Some(PlanReviewState::Confirm {
                    target: PlanExecutionTarget::Fresh,
                    selected: 0,
                    submitting: false,
                });
            }
            _ => {
                self.state.plan_review = None;
                if !self.state.plan_mode_active {
                    return self.toggle_plan_mode(true);
                }
            }
        }
        Vec::new()
    }

    fn update_approval_key(&mut self, key: KeyEvent) -> Vec<AppEffect> {
        let Some(approval) = self.state.approval.as_mut() else {
            return Vec::new();
        };
        if approval.replying {
            return Vec::new();
        }

        let interrupt = matches!(key.code, KeyCode::Esc)
            || (key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('c' | 'C')));
        if interrupt {
            approval.replying = true;
            self.state.run_state = RunState::Aborting;
            return vec![AppEffect::AbortAndClearQueue];
        }
        let direct_decision = match key.code {
            KeyCode::Char('y' | 'Y') => Some(ApprovalDecision::Allow),
            KeyCode::Char('g' | 'G') if approval.goal_id.is_some() => {
                Some(ApprovalDecision::AllowGoal)
            }
            KeyCode::Char('n' | 'N') => Some(ApprovalDecision::Deny),
            _ => None,
        };
        let enabled = vec![true; if approval.goal_id.is_some() { 3 } else { 2 }];
        let decision = if let Some(decision) = direct_decision {
            Some(decision)
        } else {
            match update_choice_navigation(key, &mut approval.selected, &enabled) {
                ChoiceNavAction::Handled => return Vec::new(),
                ChoiceNavAction::Confirm(selected) => {
                    Some(match (approval.goal_id.is_some(), selected) {
                        (_, 0) => ApprovalDecision::Allow,
                        (true, 1) => ApprovalDecision::AllowGoal,
                        _ => ApprovalDecision::Deny,
                    })
                }
                ChoiceNavAction::Cancel => {
                    approval.replying = true;
                    self.state.run_state = RunState::Aborting;
                    return vec![AppEffect::AbortAndClearQueue];
                }
                ChoiceNavAction::Unhandled => None,
            }
        };
        let Some(decision) = decision else {
            return Vec::new();
        };

        approval.replying = true;
        let approval_id = approval.approval_id.clone();
        vec![AppEffect::ReplyApproval {
            approval_id,
            decision,
        }]
    }

    fn toggle_plan_mode(&mut self, active: bool) -> Vec<AppEffect> {
        if !self.state.can_toggle_plan_mode() {
            return Vec::new();
        }
        if active == self.state.plan_mode_active {
            return Vec::new();
        }

        self.state.pending_plan_mode = Some(active);
        vec![AppEffect::SetPlanMode(active)]
    }

    fn update_auth_key(&mut self, key: KeyEvent) -> Vec<AppEffect> {
        let cancel = matches!(key.code, KeyCode::Esc)
            || (key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('c' | 'C')));
        if cancel {
            if let AuthState::Running(flow) = &mut self.state.auth_state {
                let flow_id = flow.id.clone();
                flow.prompt = None;
                flow.status = "Cancelling login…".to_owned();
                return vec![AppEffect::AuthCancel { flow_id }];
            }
            self.state.auth_state = AuthState::Inactive;
            self.state.run_state = self.run_state_after_auth();
            return Vec::new();
        }

        match &mut self.state.auth_state {
            AuthState::Selecting {
                choices,
                selected,
                filter,
            } => {
                let matching = matching_auth_choice_indices(choices, filter.text());
                if is_previous_selection_key(key) {
                    if matching.is_empty() {
                        return Vec::new();
                    }
                    *selected = previous_wrapped(*selected, matching.len());
                    return Vec::new();
                }
                if is_next_selection_key(key) {
                    if matching.is_empty() {
                        return Vec::new();
                    }
                    *selected = next_wrapped(*selected, matching.len());
                    return Vec::new();
                }
                if key.code == KeyCode::Enter {
                    let Some(choice_index) = matching.get(*selected).copied() else {
                        return Vec::new();
                    };
                    let choice = choices[choice_index].clone();
                    let flow_id = format!("auth-flow-{}", self.state.next_auth_flow_id);
                    self.state.next_auth_flow_id += 1;
                    self.state.auth_state = AuthState::Running(Box::new(AuthFlowState {
                        id: flow_id.clone(),
                        provider_name: choice.provider_name,
                        status: "Starting login…".to_owned(),
                        url: None,
                        device_code: None,
                        prompt: None,
                    }));
                    return vec![AppEffect::AuthLogin {
                        flow_id,
                        provider_id: choice.provider_id,
                        auth_type: choice.auth_type,
                    }];
                }
                match key.code {
                    KeyCode::Backspace => filter.backspace(),
                    KeyCode::Delete => filter.delete(),
                    KeyCode::Left => filter.move_left(),
                    KeyCode::Right => filter.move_right(),
                    KeyCode::Home => filter.move_home(),
                    KeyCode::End => filter.move_end(),
                    KeyCode::Char('u' | 'U') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        filter.clear();
                    }
                    KeyCode::Char(character)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::SUPER) =>
                    {
                        filter.insert_char(character);
                    }
                    _ => return Vec::new(),
                }
                *selected = 0;
            }
            AuthState::Running(flow) => {
                let Some(prompt) = flow.prompt.as_mut() else {
                    return Vec::new();
                };
                if prompt.kind == AuthPromptKind::Select {
                    if prompt.options.is_empty() {
                        return Vec::new();
                    }
                    let enabled = vec![true; prompt.options.len()];
                    match update_choice_navigation(key, &mut prompt.selected, &enabled) {
                        ChoiceNavAction::Handled => return Vec::new(),
                        ChoiceNavAction::Confirm(selected) => {
                            let flow_id = flow.id.clone();
                            let prompt_id = prompt.id.clone();
                            let value = prompt.options[selected].id.clone();
                            flow.prompt = None;
                            flow.status = "Continuing authentication…".to_owned();
                            return vec![AppEffect::AuthReply {
                                flow_id,
                                prompt_id,
                                value: AuthResponse::new(value),
                            }];
                        }
                        ChoiceNavAction::Cancel => unreachable!("escape is handled before routing"),
                        ChoiceNavAction::Unhandled => {}
                    }
                    return Vec::new();
                }

                match key.code {
                    KeyCode::Enter => {
                        let value = prompt.editor.take();
                        if value.is_empty() {
                            return Vec::new();
                        }
                        let flow_id = flow.id.clone();
                        let prompt_id = prompt.id.clone();
                        flow.prompt = None;
                        flow.status = "Continuing authentication…".to_owned();
                        return vec![AppEffect::AuthReply {
                            flow_id,
                            prompt_id,
                            value: AuthResponse::new(value),
                        }];
                    }
                    KeyCode::Backspace => prompt.editor.backspace(),
                    KeyCode::Delete => prompt.editor.delete(),
                    KeyCode::Left => prompt.editor.move_left(),
                    KeyCode::Right => prompt.editor.move_right(),
                    KeyCode::Home => prompt.editor.move_home(),
                    KeyCode::End => prompt.editor.move_end(),
                    KeyCode::Char('u' | 'U') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        prompt.editor.clear();
                    }
                    KeyCode::Char(character)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::SUPER) =>
                    {
                        prompt.editor.insert_char(character);
                    }
                    _ => {}
                }
            }
            AuthState::Inactive | AuthState::LoadingProviders => {}
        }
        Vec::new()
    }

    fn update_host(&mut self, event: RpcEvent) -> Vec<AppEffect> {
        let mut effects = Vec::new();
        if event
            .payload
            .get("scopeId")
            .and_then(Value::as_str)
            .is_some_and(|scope_id| scope_id != self.state.session.session_id)
        {
            return effects;
        }
        match event.kind.as_str() {
            "approval_request" => {
                let Some(approval_id) = string_field(&event.payload, "approvalId") else {
                    return effects;
                };
                let Some(tool_call_id) = string_field(&event.payload, "toolCallId") else {
                    return effects;
                };
                let tool_name = string_field(&event.payload, "toolName")
                    .unwrap_or_else(|| "unknown".to_owned());
                let input = event.payload["input"].clone();
                if let Some(tool) = self.find_tool_mut(Some(&tool_call_id)) {
                    tool.status = ToolStatus::WaitingApproval;
                    tool.args = input.clone();
                }
                self.state.approval = Some(ApprovalState {
                    approval_id,
                    tool_call_id,
                    tool_name,
                    input,
                    agent_id: string_field(&event.payload, "agentId"),
                    agent_profile: string_field(&event.payload, "agentProfile"),
                    model: string_field(&event.payload, "model"),
                    goal_id: string_field(&event.payload, "goalId"),
                    reason: string_field(&event.payload, "reason"),
                    risk: string_field(&event.payload, "risk"),
                    selected: 0,
                    replying: false,
                });
            }
            "question_request" => {
                let Some(request_id) = string_field(&event.payload, "requestId") else {
                    return effects;
                };
                let Ok(questions) =
                    serde_json::from_value::<Vec<PlanQuestion>>(event.payload["questions"].clone())
                else {
                    self.set_error("Host sent invalid clarification questions".to_owned());
                    return effects;
                };
                if questions.is_empty() {
                    self.set_error("Host sent an empty clarification request".to_owned());
                    return effects;
                }
                self.state.question = Some(QuestionFlowState {
                    request_id,
                    questions,
                    current: 0,
                    selected: 0,
                    custom_answer: false,
                    editor: EditorState::default(),
                    answers: Vec::new(),
                    replying: false,
                });
            }
            "question_cancelled" => {
                let request_id = string_field(&event.payload, "requestId");
                if self
                    .state
                    .question
                    .as_ref()
                    .is_some_and(|question| request_id.as_deref() == Some(&question.request_id))
                {
                    self.state.question = None;
                }
            }
            "plan_ready" => {
                if let Ok(artifact) =
                    serde_json::from_value::<PlanArtifact>(event.payload["artifact"].clone())
                {
                    self.receive_plan(artifact, true);
                }
            }
            "plan_mode_state" => {
                if let Some(active) = event.payload["active"].as_bool() {
                    self.state.plan_mode_active = active;
                    self.state.pending_plan_mode = None;
                }
            }
            "plan_state" => {
                if event.payload["artifact"].is_null() {
                    self.state.plan = None;
                    self.state.plan_review = None;
                } else if let Ok(artifact) =
                    serde_json::from_value::<PlanArtifact>(event.payload["artifact"].clone())
                {
                    self.receive_plan(artifact, false);
                }
            }
            "session_list_progress" => {
                let browser_id = string_field(&event.payload, "browserId");
                let scope = string_field(&event.payload, "scope");
                if let Some(browser) = self.state.session_browser.as_mut()
                    && browser_id.as_deref() == browser.browser_id.as_deref()
                    && scope.as_deref()
                        == Some(match browser.scope {
                            SessionScope::Current => "current",
                            SessionScope::All => "all",
                        })
                {
                    let loaded = event.payload["loaded"].as_u64().unwrap_or(0);
                    let total = event.payload["total"].as_u64().unwrap_or(0);
                    browser.loaded = Some((loaded, total));
                    browser.loading = true;
                }
            }
            "plan_executing" => {
                if let Ok(artifact) =
                    serde_json::from_value::<PlanArtifact>(event.payload["artifact"].clone())
                {
                    self.state.plan = Some(artifact);
                    self.state.plan_review = None;
                }
            }
            "plan_completed" => {
                if let Ok(artifact) =
                    serde_json::from_value::<PlanArtifact>(event.payload["artifact"].clone())
                {
                    self.state.transcript.push(TranscriptItem::Notice(format!(
                        "Plan {} r{} completed.",
                        artifact.id, artifact.revision
                    )));
                    self.state.plan = Some(artifact);
                    self.state.plan_review = None;
                }
            }
            "context_budget" => {
                if let Ok(snapshot) =
                    serde_json::from_value::<ContextSnapshot>(event.payload["snapshot"].clone())
                    && self.snapshot_scope_matches(snapshot.scope_id.as_deref())
                    && snapshot.revision >= self.state.context.revision
                {
                    self.state.context = snapshot;
                }
                if let Some(warning) = string_field(&event.payload, "policyWarning") {
                    self.state.transcript.push(TranscriptItem::Notice(warning));
                }
            }
            "workspace_state" => {
                let resources =
                    serde_json::from_value::<ResourceSnapshot>(event.payload["resources"].clone());
                let agents =
                    serde_json::from_value::<AgentsSnapshot>(event.payload["agents"].clone());
                if let (Ok(resources), Ok(agents)) = (resources, agents)
                    && self.snapshot_scope_matches(resources.scope_id.as_deref())
                    && self.snapshot_scope_matches(agents.scope_id.as_deref())
                    && resources.revision >= self.state.resources.revision
                    && agents.revision >= self.state.agents.revision
                {
                    self.state.command_catalog =
                        crate::command::CommandCatalog::new(resources.commands.clone());
                    self.state.resources = resources;
                    self.state.agents = agents;
                }
            }
            "resource_state" => {
                if let Ok(snapshot) =
                    serde_json::from_value::<ResourceSnapshot>(event.payload["snapshot"].clone())
                    && self.snapshot_scope_matches(snapshot.scope_id.as_deref())
                    && snapshot.revision >= self.state.resources.revision
                {
                    self.state.command_catalog =
                        crate::command::CommandCatalog::new(snapshot.commands.clone());
                    self.state.resources = snapshot;
                }
            }
            "goal_state" => {
                if let Ok(snapshot) =
                    serde_json::from_value::<GoalSnapshot>(event.payload["snapshot"].clone())
                {
                    self.receive_goal(snapshot, false);
                }
            }
            "goal_spec_ready" => {
                if let Ok(snapshot) =
                    serde_json::from_value::<GoalSnapshot>(event.payload["snapshot"].clone())
                {
                    self.receive_goal(snapshot, true);
                }
            }
            "agents_state" => {
                if let Ok(snapshot) =
                    serde_json::from_value::<AgentsSnapshot>(event.payload["snapshot"].clone())
                    && self.snapshot_scope_matches(snapshot.scope_id.as_deref())
                    && snapshot.revision >= self.state.agents.revision
                {
                    self.state.agents = snapshot;
                }
            }
            "subagent_state" => {
                let lifecycle =
                    string_field(&event.payload, "event").unwrap_or_else(|| "updated".to_owned());
                if let Ok(agent) =
                    serde_json::from_value::<ActiveAgentSnapshot>(event.payload["agent"].clone())
                {
                    if matches!(
                        lifecycle.as_str(),
                        "queued"
                            | "preparing_isolation"
                            | "isolated"
                            | "shared"
                            | "shared_fallback"
                            | "started"
                            | "resolving"
                    ) {
                        if let Some(current) = self
                            .state
                            .agents
                            .active
                            .iter_mut()
                            .find(|current| current.id == agent.id)
                        {
                            *current = agent.clone();
                        } else {
                            self.state.agents.active.push(agent.clone());
                        }
                    } else {
                        self.state
                            .agents
                            .active
                            .retain(|current| current.id != agent.id);
                    }
                    self.state
                        .transcript
                        .push(TranscriptItem::Subagent(SubagentTranscript {
                            event: lifecycle,
                            agent,
                            result: event.payload.get("result").cloned(),
                            error: string_field(&event.payload, "error"),
                        }));
                }
            }
            "subagent_integration" => {
                let lifecycle =
                    string_field(&event.payload, "event").unwrap_or_else(|| "updated".to_owned());
                let agent =
                    serde_json::from_value::<ActiveAgentSnapshot>(event.payload["agent"].clone());
                let integration = serde_json::from_value::<WorktreeIntegrationSnapshot>(
                    event.payload["integration"].clone(),
                );
                if let (Ok(agent), Ok(integration)) = (agent, integration) {
                    if matches!(
                        lifecycle.as_str(),
                        "pending" | "conflicted" | "needs_reconciliation"
                    ) {
                        self.enqueue_integration_prompt(IntegrationPromptState {
                            agent: agent.clone(),
                            integration: integration.clone(),
                            selected: if lifecycle == "conflicted" {
                                if integration.resolver_available { 1 } else { 2 }
                            } else if lifecycle == "needs_reconciliation" {
                                2
                            } else {
                                0
                            },
                            submitting: false,
                        });
                    } else if matches!(lifecycle.as_str(), "resolving" | "applied" | "discarded") {
                        self.remove_integration_prompt(&agent.id);
                    }
                    let changed = integration.changed_paths.len();
                    let detail = string_field(&event.payload, "error")
                        .or(integration.warning.clone())
                        .unwrap_or_default();
                    self.state.transcript.push(TranscriptItem::Notice(format!(
                        "Subagent {} integration {} · {} files · {} bytes{}",
                        agent.id,
                        lifecycle,
                        changed,
                        integration.patch_bytes,
                        if detail.is_empty() {
                            String::new()
                        } else {
                            format!(" · {detail}")
                        }
                    )));
                }
            }
            "goal_review" => {
                let verdict = string_field(&event.payload["review"], "verdict")
                    .unwrap_or_else(|| "unknown".to_owned());
                self.state.transcript.push(TranscriptItem::Notice(format!(
                    "Independent Goal review: {verdict}."
                )));
            }
            "goal_error" => {
                let error = string_field(&event.payload, "error")
                    .unwrap_or_else(|| "Goal failed".to_owned());
                self.set_error(error);
            }
            "host_warning" => {
                let warning = string_field(&event.payload, "message")
                    .unwrap_or_else(|| "Host reported a recoverable warning".to_owned());
                self.state.transcript.push(TranscriptItem::Error(warning));
            }
            "plan_execution_error" => {
                if let Ok(artifact) =
                    serde_json::from_value::<PlanArtifact>(event.payload["artifact"].clone())
                {
                    self.receive_plan(artifact, true);
                }
                let error = string_field(&event.payload, "error")
                    .unwrap_or_else(|| "Plan execution failed".to_owned());
                self.set_error(error);
            }
            "approval_blocked" => {
                let tool_call_id = string_field(&event.payload, "toolCallId");
                let reason = string_field(&event.payload, "reason")
                    .unwrap_or_else(|| "Mutation was blocked by the host".to_owned());
                if let Some(tool) = self.find_tool_mut(tool_call_id.as_deref()) {
                    tool.status = ToolStatus::Denied;
                    tool.output = reason.clone();
                }
                self.state.transcript.push(TranscriptItem::Error(reason));
            }
            "auth_prompt" => {
                let Some(flow_id) = string_field(&event.payload, "flowId") else {
                    return effects;
                };
                let Some(prompt_id) = string_field(&event.payload, "promptId") else {
                    return effects;
                };
                let Some(prompt_type) = string_field(&event.payload, "promptType") else {
                    return effects;
                };
                let kind = match prompt_type.as_str() {
                    "text" => AuthPromptKind::Text,
                    "secret" => AuthPromptKind::Secret,
                    "select" => AuthPromptKind::Select,
                    "manual_code" => AuthPromptKind::ManualCode,
                    _ => return effects,
                };
                let options = event.payload["options"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|option| {
                        Some(AuthPromptOption {
                            id: string_field(option, "id")?,
                            label: string_field(option, "label")?,
                            description: string_field(option, "description"),
                        })
                    })
                    .collect();
                if let AuthState::Running(flow) = &mut self.state.auth_state
                    && flow.id == flow_id
                {
                    flow.status = "Input required".to_owned();
                    flow.prompt = Some(AuthPromptState {
                        id: prompt_id,
                        kind,
                        message: string_field(&event.payload, "message")
                            .unwrap_or_else(|| "Authentication input".to_owned()),
                        placeholder: string_field(&event.payload, "placeholder"),
                        options,
                        selected: 0,
                        editor: EditorState::default(),
                    });
                }
            }
            "auth_prompt_cancelled" => {
                let flow_id = string_field(&event.payload, "flowId");
                let prompt_id = string_field(&event.payload, "promptId");
                if let AuthState::Running(flow) = &mut self.state.auth_state
                    && flow_id.as_deref() == Some(flow.id.as_str())
                    && flow
                        .prompt
                        .as_ref()
                        .is_some_and(|prompt| prompt_id.as_deref() == Some(prompt.id.as_str()))
                {
                    flow.prompt = None;
                    flow.status = "Waiting for authentication…".to_owned();
                }
            }
            "auth_notify" => {
                let Some(flow_id) = string_field(&event.payload, "flowId") else {
                    return effects;
                };
                let notification = &event.payload["event"];
                let Some(kind) = notification["type"].as_str() else {
                    return effects;
                };
                if let AuthState::Running(flow) = &mut self.state.auth_state
                    && flow.id == flow_id
                {
                    let mut candidate_url = None;
                    match kind {
                        "auth_url" => {
                            candidate_url = string_field(notification, "url");
                            flow.status = string_field(notification, "instructions")
                                .unwrap_or_else(|| "Open the URL to continue login.".to_owned());
                        }
                        "device_code" => {
                            candidate_url = string_field(notification, "verificationUri");
                            flow.device_code = string_field(notification, "userCode");
                            flow.status = "Waiting for device authorization…".to_owned();
                        }
                        "progress" | "info" => {
                            flow.status = string_field(notification, "message")
                                .unwrap_or_else(|| "Authenticating…".to_owned());
                            if kind == "info"
                                && flow.url.is_none()
                                && let Some(url) = notification["links"]
                                    .as_array()
                                    .and_then(|links| links.first())
                                    .and_then(|link| link["url"].as_str())
                            {
                                candidate_url = Some(url.to_owned());
                            }
                        }
                        _ => {}
                    }
                    if let Some(url) = candidate_url.filter(|url| is_safe_web_url(url)) {
                        let should_open = flow.url.as_deref() != Some(url.as_str());
                        flow.url = Some(url.clone());
                        if should_open {
                            effects.push(AppEffect::OpenUrl(url));
                        }
                    }
                }
            }
            "auth_complete" => {
                if let AuthState::Running(flow) = &mut self.state.auth_state {
                    flow.prompt = None;
                    flow.status = "Authentication complete.".to_owned();
                }
            }
            "auth_protocol_error" | "host_protocol_error" => {
                let error = string_field(&event.payload, "error")
                    .unwrap_or_else(|| "Authentication protocol failed".to_owned());
                self.set_auth_error(error);
            }
            _ => {}
        }
        effects
    }

    fn update_pi(&mut self, event: RpcEvent) {
        match event.kind.as_str() {
            "agent_start" => {
                self.state.run_state = RunState::Running;
                self.state.session.is_streaming = true;
            }
            "agent_end" => {
                self.state.run_state = RunState::Idle;
                self.state.session.is_streaming = false;
                if let Some(approval) = self.state.approval.take()
                    && let Some(tool) = self.find_tool_mut(Some(&approval.tool_call_id))
                    && tool.status == ToolStatus::WaitingApproval
                {
                    tool.status = ToolStatus::Denied;
                }
                if self
                    .state
                    .question
                    .as_ref()
                    .is_some_and(|question| question.replying)
                {
                    self.state.question = None;
                }
            }
            "queue_update" => {
                let steering = event.payload["steering"].as_array().map_or(0, Vec::len);
                let follow_up = event.payload["followUp"].as_array().map_or(0, Vec::len);
                self.state.session.pending_message_count =
                    (steering.saturating_add(follow_up)) as u64;
            }
            "message_start" => {
                if event.payload["message"]["role"].as_str() == Some("assistant") {
                    self.ensure_assistant();
                }
            }
            "message_update" => self.update_message(event.payload),
            "message_end" => {
                if let Some(message) = self.last_assistant_mut() {
                    message.complete = true;
                }
            }
            "tool_execution_start" => {
                let id = string_field(&event.payload, "toolCallId")
                    .unwrap_or_else(|| format!("tool-{}", self.state.transcript.len()));
                let name = string_field(&event.payload, "toolName")
                    .unwrap_or_else(|| "unknown".to_owned());
                self.state
                    .transcript
                    .push(TranscriptItem::Tool(ToolExecution {
                        id,
                        name,
                        args: event.payload["args"].clone(),
                        output: String::new(),
                        status: ToolStatus::Running,
                    }));
            }
            "tool_execution_update" => {
                let id = string_field(&event.payload, "toolCallId");
                let output = tool_result_text(&event.payload["partialResult"]);
                if let Some(tool) = self.find_tool_mut(id.as_deref())
                    && let Some(output) = output
                {
                    tool.output = output;
                }
            }
            "tool_execution_end" => {
                let id = string_field(&event.payload, "toolCallId");
                let failed = event.payload["isError"].as_bool().unwrap_or(false);
                let output = tool_result_text(&event.payload["result"]);
                if let Some(tool) = self.find_tool_mut(id.as_deref()) {
                    if let Some(output) = output {
                        tool.output = output;
                    }
                    tool.status = if tool.status == ToolStatus::Denied {
                        ToolStatus::Denied
                    } else if failed {
                        ToolStatus::Failed
                    } else {
                        ToolStatus::Succeeded
                    };
                }
                if self
                    .state
                    .approval
                    .as_ref()
                    .is_some_and(|approval| id.as_deref() == Some(approval.tool_call_id.as_str()))
                {
                    self.state.approval = None;
                }
            }
            "compaction_start" => {
                self.state.run_state = RunState::Compacting;
                self.state.session.is_compacting = true;
                self.state.compact_lifecycle_finished = false;
            }
            "compaction_end" => {
                self.state.session.is_compacting = false;
                self.state.compact_lifecycle_finished = true;
                let reason =
                    string_field(&event.payload, "reason").unwrap_or_else(|| "unknown".to_owned());
                let aborted = event.payload["aborted"].as_bool().unwrap_or(false);
                let will_retry = event.payload["willRetry"].as_bool().unwrap_or(false);
                if aborted {
                    self.state.run_state = RunState::Idle;
                    self.state.transcript.push(TranscriptItem::Error(format!(
                        "{} compaction was aborted.",
                        compaction_reason_label(&reason)
                    )));
                } else if event.payload["result"].is_null() {
                    let error = string_field(&event.payload, "errorMessage").unwrap_or_else(|| {
                        format!("{} compaction failed.", compaction_reason_label(&reason))
                    });
                    self.set_error(error);
                } else {
                    match parse_compaction_record(&event.payload) {
                        Ok(record) => {
                            let key = record.deduplication_key();
                            if self.state.seen_compactions.insert(key) {
                                self.state
                                    .transcript
                                    .push(TranscriptItem::Compaction(record));
                            }
                            self.state.context.usage_state = ContextUsageState::Recalculating;
                            self.state.context.actual_tokens = None;
                            self.state.context.actual_percent = None;
                            self.state.run_state = if will_retry {
                                RunState::Running
                            } else {
                                RunState::Idle
                            };
                        }
                        Err(error) => self.set_error(error),
                    }
                }
            }
            "error" => {
                let message = event.payload["error"]["message"]
                    .as_str()
                    .or_else(|| event.payload["error"].as_str())
                    .unwrap_or("Pi stream error")
                    .to_owned();
                self.set_pi_error(message);
            }
            _ => {}
        }
    }

    fn update_message(&mut self, payload: serde_json::Value) {
        let update = &payload["assistantMessageEvent"];
        let Some(kind) = update["type"].as_str() else {
            return;
        };

        match kind {
            "text_delta" => {
                if let Some(delta) = update["delta"].as_str() {
                    self.ensure_assistant().text.push_str(delta);
                }
            }
            "thinking_delta" => {
                if let Some(delta) = update["delta"].as_str() {
                    self.ensure_assistant().thinking.push_str(delta);
                }
            }
            "error" => {
                let message = update["error"]["message"]
                    .as_str()
                    .or_else(|| update["error"].as_str())
                    .unwrap_or("Pi message error")
                    .to_owned();
                self.set_pi_error(message);
            }
            _ => {}
        }
    }

    fn ensure_assistant(&mut self) -> &mut AssistantMessage {
        let needs_new = !matches!(
            self.state.transcript.last(),
            Some(TranscriptItem::Assistant(message)) if !message.complete
        );
        if needs_new {
            self.state
                .transcript
                .push(TranscriptItem::Assistant(AssistantMessage::default()));
        }
        self.last_assistant_mut()
            .expect("assistant item was just inserted")
    }

    fn last_assistant_mut(&mut self) -> Option<&mut AssistantMessage> {
        self.state
            .transcript
            .iter_mut()
            .rev()
            .find_map(|item| match item {
                TranscriptItem::Assistant(message) => Some(message),
                _ => None,
            })
    }

    fn find_tool_mut(&mut self, id: Option<&str>) -> Option<&mut ToolExecution> {
        self.state
            .transcript
            .iter_mut()
            .rev()
            .find_map(|item| match item {
                TranscriptItem::Tool(tool)
                    if id.map_or(
                        matches!(
                            tool.status,
                            ToolStatus::WaitingApproval | ToolStatus::Running
                        ),
                        |id| tool.id == id,
                    ) =>
                {
                    Some(tool)
                }
                _ => None,
            })
    }

    fn fail_pending_user(&mut self) {
        if let Some(message) = self
            .state
            .transcript
            .iter_mut()
            .rev()
            .find_map(|item| match item {
                TranscriptItem::User(message) if message.status == UserMessageStatus::Pending => {
                    Some(message)
                }
                _ => None,
            })
        {
            message.status = UserMessageStatus::Failed;
        }
    }

    fn submit(&mut self, message: String) -> Vec<AppEffect> {
        match self.state.command_catalog.route(&message) {
            CommandRoute::Local(command) => return self.run_local_command(message, command),
            CommandRoute::Unknown { name, suggestions } => {
                self.push_user(message, UserMessageStatus::Failed);
                let suggestion = if suggestions.is_empty() {
                    " Use /help to list available commands.".to_owned()
                } else {
                    format!(
                        " Did you mean {}?",
                        suggestions
                            .iter()
                            .map(|name| format!("/{name}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                self.state.transcript.push(TranscriptItem::Notice(format!(
                    "Unknown command /{name}.{suggestion}"
                )));
                return Vec::new();
            }
            CommandRoute::Prompt => {}
        }

        if self.state.run_state == RunState::AuthRequired {
            self.push_user(message, UserMessageStatus::Failed);
            self.state
                .transcript
                .push(TranscriptItem::Notice(self.login_instructions()));
            return Vec::new();
        }

        self.push_user(message.clone(), UserMessageStatus::Pending);
        self.state.run_state = RunState::Submitting;
        self.state.last_error = None;
        vec![AppEffect::Prompt(message)]
    }

    fn run_local_command(&mut self, source: String, command: LocalCommand) -> Vec<AppEffect> {
        if !matches!(
            &command,
            LocalCommand::Plan(_)
                | LocalCommand::Compact(_)
                | LocalCommand::Context
                | LocalCommand::Resources
                | LocalCommand::Reload
                | LocalCommand::Trust(_)
                | LocalCommand::Goal(_)
                | LocalCommand::Goals
                | LocalCommand::Model(_)
                | LocalCommand::Thinking(_)
                | LocalCommand::Agents(_)
                | LocalCommand::Agent(_)
                | LocalCommand::New(_)
                | LocalCommand::Resume(_)
                | LocalCommand::Tree(_)
        ) {
            self.push_user(source, UserMessageStatus::Accepted);
        }
        match command {
            LocalCommand::Login => {
                self.state.auth_state = AuthState::LoadingProviders;
                self.state.run_state = RunState::Authenticating;
                vec![AppEffect::AuthList]
            }
            LocalCommand::New(argument) => {
                if argument.is_some() {
                    self.state
                        .transcript
                        .push(TranscriptItem::Error("Usage: /new".to_owned()));
                    return Vec::new();
                }
                self.state.run_state = RunState::SwitchingSession;
                self.state.last_error = None;
                vec![AppEffect::NewSession]
            }
            LocalCommand::Resume(argument) => {
                if argument.is_some() {
                    self.state
                        .transcript
                        .push(TranscriptItem::Error("Usage: /resume".to_owned()));
                    return Vec::new();
                }
                self.state.session_browser = Some(SessionBrowserState::loading());
                self.state.last_error = None;
                vec![AppEffect::OpenSessionBrowser]
            }
            LocalCommand::Tree(argument) => {
                if argument.is_some() {
                    self.state
                        .transcript
                        .push(TranscriptItem::Error("Usage: /tree".to_owned()));
                    return Vec::new();
                }
                self.state.tree_browser = Some(TreeBrowserState::loading());
                self.state.last_error = None;
                vec![AppEffect::GetTreeState {
                    filter_mode: TreeFilterMode::Default,
                    query: String::new(),
                    folded_entry_ids: Vec::new(),
                    generation: 0,
                }]
            }
            LocalCommand::Plan(argument) => match argument.as_deref() {
                None => {
                    if self
                        .state
                        .plan
                        .as_ref()
                        .is_some_and(|plan| plan.status == PlanStatus::Submitted)
                    {
                        self.state.plan_review = Some(PlanReviewState::Menu { selected: 0 });
                    } else if !self.state.plan_mode_active {
                        return self.toggle_plan_mode(true);
                    } else {
                        self.state.transcript.push(TranscriptItem::Notice(
                            "Plan mode is active; no submitted Plan is awaiting review.".to_owned(),
                        ));
                    }
                    Vec::new()
                }
                Some("exit") => self.toggle_plan_mode(false),
                Some("status") => {
                    self.state.transcript.push(TranscriptItem::Notice(format!(
                        "Plan mode is {}.",
                        if self.state.plan_mode_active {
                            "active"
                        } else {
                            "inactive"
                        }
                    )));
                    Vec::new()
                }
                Some("run" | "run current") => {
                    if self
                        .state
                        .plan
                        .as_ref()
                        .is_some_and(|plan| plan.status == PlanStatus::Submitted)
                    {
                        self.state.plan_review = Some(PlanReviewState::Confirm {
                            target: PlanExecutionTarget::Current,
                            selected: 0,
                            submitting: false,
                        });
                    } else {
                        self.state.transcript.push(TranscriptItem::Error(
                            "No submitted Plan is available to run.".to_owned(),
                        ));
                    }
                    Vec::new()
                }
                Some("run fresh") => {
                    if self
                        .state
                        .plan
                        .as_ref()
                        .is_some_and(|plan| plan.status == PlanStatus::Submitted)
                    {
                        self.state.plan_review = Some(PlanReviewState::Confirm {
                            target: PlanExecutionTarget::Fresh,
                            selected: 0,
                            submitting: false,
                        });
                    } else {
                        self.state.transcript.push(TranscriptItem::Error(
                            "No submitted Plan is available to run.".to_owned(),
                        ));
                    }
                    Vec::new()
                }
                Some(_) => {
                    self.state.transcript.push(TranscriptItem::Error(
                        "Usage: /plan [exit|status|run [current|fresh]]".to_owned(),
                    ));
                    Vec::new()
                }
            },
            LocalCommand::Compact(instructions) => {
                self.state.run_state = RunState::Compacting;
                self.state.last_error = None;
                self.state.compact_lifecycle_finished = false;
                vec![AppEffect::Compact(instructions)]
            }
            LocalCommand::Context => vec![AppEffect::GetContextState],
            LocalCommand::Resources => vec![AppEffect::GetResources],
            LocalCommand::Reload => vec![AppEffect::ReloadResources],
            LocalCommand::Trust(argument) => {
                match argument.as_deref().map(str::to_ascii_lowercase).as_deref() {
                    None | Some("status") => vec![AppEffect::GetResources],
                    Some("on" | "true" | "yes") => vec![AppEffect::SetWorkspaceTrust(true)],
                    Some("off" | "false" | "no") => vec![AppEffect::SetWorkspaceTrust(false)],
                    _ => {
                        self.state.transcript.push(TranscriptItem::Error(
                            "Usage: /trust [on|off|status]".to_owned(),
                        ));
                        Vec::new()
                    }
                }
            }
            LocalCommand::Goal(argument) => match argument.as_deref() {
                None => vec![AppEffect::GetGoal],
                Some("pause" | "resume" | "cancel") => {
                    vec![AppEffect::GoalAction(argument.expect("matched argument"))]
                }
                Some("approve") => vec![AppEffect::ApproveGoal],
                Some("from-plan") => vec![AppEffect::StartGoal {
                    objective: None,
                    from_plan: true,
                }],
                Some(objective) => vec![AppEffect::StartGoal {
                    objective: Some(objective.to_owned()),
                    from_plan: false,
                }],
            },
            LocalCommand::Goals => vec![AppEffect::GetGoals],
            LocalCommand::Model(argument) => {
                let Some(reference) = argument else {
                    return vec![AppEffect::ListModels];
                };
                let Some((provider, model_id)) = reference.split_once('/') else {
                    self.state.transcript.push(TranscriptItem::Error(
                        "Usage: /model [provider/model]".to_owned(),
                    ));
                    return Vec::new();
                };
                if provider.is_empty() || model_id.is_empty() {
                    self.state.transcript.push(TranscriptItem::Error(
                        "Usage: /model [provider/model]".to_owned(),
                    ));
                    return Vec::new();
                }
                vec![AppEffect::SetModel {
                    provider: provider.to_owned(),
                    model_id: model_id.to_owned(),
                }]
            }
            LocalCommand::Thinking(argument) => {
                let Some(level) = argument else {
                    self.state.transcript.push(TranscriptItem::Notice(format!(
                        "Thinking level: {}",
                        self.state.session.thinking_level
                    )));
                    return Vec::new();
                };
                vec![AppEffect::SetThinking(level)]
            }
            LocalCommand::Agents(argument) => {
                let Some(argument) = argument else {
                    return vec![AppEffect::GetAgents];
                };
                if argument == "reload" {
                    return vec![AppEffect::ReloadAgents];
                }
                if let Some((action, agent_id)) = argument.split_once(' ')
                    && matches!(action, "apply" | "resolve" | "keep" | "discard")
                    && !agent_id.trim().is_empty()
                {
                    return vec![AppEffect::IntegrateSubagent {
                        agent_id: agent_id.trim().to_owned(),
                        action: action.to_owned(),
                    }];
                }
                let Some(agent_id) = argument.strip_prefix("cancel ").map(str::trim) else {
                    self.state.transcript.push(TranscriptItem::Error(
                        "Usage: /agents [reload|cancel|apply|resolve|keep|discard <agent-id>]"
                            .to_owned(),
                    ));
                    return Vec::new();
                };
                if agent_id.is_empty() {
                    self.state.transcript.push(TranscriptItem::Error(
                        "Usage: /agents [reload|cancel|apply|resolve|keep|discard <agent-id>]"
                            .to_owned(),
                    ));
                    return Vec::new();
                }
                vec![AppEffect::CancelSubagent(agent_id.to_owned())]
            }
            LocalCommand::Agent(argument) => {
                let Some(argument) = argument else {
                    if self.state.agents.profiles.is_empty() {
                        self.state.open_agent_picker_on_agents = true;
                        return vec![AppEffect::GetAgents];
                    }
                    self.state.agent_picker = Some(AgentPickerState::new(&self.state.agents));
                    return Vec::new();
                };
                let Some((profile, task)) = argument.split_once(char::is_whitespace) else {
                    self.state.transcript.push(TranscriptItem::Error(
                        "Usage: /agent <name> <task>".to_owned(),
                    ));
                    return Vec::new();
                };
                let task = task.trim();
                if profile.is_empty() || task.is_empty() {
                    self.state.transcript.push(TranscriptItem::Error(
                        "Usage: /agent <name> <task>".to_owned(),
                    ));
                    return Vec::new();
                }
                vec![AppEffect::StartSubagent {
                    profile: profile.to_owned(),
                    task: task.to_owned(),
                }]
            }
            LocalCommand::Help => {
                let help = self.state.command_catalog.help_text();
                self.state.transcript.push(TranscriptItem::Notice(help));
                Vec::new()
            }
        }
    }

    fn push_user(&mut self, text: String, status: UserMessageStatus) {
        self.state
            .transcript
            .push(TranscriptItem::User(UserMessage { text, status }));
    }

    fn receive_goal(&mut self, snapshot: GoalSnapshot, show: bool) -> bool {
        if !self.snapshot_scope_matches(snapshot.scope_id.as_deref()) {
            return false;
        }
        let stale = self
            .state
            .goal
            .as_ref()
            .and_then(|current| current.goal.as_ref())
            .zip(snapshot.goal.as_ref())
            .is_some_and(|(current, incoming)| {
                (current.id == incoming.id
                    && (current.revision > incoming.revision
                        || (current.revision == incoming.revision
                            && current.updated_at > incoming.updated_at)))
                    || (current.id != incoming.id && current.updated_at > incoming.updated_at)
            });
        if stale {
            return false;
        }
        let previous_approval = self.state.goal_approval.take();
        self.state.goal_approval = snapshot
            .goal
            .as_ref()
            .is_some_and(|goal| goal.stage == "awaiting_approval")
            .then(|| {
                previous_approval.unwrap_or(GoalApprovalState {
                    selected: 0,
                    submitting: false,
                })
            });
        self.state.goal = Some(snapshot.clone());
        if show {
            self.state
                .transcript
                .push(TranscriptItem::Goal(Box::new(snapshot)));
        }
        true
    }

    fn receive_plan(&mut self, artifact: PlanArtifact, show_review: bool) {
        let stale = self.state.plan.as_ref().is_some_and(|current| {
            (current.id == artifact.id
                && (current.revision > artifact.revision
                    || (current.revision == artifact.revision
                        && current.updated_at > artifact.updated_at)))
                || (current.id != artifact.id && current.updated_at > artifact.updated_at)
        });
        if stale {
            return;
        }
        let already_rendered = self.state.transcript.iter().any(|item| {
            matches!(
                item,
                TranscriptItem::Plan(existing)
                    if existing.id == artifact.id && existing.revision == artifact.revision
            )
        });
        if !already_rendered {
            self.state
                .transcript
                .push(TranscriptItem::Plan(artifact.clone()));
        }
        let ready = artifact.status == PlanStatus::Submitted;
        self.state.plan = Some(artifact);
        if show_review && ready {
            self.state.plan_review = Some(PlanReviewState::Menu { selected: 0 });
        }
    }

    fn enqueue_integration_prompt(&mut self, prompt: IntegrationPromptState) {
        if let Some(current) = self.state.integration_prompt.as_mut()
            && current.agent.id == prompt.agent.id
        {
            *current = prompt;
            return;
        }
        if let Some(existing) = self
            .state
            .integration_prompt_queue
            .iter_mut()
            .find(|existing| existing.agent.id == prompt.agent.id)
        {
            *existing = prompt;
            return;
        }
        if self.state.integration_prompt.is_none() {
            self.state.integration_prompt = Some(prompt);
        } else {
            self.state.integration_prompt_queue.push_back(prompt);
        }
    }

    fn finish_current_integration_prompt(&mut self) {
        self.state.integration_prompt = self.state.integration_prompt_queue.pop_front();
    }

    fn remove_integration_prompt(&mut self, agent_id: &str) {
        if self
            .state
            .integration_prompt
            .as_ref()
            .is_some_and(|prompt| prompt.agent.id == agent_id)
        {
            self.finish_current_integration_prompt();
        } else {
            self.state
                .integration_prompt_queue
                .retain(|prompt| prompt.agent.id != agent_id);
        }
    }

    fn snapshot_scope_matches(&self, scope_id: Option<&str>) -> bool {
        scope_id.is_none_or(|scope_id| scope_id == self.state.session.session_id)
    }

    fn set_pi_error(&mut self, error: String) {
        if is_missing_credentials(&error) {
            self.state.run_state = RunState::AuthRequired;
            self.state.last_error = Some(error);
            self.state.transcript.push(TranscriptItem::Error(
                "No credentials are configured for the selected model.".to_owned(),
            ));
            self.state
                .transcript
                .push(TranscriptItem::Notice(self.login_instructions()));
        } else {
            self.set_error(error);
        }
    }

    fn login_instructions(&self) -> String {
        "Use /login to authenticate inside Nabla.".to_owned()
    }

    fn set_auth_error(&mut self, error: String) {
        if let AuthState::Running(flow) = &mut self.state.auth_state {
            flow.prompt = None;
            flow.status = error;
        } else {
            self.set_error(error);
        }
    }

    fn run_state_after_auth(&self) -> RunState {
        if self
            .state
            .last_error
            .as_deref()
            .is_some_and(is_missing_credentials)
        {
            RunState::AuthRequired
        } else {
            RunState::Idle
        }
    }

    fn set_error(&mut self, error: String) {
        self.state.run_state = RunState::Error;
        self.state.last_error = Some(error.clone());
        self.state.transcript.push(TranscriptItem::Error(error));
    }
}

fn is_previous_selection_key(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Up | KeyCode::BackTab)
        || (key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('p' | 'P')))
}

fn is_next_selection_key(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Down | KeyCode::Tab)
        || (key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('n' | 'N')))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChoiceNavAction {
    Handled,
    Confirm(usize),
    Cancel,
    Unhandled,
}

fn update_choice_navigation(
    key: KeyEvent,
    selected: &mut usize,
    enabled: &[bool],
) -> ChoiceNavAction {
    if matches!(key.code, KeyCode::Esc) {
        return ChoiceNavAction::Cancel;
    }
    if enabled.is_empty() || !enabled.iter().any(|enabled| *enabled) {
        return ChoiceNavAction::Unhandled;
    }
    if is_previous_selection_key(key) {
        *selected = next_enabled_choice(*selected, enabled, false);
        return ChoiceNavAction::Handled;
    }
    if is_next_selection_key(key) {
        *selected = next_enabled_choice(*selected, enabled, true);
        return ChoiceNavAction::Handled;
    }
    if let KeyCode::Char(character @ '1'..='9') = key.code
        && key.modifiers.is_empty()
    {
        let index = character.to_digit(10).unwrap_or(1) as usize - 1;
        if enabled.get(index).copied().unwrap_or(false) {
            *selected = index;
        }
        return ChoiceNavAction::Handled;
    }
    if key.code == KeyCode::Enter && enabled.get(*selected).copied().unwrap_or(false) {
        return ChoiceNavAction::Confirm(*selected);
    }
    ChoiceNavAction::Unhandled
}

fn next_enabled_choice(selected: usize, enabled: &[bool], forward: bool) -> usize {
    let mut index = selected.min(enabled.len().saturating_sub(1));
    for _ in 0..enabled.len() {
        index = if forward {
            next_wrapped(index, enabled.len())
        } else {
            previous_wrapped(index, enabled.len())
        };
        if enabled[index] {
            return index;
        }
    }
    selected
}

fn is_missing_credentials(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("no api key found")
        || error.contains("no credentials")
        || (error.contains("/login") && error.contains("api key"))
}

fn string_field(value: &serde_json::Value, name: &str) -> Option<String> {
    value.get(name)?.as_str().map(ToOwned::to_owned)
}

fn short_session_id(session_id: &str) -> String {
    session_id.chars().take(8).collect()
}

fn tree_branch_segment_index(items: &[TreeItem], selected: usize, down: bool) -> Option<usize> {
    let index_by_id = items
        .iter()
        .enumerate()
        .map(|(index, item)| (item.entry_id.as_str(), index))
        .collect::<std::collections::HashMap<_, _>>();
    let mut current_id = items.get(selected)?.entry_id.as_str();

    if down {
        loop {
            let children = items
                .iter()
                .enumerate()
                .filter(|(_, item)| item.parent_id.as_deref() == Some(current_id))
                .collect::<Vec<_>>();
            match children.as_slice() {
                [] => return index_by_id.get(current_id).copied(),
                [(index, _)] => current_id = items[*index].entry_id.as_str(),
                [(index, _), ..] => return Some(*index),
            }
        }
    }

    loop {
        let current = items.get(*index_by_id.get(current_id)?)?;
        let Some(parent_id) = current.parent_id.as_deref() else {
            return index_by_id.get(current_id).copied();
        };
        let sibling_count = items
            .iter()
            .filter(|item| item.parent_id.as_deref() == Some(parent_id))
            .count();
        let current_index = *index_by_id.get(current_id)?;
        if sibling_count > 1 && current_index < selected {
            return Some(current_index);
        }
        current_id = parent_id;
    }
}

fn tool_result_text(value: &serde_json::Value) -> Option<String> {
    let content = value.get("content")?.as_array()?;
    let text = content
        .iter()
        .filter(|part| part.get("type").and_then(serde_json::Value::as_str) == Some("text"))
        .filter_map(|part| part.get("text").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty()).then_some(text)
}

fn parse_compaction_record(payload: &serde_json::Value) -> Result<CompactionRecord, String> {
    let reason = payload["reason"]
        .as_str()
        .ok_or_else(|| "Compaction event has no reason.".to_owned())?
        .to_owned();
    let result = payload["result"]
        .as_object()
        .ok_or_else(|| "Compaction completed without a result.".to_owned())?;
    let first_kept_entry_id = result
        .get("firstKeptEntryId")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "Compaction result has no first kept entry.".to_owned())?
        .to_owned();
    let tokens_before = result
        .get("tokensBefore")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "Compaction result has no token count.".to_owned())?;
    let estimated_tokens_after = result
        .get("estimatedTokensAfter")
        .and_then(serde_json::Value::as_u64);
    let tokens_saved = estimated_tokens_after.map(|after| tokens_before.saturating_sub(after));
    let saved_percent = tokens_saved.and_then(|saved| {
        (tokens_before > 0).then_some((saved as f64 / tokens_before as f64) * 100.0)
    });
    let details = result.get("details").and_then(serde_json::Value::as_object);
    let read_files = detail_files(details, "readFiles");
    let modified_files = detail_files(details, "modifiedFiles");
    let file_count = read_files
        .iter()
        .chain(&modified_files)
        .collect::<std::collections::HashSet<_>>()
        .len() as u64;

    Ok(CompactionRecord {
        reason,
        first_kept_entry_id,
        tokens_before,
        estimated_tokens_after,
        tokens_saved,
        saved_percent,
        file_count,
        read_file_count: read_files.len() as u64,
        modified_file_count: modified_files.len() as u64,
    })
}

fn detail_files(
    details: Option<&serde_json::Map<String, serde_json::Value>>,
    field: &str,
) -> Vec<String> {
    details
        .and_then(|details| details.get(field))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .collect()
}

fn compaction_reason_label(reason: &str) -> &'static str {
    match reason {
        "manual" => "Manual",
        "threshold" => "Threshold",
        "overflow" => "Overflow recovery",
        _ => "Context",
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyEvent, KeyModifiers};
    use serde_json::json;

    use super::*;
    use crate::host::{
        AuthLoginData, AuthMethod, AuthProvider, HostPlanModeData, PlanExecutionData,
        QueueClearData, SessionCommandData, TreeNavigateData,
    };

    fn state() -> PiState {
        PiState {
            model: Some(json!({"provider": "test", "name": "fake"})),
            thinking_level: "off".to_owned(),
            is_streaming: false,
            is_compacting: false,
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

    fn press(code: KeyCode) -> AppEvent {
        AppEvent::Terminal(TerminalEvent::Key(KeyEvent::new(code, KeyModifiers::NONE)))
    }

    fn press_with(code: KeyCode, modifiers: KeyModifiers) -> AppEvent {
        AppEvent::Terminal(TerminalEvent::Key(KeyEvent::new(code, modifiers)))
    }

    fn plan(status: PlanStatus) -> PlanArtifact {
        PlanArtifact {
            schema_version: 2,
            id: "plan-1".to_owned(),
            revision: 2,
            status,
            title: "Structured planning".to_owned(),
            summary: "Make plans first-class.".to_owned(),
            body_markdown: "1. Ask questions.\n2. Submit a plan.".to_owned(),
            assumptions: vec!["Single-select questions".to_owned()],
            test_plan: vec!["Run cargo test".to_owned()],
            source_session_id: "session-1".to_owned(),
            created_at: "2026-01-01T00:00:00.000Z".to_owned(),
            updated_at: "2026-01-01T00:00:01.000Z".to_owned(),
            last_execution_error: None,
        }
    }

    fn session_summary(path: &str, id: &str, current: bool, cwd_available: bool) -> SessionSummary {
        SessionSummary {
            path: path.to_owned(),
            id: id.to_owned(),
            cwd: format!("/workspace/{id}"),
            cwd_available,
            name: Some(format!("Session {id}")),
            parent_session_path: None,
            created_at: "2026-01-01T00:00:00.000Z".to_owned(),
            modified_at: "2026-01-01T00:00:01.000Z".to_owned(),
            message_count: 2,
            first_message: format!("first message in {id}"),
            depth: 0,
            is_last: false,
            current,
        }
    }

    fn tree_item(
        entry_id: &str,
        parent_id: Option<&str>,
        is_leaf: bool,
        foldable: bool,
    ) -> TreeItem {
        TreeItem {
            entry_id: entry_id.to_owned(),
            parent_id: parent_id.map(ToOwned::to_owned),
            kind: "message".to_owned(),
            role: Some("user".to_owned()),
            preview: format!("user: {entry_id}"),
            label: None,
            label_timestamp: None,
            visual_depth: usize::from(parent_id.is_some()),
            show_connector: parent_id.is_some(),
            gutter_positions: Vec::new(),
            is_last: true,
            is_active_path: true,
            is_leaf,
            foldable,
            folded: false,
        }
    }

    fn activation(session_id: &str) -> SessionActivationData {
        let mut pi_state = state();
        pi_state.session_id = session_id.to_owned();
        pi_state.session_name = Some("Restored work".to_owned());
        pi_state.session_file = Some(format!("/sessions/{session_id}.jsonl"));
        pi_state.message_count = 4;
        SessionActivationData {
            state: pi_state,
            cwd: "/workspace/restored".to_owned(),
            plan_mode: false,
            goal: GoalSnapshot {
                scope_id: Some(session_id.to_owned()),
                goal: None,
                state_path: "/state/session.json".to_owned(),
            },
            history: vec![
                SessionHistoryItem::User {
                    text: "restored question".to_owned(),
                },
                SessionHistoryItem::Assistant {
                    text: "restored answer".to_owned(),
                    thinking: "restored reasoning".to_owned(),
                },
                SessionHistoryItem::ToolCall {
                    id: "tool-restored".to_owned(),
                    name: "read".to_owned(),
                    args: json!({"path": "src/lib.rs"}),
                },
                SessionHistoryItem::ToolResult {
                    id: "tool-restored".to_owned(),
                    name: "read".to_owned(),
                    output: "restored source".to_owned(),
                    is_error: false,
                },
                SessionHistoryItem::Compaction {
                    first_kept_entry_id: "entry-kept".to_owned(),
                    tokens_before: 82_000,
                    file_count: 3,
                },
                SessionHistoryItem::BranchSummary {
                    summary: "restored branch summary".to_owned(),
                },
            ],
            plan: Some(plan(PlanStatus::Executing)),
            context: ContextSnapshot {
                usage_state: ContextUsageState::Recalculating,
                epoch: 4,
                ..ContextSnapshot::default()
            },
        }
    }

    #[test]
    fn clarification_questions_are_answered_sequentially_with_custom_input() {
        let mut app = App::new(state());
        app.state.run_state = RunState::Running;
        app.update(AppEvent::Host(RpcEvent {
            kind: "question_request".to_owned(),
            payload: json!({
                "requestId": "question-1",
                "questions": [
                    {
                        "id": "scope",
                        "prompt": "Which scope?",
                        "options": [
                            {"id": "small", "label": "Small"},
                            {"id": "complete", "label": "Complete"}
                        ]
                    },
                    {
                        "id": "compat",
                        "prompt": "Compatibility target?",
                        "options": [
                            {"id": "current", "label": "Current"},
                            {"id": "legacy", "label": "Legacy"}
                        ]
                    }
                ]
            }),
        }));

        app.update(press(KeyCode::Down));
        assert!(app.update(press(KeyCode::Enter)).is_empty());
        let flow = app.state.question.as_ref().expect("question flow");
        assert_eq!(flow.current, 1);
        assert_eq!(flow.answers[0].option_id.as_deref(), Some("complete"));

        app.update(press(KeyCode::BackTab));
        app.update(press(KeyCode::Enter));
        app.update(AppEvent::Terminal(TerminalEvent::Paste(
            "Rust 1.85+".to_owned(),
        )));
        let effects = app.update(press(KeyCode::Enter));

        assert!(matches!(
            effects.as_slice(),
            [AppEffect::ReplyQuestions {
                request_id,
                answers,
            }] if request_id == "question-1"
                && answers.len() == 2
                && answers[1].value == "Rust 1.85+"
                && answers[1].option_id.is_none()
        ));
    }

    #[test]
    fn plan_review_current_context_requires_confirmation_and_leaves_plan_mode() {
        let mut app = App::new(state());
        app.state.plan_mode_active = true;
        let ready = plan(PlanStatus::Submitted);
        app.update(AppEvent::Host(RpcEvent {
            kind: "plan_ready".to_owned(),
            payload: json!({"artifact": ready}),
        }));

        assert!(matches!(
            app.state.plan_review,
            Some(PlanReviewState::Menu { selected: 0 })
        ));
        assert!(app.update(press(KeyCode::Enter)).is_empty());
        assert!(matches!(
            app.state.plan_review,
            Some(PlanReviewState::Confirm {
                target: PlanExecutionTarget::Current,
                submitting: false,
                ..
            })
        ));

        assert_eq!(
            app.update(press(KeyCode::Enter)),
            vec![AppEffect::ExecutePlan(PlanExecutionTarget::Current)]
        );
        let executing = plan(PlanStatus::Executing);
        app.update(AppEvent::Command(CommandEvent::PlanExecutionFinished {
            target: PlanExecutionTarget::Current,
            result: Ok(Box::new(PlanExecutionData {
                artifact: executing,
                session_id: "session-1".to_owned(),
                fresh: false,
            })),
        }));

        assert!(!app.state.plan_mode_active);
        assert_eq!(app.state.session.session_id, "session-1");
        assert!(app.state.plan_review.is_none());
    }

    #[test]
    fn plan_review_fresh_context_is_a_distinct_execution_effect() {
        let mut app = App::new(state());
        app.update(AppEvent::Host(RpcEvent {
            kind: "plan_ready".to_owned(),
            payload: json!({"artifact": plan(PlanStatus::Submitted)}),
        }));

        app.update(press(KeyCode::Down));
        app.update(press(KeyCode::Enter));
        assert!(matches!(
            app.state.plan_review,
            Some(PlanReviewState::Confirm {
                target: PlanExecutionTarget::Fresh,
                submitting: false,
                ..
            })
        ));
        assert_eq!(
            app.update(press(KeyCode::Char('y'))),
            vec![AppEffect::ExecutePlan(PlanExecutionTarget::Fresh)]
        );
    }

    #[test]
    fn editor_moves_and_deletes_on_unicode_boundaries() {
        let mut editor = EditorState::default();
        editor.insert_text("你a");
        editor.move_left();
        editor.backspace();

        assert_eq!(editor.text(), "a");
        assert_eq!(editor.cursor(), 0);
    }

    #[test]
    fn multiline_input_keys_take_priority_over_send_while_streaming() {
        let mut app = App::new(state());
        app.state.run_state = RunState::Running;
        app.state.session.is_streaming = true;
        app.state.editor.insert_text("first");

        assert!(
            app.update(press_with(KeyCode::Enter, KeyModifiers::SHIFT))
                .is_empty()
        );
        app.update(press_with(KeyCode::Char('j'), KeyModifiers::CONTROL));

        assert_eq!(app.state.editor.text(), "first\n\n");
        assert!(
            app.state
                .transcript
                .iter()
                .all(|item| !matches!(item, TranscriptItem::User(_)))
        );
    }

    #[test]
    fn enter_creates_pending_user_and_prompt_effect() {
        let mut app = App::new(state());
        app.update(press(KeyCode::Char('h')));
        app.update(press(KeyCode::Char('i')));
        let effects = app.update(press(KeyCode::Enter));

        assert_eq!(effects, vec![AppEffect::Prompt("hi".to_owned())]);
        assert_eq!(app.state().run_state, RunState::Submitting);
        assert!(matches!(
            app.state().transcript.last(),
            Some(TranscriptItem::User(UserMessage {
                status: UserMessageStatus::Pending,
                ..
            }))
        ));
    }

    #[test]
    fn running_input_uses_pi_steer_follow_up_and_restores_cleared_queue() {
        let mut app = App::new(state());
        app.state.run_state = RunState::Running;
        app.state.session.is_streaming = true;
        app.state.editor.insert_text("steer now");

        assert_eq!(
            app.update(press(KeyCode::Enter)),
            vec![AppEffect::Steer("steer now".to_owned())]
        );

        app.state.editor.insert_text("after completion");
        assert_eq!(
            app.update(press_with(KeyCode::Enter, KeyModifiers::ALT)),
            vec![AppEffect::FollowUp("after completion".to_owned())]
        );
        app.update(AppEvent::Pi(RpcEvent {
            kind: "queue_update".to_owned(),
            payload: json!({
                "type": "queue_update",
                "steering": ["steer now"],
                "followUp": ["after completion"]
            }),
        }));
        assert_eq!(app.state().session.pending_message_count, 2);
        assert_eq!(
            app.update(press_with(KeyCode::Up, KeyModifiers::ALT)),
            vec![AppEffect::ClearQueue]
        );

        app.update(AppEvent::Command(CommandEvent::QueueCleared(Ok(Box::new(
            QueueClearData {
                steering: vec!["steer now".to_owned()],
                follow_up: vec!["after completion".to_owned()],
                restored_text: "steer now\n\nafter completion".to_owned(),
            },
        )))));
        assert_eq!(app.state().editor.text(), "steer now\n\nafter completion");
    }

    #[test]
    fn harness_commands_are_local_and_goal_is_explicit() {
        let mut app = App::new(state());
        assert!(!app.state().plan_mode_active);

        for (source, expected) in [
            ("/resources", AppEffect::GetResources),
            ("/reload", AppEffect::ReloadResources),
            ("/goals", AppEffect::GetGoals),
            ("/agents", AppEffect::GetAgents),
            ("/agents reload", AppEffect::ReloadAgents),
            (
                "/agents apply agent-7",
                AppEffect::IntegrateSubagent {
                    agent_id: "agent-7".to_owned(),
                    action: "apply".to_owned(),
                },
            ),
        ] {
            app.state.editor.replace(source.to_owned());
            assert_eq!(app.update(press(KeyCode::Enter)), vec![expected]);
        }
        app.state
            .editor
            .replace("/goal implement leases".to_owned());
        assert_eq!(
            app.update(press(KeyCode::Enter)),
            vec![AppEffect::StartGoal {
                objective: Some("implement leases".to_owned()),
                from_plan: false,
            }]
        );
        app.state.editor.replace("/goal from-plan".to_owned());
        assert_eq!(
            app.update(press(KeyCode::Enter)),
            vec![AppEffect::StartGoal {
                objective: None,
                from_plan: true,
            }]
        );
        assert!(
            !app.state()
                .transcript
                .iter()
                .any(|item| matches!(item, TranscriptItem::User(_)))
        );
    }

    #[test]
    fn direct_subagent_command_is_local_and_backgrounded() {
        let mut app = App::new(state());
        app.state
            .editor
            .replace("/agent reviewer review the diff".to_owned());

        assert_eq!(
            app.update(press(KeyCode::Enter)),
            vec![AppEffect::StartSubagent {
                profile: "reviewer".to_owned(),
                task: "review the diff".to_owned(),
            }]
        );
        assert!(
            !app.state()
                .transcript
                .iter()
                .any(|item| matches!(item, TranscriptItem::User(_)))
        );
    }

    #[test]
    fn agent_picker_completes_a_profile_without_starting_it() {
        let mut app = App::new(state());
        app.state.agents = serde_json::from_value(json!({
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
            "active": [],
            "diagnostics": []
        }))
        .unwrap();
        app.state.editor.replace("/agent".to_owned());

        assert!(app.update(press(KeyCode::Enter)).is_empty());
        assert!(app.state.agent_picker.is_some());
        assert!(app.update(press(KeyCode::Enter)).is_empty());
        assert!(app.state.agent_picker.is_none());
        assert_eq!(app.state.editor.text(), "/agent reviewer ");
    }

    #[test]
    fn subagent_lifecycle_updates_active_state_and_structured_transcript() {
        let mut app = App::new(state());
        let agent = json!({
            "id": "agent-1",
            "profile": "reviewer",
            "task": "Review",
            "taskId": null,
            "goalId": null,
            "lifecycle": "running",
            "startedAt": "2026-01-01T00:00:00Z",
            "turns": 2,
            "maxTurns": 12,
            "model": "test/model",
            "originSessionId": "session-1"
        });
        app.update(AppEvent::Host(RpcEvent {
            kind: "subagent_state".to_owned(),
            payload: json!({"event": "started", "agent": agent.clone()}),
        }));
        assert_eq!(app.state.agents.active.len(), 1);

        app.update(AppEvent::Host(RpcEvent {
            kind: "subagent_state".to_owned(),
            payload: json!({
                "event": "completed",
                "agent": agent,
                "result": {"status": "completed", "summary": "Looks good"}
            }),
        }));
        assert!(app.state.agents.active.is_empty());
        assert!(app.state.transcript.iter().any(|item| matches!(
            item,
            TranscriptItem::Subagent(SubagentTranscript { event, .. })
                if event == "completed"
        )));
    }

    #[test]
    fn pending_worktree_result_opens_integration_choices() {
        let mut app = App::new(state());
        app.update(AppEvent::Host(RpcEvent {
            kind: "subagent_integration".to_owned(),
            payload: json!({
                "event": "pending",
                "agent": {
                    "id": "agent-9",
                    "profile": "worker",
                    "task": "Implement",
                    "lifecycle": "awaiting_integration",
                    "startedAt": "2026-01-01T00:00:00Z",
                    "turns": 3,
                    "maxTurns": 32,
                    "model": "test/model",
                    "originSessionId": "session-1",
                    "isolationBackend": "worktree",
                    "integrationStatus": "pending"
                },
                "integration": {
                    "backend": "worktree",
                    "status": "pending",
                    "artifactId": "artifact-1",
                    "changedPaths": ["src/lib.rs"],
                    "patchBytes": 123
                }
            }),
        }));
        assert!(app.state.integration_prompt.is_some());
        assert_eq!(
            app.update(press(KeyCode::Enter)),
            vec![AppEffect::IntegrateSubagent {
                agent_id: "agent-9".to_owned(),
                action: "apply".to_owned(),
            }]
        );
    }

    #[test]
    fn recoverable_host_warnings_are_visible_in_the_transcript() {
        let mut app = App::new(state());
        app.update(AppEvent::Host(RpcEvent {
            kind: "host_warning".to_owned(),
            payload: json!({"message": "worktree recovery could not be persisted"}),
        }));

        assert!(app.state.transcript.iter().any(|item| matches!(
            item,
            TranscriptItem::Error(message)
                if message == "worktree recovery could not be persisted"
        )));
    }

    #[test]
    fn integration_prompts_queue_and_advance_without_overwrite() {
        let mut app = App::new(state());
        let event = |id: &str, lifecycle: &str| {
            AppEvent::Host(RpcEvent {
                kind: "subagent_integration".to_owned(),
                payload: json!({
                    "event": lifecycle,
                    "agent": {
                        "id": id,
                        "profile": "worker",
                        "task": "Implement",
                        "lifecycle": "awaiting_integration",
                        "startedAt": "2026-01-01T00:00:00Z",
                        "turns": 3,
                        "maxTurns": 32,
                        "model": "test/model",
                        "originSessionId": "session-1",
                        "isolationBackend": "worktree",
                        "integrationStatus": lifecycle
                    },
                    "integration": {
                        "backend": "worktree",
                        "status": lifecycle,
                        "artifactId": format!("artifact-{id}"),
                        "changedPaths": ["src/lib.rs"],
                        "patchBytes": 123
                    }
                }),
            })
        };
        app.update(event("agent-1", "pending"));
        app.update(event("agent-2", "pending"));
        assert_eq!(
            app.state
                .integration_prompt
                .as_ref()
                .map(|prompt| prompt.agent.id.as_str()),
            Some("agent-1")
        );
        assert_eq!(app.state.integration_prompt_queue.len(), 1);

        app.update(event("agent-1", "applied"));
        assert_eq!(
            app.state
                .integration_prompt
                .as_ref()
                .map(|prompt| prompt.agent.id.as_str()),
            Some("agent-2")
        );
        assert!(app.state.integration_prompt_queue.is_empty());
    }

    #[test]
    fn modal_priority_routes_keys_to_the_visible_question() {
        let mut app = App::new(state());
        app.update(press_with(KeyCode::Char('o'), KeyModifiers::CONTROL));
        assert_eq!(app.state.active_modal_kind(), Some(UiModalKind::Transcript));
        app.update(AppEvent::Host(RpcEvent {
            kind: "subagent_integration".to_owned(),
            payload: json!({
                "event": "pending",
                "agent": {
                    "id": "agent-9", "profile": "worker", "task": "Implement",
                    "lifecycle": "awaiting_integration", "startedAt": "now",
                    "turns": 1, "maxTurns": 3, "model": "test/model",
                    "originSessionId": "session-1"
                },
                "integration": {
                    "backend": "worktree", "status": "pending",
                    "changedPaths": [], "patchBytes": 1
                }
            }),
        }));
        app.update(AppEvent::Host(RpcEvent {
            kind: "question_request".to_owned(),
            payload: json!({
                "requestId": "question-visible",
                "questions": [{
                    "id": "choice",
                    "prompt": "Choose",
                    "options": [
                        {"id": "first", "label": "First"},
                        {"id": "second", "label": "Second"}
                    ]
                }]
            }),
        }));
        assert_eq!(app.state.active_modal_kind(), Some(UiModalKind::Question));
        app.update(press(KeyCode::Down));
        assert!(matches!(
            app.update(press(KeyCode::Enter)).as_slice(),
            [AppEffect::ReplyQuestions { request_id, answers }]
                if request_id == "question-visible"
                    && answers[0].option_id.as_deref() == Some("second")
        ));
        assert!(app.state.integration_prompt.is_some());
        assert!(app.state.transcript_viewer.is_some());
    }

    #[test]
    fn newer_goal_lifecycle_event_wins_over_an_earlier_rpc_response() {
        let mut app = App::new(state());
        let snapshot = |revision: u64, stage: &str| {
            serde_json::from_value::<GoalSnapshot>(json!({
                "goal": {
                    "id": "goal-1",
                    "sessionId": "session-1",
                    "objective": "Implement",
                    "stage": stage,
                    "revision": revision,
                    "constraints": [],
                    "acceptanceCriteria": [],
                    "tasks": [],
                    "reviews": [],
                    "repairCycles": 0
                },
                "statePath": "/state/goal.json"
            }))
            .unwrap()
        };
        app.update(AppEvent::Host(RpcEvent {
            kind: "goal_state".to_owned(),
            payload: json!({
                "type": "goal_state",
                "snapshot": serde_json::to_value(snapshot(2, "blocked")).unwrap()
            }),
        }));
        app.update(AppEvent::Command(CommandEvent::GoalStarted(Ok(Box::new(
            snapshot(1, "preparing"),
        )))));

        let goal = app
            .state()
            .goal
            .as_ref()
            .and_then(|snapshot| snapshot.goal.as_ref())
            .unwrap();
        assert_eq!(goal.revision, 2);
        assert_eq!(goal.stage, "blocked");
        assert_ne!(app.state().run_state, RunState::Submitting);
        assert!(!app.state().plan_mode_active);
    }

    #[test]
    fn stale_plan_status_and_foreign_scope_snapshots_cannot_replace_current_state() {
        let mut app = App::new(state());
        let mut current = plan(PlanStatus::Executing);
        current.updated_at = "2026-01-01T00:00:03.000Z".to_owned();
        app.state.plan = Some(current.clone());

        let mut stale = plan(PlanStatus::Submitted);
        stale.updated_at = "2026-01-01T00:00:02.000Z".to_owned();
        app.update(AppEvent::Command(CommandEvent::PlanStateFinished(Ok(
            Box::new(crate::host::PlanStateData {
                scope_id: Some("session-1".to_owned()),
                artifact: Some(stale),
            }),
        ))));
        assert_eq!(
            app.state.plan.as_ref().unwrap().status,
            PlanStatus::Executing
        );

        let mut foreign = plan(PlanStatus::Completed);
        foreign.updated_at = "2026-01-01T00:00:04.000Z".to_owned();
        app.update(AppEvent::Command(CommandEvent::PlanStateFinished(Ok(
            Box::new(crate::host::PlanStateData {
                scope_id: Some("session-other".to_owned()),
                artifact: Some(foreign),
            }),
        ))));
        assert_eq!(app.state.plan.as_ref().unwrap(), &current);
    }

    #[test]
    fn stale_cross_goal_and_lower_revision_snapshots_are_ignored() {
        let mut app = App::new(state());
        let snapshot = |id: &str, revision: u64, updated_at: &str| {
            serde_json::from_value::<GoalSnapshot>(json!({
                "scopeId": "session-1",
                "goal": {
                    "id": id,
                    "sessionId": "session-1",
                    "objective": "Implement",
                    "stage": "executing",
                    "revision": revision,
                    "constraints": [],
                    "acceptanceCriteria": [],
                    "tasks": [],
                    "reviews": [],
                    "repairCycles": 0,
                    "updatedAt": updated_at
                },
                "statePath": "/state/goal.json"
            }))
            .unwrap()
        };
        assert!(app.receive_goal(snapshot("goal-new", 2, "2026-01-01T00:00:03.000Z"), false,));
        assert!(!app.receive_goal(snapshot("goal-old", 99, "2026-01-01T00:00:02.000Z"), false,));
        assert!(!app.receive_goal(snapshot("goal-new", 1, "2026-01-01T00:00:04.000Z"), false,));
        assert_eq!(
            app.state
                .goal
                .as_ref()
                .and_then(|snapshot| snapshot.goal.as_ref())
                .unwrap()
                .id,
            "goal-new"
        );
    }

    #[test]
    fn host_events_from_a_previous_session_scope_are_ignored() {
        let mut app = App::new(state());
        let initial_revision = app.state.resources.revision;
        app.update(AppEvent::Host(RpcEvent {
            kind: "resource_state".to_owned(),
            payload: json!({
                "scopeId": "session-old",
                "snapshot": {
                    "scopeId": "session-old",
                    "trusted": true,
                    "contextFiles": [],
                    "skills": [],
                    "prompts": [],
                    "extensions": [],
                    "commands": [],
                    "diagnostics": [],
                    "revision": initial_revision + 10
                }
            }),
        }));

        assert_eq!(app.state.resources.revision, initial_revision);
        assert!(!app.state.resources.trusted);
    }

    #[test]
    fn workspace_state_updates_resources_and_agents_atomically() {
        let mut app = App::new(state());
        let event = |agents: Value| {
            AppEvent::Host(RpcEvent {
                kind: "workspace_state".to_owned(),
                payload: json!({
                    "scopeId": "session-1",
                    "resources": {
                        "scopeId": "session-1",
                        "trusted": true,
                        "contextFiles": ["AGENTS.md"],
                        "skills": [],
                        "prompts": [],
                        "extensions": [],
                        "commands": [],
                        "diagnostics": [],
                        "revision": 2
                    },
                    "agents": agents
                }),
            })
        };

        app.update(event(json!({"invalid": true})));
        assert!(!app.state.resources.trusted);
        assert_eq!(app.state.agents.revision, 0);

        app.update(event(json!({
            "scopeId": "session-1",
            "revision": 2,
            "maxParallel": 3,
            "profiles": [],
            "active": [],
            "pending": [],
            "diagnostics": []
        })));
        assert!(app.state.resources.trusted);
        assert_eq!(app.state.agents.revision, 2);
    }

    #[test]
    fn goal_spec_approval_is_independent_from_plan_mode_and_plan_review() {
        let mut app = App::new(state());
        app.state.plan_mode_active = true;
        app.update(AppEvent::Host(RpcEvent {
            kind: "goal_spec_ready".to_owned(),
            payload: json!({
                "snapshot": {
                    "goal": {
                        "id": "goal-1",
                        "sessionId": "session-1",
                        "objective": "Background work",
                        "stage": "awaiting_approval",
                        "revision": 2,
                        "constraints": [],
                        "acceptanceCriteria": ["cargo test"],
                        "spec": {
                            "revision": 1,
                            "summary": "Execute independently",
                            "acceptanceCriteria": ["cargo test"],
                            "allowedTools": ["read"],
                            "allowedPaths": ["."],
                            "allowedCommands": []
                        },
                        "tasks": [],
                        "reviews": [],
                        "repairCycles": 0
                    },
                    "statePath": "/state/goal.json"
                }
            }),
        }));

        assert!(app.state.plan_mode_active);
        assert!(app.state.plan_review.is_none());
        assert!(matches!(
            app.state.goal_approval,
            Some(GoalApprovalState {
                submitting: false,
                ..
            })
        ));
        assert_eq!(
            app.update(press(KeyCode::Enter)),
            vec![AppEffect::ApproveGoal]
        );
    }

    #[test]
    fn prompt_acceptance_does_not_finish_the_agent_run() {
        let mut app = App::new(state());
        app.state.run_state = RunState::Submitting;
        app.update(AppEvent::Command(CommandEvent::PromptFinished(Ok(()))));
        assert_eq!(app.state().run_state, RunState::Submitting);

        app.update(AppEvent::Pi(RpcEvent {
            kind: "agent_start".to_owned(),
            payload: json!({"type": "agent_start"}),
        }));
        assert_eq!(app.state().run_state, RunState::Running);
        assert!(app.state().session.is_streaming);

        app.update(AppEvent::Pi(RpcEvent {
            kind: "agent_end".to_owned(),
            payload: json!({"type": "agent_end", "messages": []}),
        }));
        assert_eq!(app.state().run_state, RunState::Idle);
        assert!(!app.state().session.is_streaming);
    }

    #[test]
    fn tick_only_schedules_render_and_does_not_dirty_state() {
        let mut app = App::new(state());
        assert!(app.take_redraw_request());

        let effects = app.update(AppEvent::Tick);

        assert!(effects.is_empty());
        assert!(!app.take_redraw_request());
    }

    #[test]
    fn command_failure_enters_the_same_reducer_as_pi_events() {
        let mut app = App::new(state());
        app.state.editor.insert_text("hello");
        app.update(press(KeyCode::Enter));

        app.update(AppEvent::Command(CommandEvent::PromptFinished(Err(
            "request failed".to_owned(),
        ))));

        assert_eq!(app.state().run_state, RunState::Error);
        assert!(matches!(
            &app.state().transcript[0],
            TranscriptItem::User(UserMessage {
                status: UserMessageStatus::Failed,
                ..
            })
        ));
        assert!(matches!(
            &app.state().transcript[1],
            TranscriptItem::Error(error) if error == "request failed"
        ));
    }

    #[test]
    fn login_is_handled_locally_instead_of_becoming_a_prompt() {
        let mut app = App::new(state());
        app.state.editor.insert_text("/login");

        let effects = app.update(press(KeyCode::Enter));

        assert_eq!(effects, vec![AppEffect::AuthList]);
        assert_eq!(app.state().run_state, RunState::Authenticating);
        assert!(matches!(
            app.state().auth_state,
            AuthState::LoadingProviders
        ));
        assert!(matches!(
            &app.state().transcript[0],
            TranscriptItem::User(UserMessage {
                status: UserMessageStatus::Accepted,
                ..
            })
        ));
    }

    #[test]
    fn login_provider_list_filters_and_selects_from_the_search_input() {
        let mut app = App::new(state());
        app.state.auth_state = AuthState::LoadingProviders;
        app.state.run_state = RunState::Authenticating;
        app.update(AppEvent::Command(CommandEvent::AuthProvidersFinished(Ok(
            vec![
                AuthProvider {
                    id: "openai-codex".to_owned(),
                    name: "OpenAI Codex".to_owned(),
                    configured: false,
                    configured_type: None,
                    configured_source: None,
                    methods: vec![AuthMethod {
                        kind: "oauth".to_owned(),
                        label: "ChatGPT Plus/Pro".to_owned(),
                        available: true,
                    }],
                },
                AuthProvider {
                    id: "github-copilot".to_owned(),
                    name: "GitHub Copilot".to_owned(),
                    configured: false,
                    configured_type: None,
                    configured_source: None,
                    methods: vec![AuthMethod {
                        kind: "oauth".to_owned(),
                        label: "GitHub device login".to_owned(),
                        available: true,
                    }],
                },
            ],
        ))));

        app.update(AppEvent::Terminal(TerminalEvent::Paste(
            "github device".to_owned(),
        )));
        let AuthState::Selecting {
            choices,
            selected,
            filter,
        } = &app.state().auth_state
        else {
            panic!("expected searchable provider selection");
        };
        assert_eq!(filter.text(), "github device");
        assert_eq!(*selected, 0);
        assert_eq!(
            matching_auth_choice_indices(choices, filter.text()),
            vec![1]
        );

        let effects = app.update(press(KeyCode::Enter));
        assert!(matches!(
            effects.as_slice(),
            [AppEffect::AuthLogin {
                provider_id,
                auth_type,
                ..
            }] if provider_id == "github-copilot" && auth_type == "oauth"
        ));
    }

    #[test]
    fn oauth_url_is_stored_opened_once_and_rejects_unsafe_schemes() {
        let mut app = App::new(state());
        app.state.run_state = RunState::Authenticating;
        app.state.auth_state = AuthState::Running(Box::new(AuthFlowState {
            id: "flow-1".to_owned(),
            provider_name: "OpenAI Codex".to_owned(),
            status: "Starting login…".to_owned(),
            url: None,
            device_code: None,
            prompt: None,
        }));
        let event = || {
            AppEvent::Host(RpcEvent {
                kind: "auth_notify".to_owned(),
                payload: json!({
                    "type": "auth_notify",
                    "flowId": "flow-1",
                    "event": {
                        "type": "auth_url",
                        "url": "https://auth.openai.com/oauth/authorize?state=test",
                        "instructions": "Continue in your browser"
                    }
                }),
            })
        };

        assert_eq!(
            app.update(event()),
            vec![AppEffect::OpenUrl(
                "https://auth.openai.com/oauth/authorize?state=test".to_owned()
            )]
        );
        assert!(app.update(event()).is_empty());
        let AuthState::Running(flow) = &app.state().auth_state else {
            panic!("expected active auth flow");
        };
        assert_eq!(
            flow.url.as_deref(),
            Some("https://auth.openai.com/oauth/authorize?state=test")
        );

        let effects = app.update(AppEvent::Host(RpcEvent {
            kind: "auth_notify".to_owned(),
            payload: json!({
                "type": "auth_notify",
                "flowId": "flow-1",
                "event": {
                    "type": "auth_url",
                    "url": "javascript:alert(1)"
                }
            }),
        }));
        assert!(effects.is_empty());
    }

    #[test]
    fn login_provider_secret_prompt_and_completion_stay_inside_auth_state() {
        let mut app = App::new(state());
        app.state.editor.insert_text("/login");
        app.update(press(KeyCode::Enter));
        app.update(AppEvent::Command(CommandEvent::AuthProvidersFinished(Ok(
            vec![AuthProvider {
                id: "test".to_owned(),
                name: "Test Provider".to_owned(),
                configured: false,
                configured_type: None,
                configured_source: None,
                methods: vec![AuthMethod {
                    kind: "api_key".to_owned(),
                    label: "API key".to_owned(),
                    available: true,
                }],
            }],
        ))));

        let effects = app.update(press(KeyCode::Enter));
        assert!(matches!(
            effects.as_slice(),
            [AppEffect::AuthLogin {
                provider_id,
                auth_type,
                ..
            }] if provider_id == "test" && auth_type == "api_key"
        ));
        let AuthState::Running(flow) = &app.state().auth_state else {
            panic!("expected active auth flow");
        };
        let flow_id = flow.id.clone();

        app.update(AppEvent::Host(RpcEvent {
            kind: "auth_prompt".to_owned(),
            payload: json!({
                "type": "auth_prompt",
                "flowId": flow_id,
                "promptId": "prompt-1",
                "promptType": "secret",
                "message": "Enter API key"
            }),
        }));
        app.update(press(KeyCode::Char('s')));
        app.update(press(KeyCode::Char('k')));

        let effects = app.update(press(KeyCode::Enter));
        assert!(matches!(
            effects.as_slice(),
            [AppEffect::AuthReply {
                prompt_id,
                value,
                ..
            }] if prompt_id == "prompt-1" && value.expose() == "sk"
        ));
        assert_eq!(app.state().transcript.len(), 1);

        app.update(AppEvent::Command(CommandEvent::AuthLoginFinished(Ok(
            AuthLoginData {
                provider_id: "test".to_owned(),
                credential_type: "api_key".to_owned(),
                selected_model: None,
            },
        ))));
        assert!(matches!(app.state().auth_state, AuthState::Inactive));
        assert_eq!(app.state().run_state, RunState::Idle);
    }

    #[test]
    fn compact_uses_the_dedicated_rpc_effect() {
        let mut app = App::new(state());
        app.state.editor.insert_text("/compact keep decisions");

        let effects = app.update(press(KeyCode::Enter));

        assert_eq!(
            effects,
            vec![AppEffect::Compact(Some("keep decisions".to_owned()))]
        );
        assert_eq!(app.state().run_state, RunState::Compacting);
        assert!(
            !app.state()
                .transcript
                .iter()
                .any(|item| matches!(item, TranscriptItem::User(_)))
        );
    }

    #[test]
    fn context_is_a_read_only_local_query_and_never_becomes_a_user_message() {
        let mut app = App::new(state());
        app.state.editor.insert_text("/context");

        assert_eq!(
            app.update(press(KeyCode::Enter)),
            vec![AppEffect::GetContextState]
        );
        assert!(app.state().transcript.is_empty());

        let snapshot = ContextSnapshot {
            context_window: Some(200_000),
            estimated_next_request_tokens: 94_000,
            ..ContextSnapshot::default()
        };
        app.update(AppEvent::Command(CommandEvent::ContextStateFinished(Ok(
            Box::new(snapshot.clone()),
        ))));

        assert_eq!(app.state().context, snapshot);
        assert!(matches!(
            app.state().transcript.as_slice(),
            [TranscriptItem::Context(_)]
        ));
    }

    #[test]
    fn shift_tab_and_plan_command_switch_only_after_host_confirmation() {
        let mut app = App::new(state());
        assert!(!app.state().plan_mode_active);
        let transcript_len = app.state().transcript.len();

        let effects = app.update(press(KeyCode::BackTab));
        assert_eq!(effects, vec![AppEffect::SetPlanMode(true)]);
        assert!(!app.state().plan_mode_active);
        assert_eq!(app.state().pending_plan_mode, Some(true));

        app.update(AppEvent::Command(CommandEvent::SetPlanModeFinished {
            requested: true,
            result: Ok(HostPlanModeData {
                active: true,
                active_tools: vec![
                    "read".to_owned(),
                    "grep".to_owned(),
                    "find".to_owned(),
                    "ls".to_owned(),
                ],
            }),
        }));
        assert!(app.state().plan_mode_active);
        assert_eq!(app.state().pending_plan_mode, None);
        assert_eq!(app.state().transcript.len(), transcript_len);

        app.state.editor.insert_text("/plan exit");
        assert_eq!(
            app.update(press(KeyCode::Enter)),
            vec![AppEffect::SetPlanMode(false)]
        );
        assert_eq!(app.state().transcript.len(), transcript_len);
    }

    #[test]
    fn plan_mode_switch_is_rejected_while_agent_is_running() {
        let mut app = App::new(state());
        app.state.run_state = RunState::Running;
        let transcript_len = app.state().transcript.len();

        assert!(app.update(press(KeyCode::BackTab)).is_empty());
        assert!(!app.state().plan_mode_active);
        assert_eq!(app.state().transcript.len(), transcript_len);
    }

    #[test]
    fn mutation_tool_waits_for_approval_and_resumes_after_allow() {
        let mut app = App::new(state());
        app.state.run_state = RunState::Running;
        app.update(AppEvent::Pi(RpcEvent {
            kind: "tool_execution_start".to_owned(),
            payload: json!({
                "type": "tool_execution_start",
                "toolCallId": "call-1",
                "toolName": "bash",
                "args": {"command": "cargo test"}
            }),
        }));

        app.update(AppEvent::Host(RpcEvent {
            kind: "approval_request".to_owned(),
            payload: json!({
                "type": "approval_request",
                "approvalId": "approval-1",
                "toolCallId": "call-1",
                "toolName": "bash",
                "input": {"command": "cargo test"}
            }),
        }));
        assert!(matches!(
            app.state().transcript.last(),
            Some(TranscriptItem::Tool(ToolExecution {
                status: ToolStatus::WaitingApproval,
                ..
            }))
        ));

        let effects = app.update(press(KeyCode::Char('y')));
        assert_eq!(
            effects,
            vec![AppEffect::ReplyApproval {
                approval_id: "approval-1".to_owned(),
                decision: ApprovalDecision::Allow,
            }]
        );

        app.update(AppEvent::Command(CommandEvent::ApprovalReplyFinished {
            approval_id: "approval-1".to_owned(),
            decision: ApprovalDecision::Allow,
            result: Ok(()),
        }));
        assert!(app.state().approval.is_none());
        assert!(matches!(
            app.state().transcript.last(),
            Some(TranscriptItem::Tool(ToolExecution {
                status: ToolStatus::Running,
                ..
            }))
        ));
    }

    #[test]
    fn approval_interrupt_denies_and_aborts_the_agent() {
        let mut app = App::new(state());
        app.state.run_state = RunState::Running;
        app.state.approval = Some(ApprovalState {
            approval_id: "approval-1".to_owned(),
            tool_call_id: "call-1".to_owned(),
            tool_name: "write".to_owned(),
            input: json!({"path": "src/lib.rs", "content": "changed"}),
            agent_id: None,
            agent_profile: None,
            model: None,
            goal_id: None,
            reason: None,
            risk: None,
            selected: 0,
            replying: false,
        });

        assert_eq!(
            app.update(press(KeyCode::Esc)),
            vec![AppEffect::AbortAndClearQueue]
        );
        assert_eq!(app.state().run_state, RunState::Aborting);
    }

    #[test]
    fn approval_and_goal_approval_use_shared_navigation_before_confirming() {
        let mut app = App::new(state());
        app.state.run_state = RunState::Running;
        app.state.approval = Some(ApprovalState {
            approval_id: "approval-1".to_owned(),
            tool_call_id: "call-1".to_owned(),
            tool_name: "write".to_owned(),
            input: json!({"path": "src/lib.rs"}),
            agent_id: None,
            agent_profile: None,
            model: None,
            goal_id: Some("goal-1".to_owned()),
            reason: None,
            risk: None,
            selected: 0,
            replying: false,
        });

        assert!(app.update(press(KeyCode::Down)).is_empty());
        assert_eq!(app.state.approval.as_ref().unwrap().selected, 1);
        assert!(app.update(press(KeyCode::Down)).is_empty());
        assert_eq!(
            app.update(press(KeyCode::Enter)),
            vec![AppEffect::ReplyApproval {
                approval_id: "approval-1".to_owned(),
                decision: ApprovalDecision::Deny,
            }]
        );

        app.state.approval = None;
        app.state.goal_approval = Some(GoalApprovalState {
            selected: 0,
            submitting: false,
        });
        assert!(app.update(press(KeyCode::Down)).is_empty());
        assert_eq!(app.state.goal_approval.as_ref().unwrap().selected, 1);
        assert!(app.update(press(KeyCode::Enter)).is_empty());
        assert!(app.state.goal_approval.is_none());
    }

    #[test]
    fn shared_choice_navigation_skips_disabled_rows_and_numbers_only_select() {
        let mut selected = 0;
        let enabled = [true, false, true];
        assert_eq!(
            update_choice_navigation(
                KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
                &mut selected,
                &enabled,
            ),
            ChoiceNavAction::Handled
        );
        assert_eq!(selected, 2);
        assert_eq!(
            update_choice_navigation(
                KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE),
                &mut selected,
                &enabled,
            ),
            ChoiceNavAction::Handled
        );
        assert_eq!(selected, 2, "a disabled numeric choice must be ignored");
        assert_eq!(
            update_choice_navigation(
                KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE),
                &mut selected,
                &enabled,
            ),
            ChoiceNavAction::Handled
        );
        assert_eq!(selected, 0);
        assert_eq!(
            update_choice_navigation(
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                &mut selected,
                &enabled,
            ),
            ChoiceNavAction::Confirm(0)
        );
    }

    #[test]
    fn repeated_choice_click_selects_then_confirms_the_active_surface() {
        let mut app = App::new(state());
        app.state.run_state = RunState::Running;
        app.state.approval = Some(ApprovalState {
            approval_id: "approval-1".to_owned(),
            tool_call_id: "call-1".to_owned(),
            tool_name: "write".to_owned(),
            input: json!({"path": "src/lib.rs"}),
            agent_id: None,
            agent_profile: None,
            model: None,
            goal_id: None,
            reason: None,
            risk: None,
            selected: 0,
            replying: false,
        });
        let click = AppEvent::UiInput(UiInputEvent::Click(UiHitTarget::ChoiceOption(1)));
        assert!(app.update(click.clone()).is_empty());
        assert_eq!(app.state.approval.as_ref().unwrap().selected, 1);
        assert_eq!(
            app.update(click),
            vec![AppEffect::ReplyApproval {
                approval_id: "approval-1".to_owned(),
                decision: ApprovalDecision::Deny,
            }]
        );
    }

    #[test]
    fn tool_updates_replace_accumulated_output_and_keep_denied_status() {
        let mut app = App::new(state());
        app.update(AppEvent::Pi(RpcEvent {
            kind: "tool_execution_start".to_owned(),
            payload: json!({
                "type": "tool_execution_start",
                "toolCallId": "call-1",
                "toolName": "bash",
                "args": {"command": "printf test"}
            }),
        }));
        app.update(AppEvent::Pi(RpcEvent {
            kind: "tool_execution_update".to_owned(),
            payload: json!({
                "type": "tool_execution_update",
                "toolCallId": "call-1",
                "partialResult": {
                    "content": [{"type": "text", "text": "test"}]
                }
            }),
        }));
        let Some(TranscriptItem::Tool(tool)) = app.state.transcript.last_mut() else {
            panic!("expected tool");
        };
        assert_eq!(tool.output, "test");
        tool.status = ToolStatus::Denied;

        app.update(AppEvent::Pi(RpcEvent {
            kind: "tool_execution_end".to_owned(),
            payload: json!({
                "type": "tool_execution_end",
                "toolCallId": "call-1",
                "result": {"content": [{"type": "text", "text": "Denied by user"}]},
                "isError": true
            }),
        }));
        assert!(matches!(
            app.state().transcript.last(),
            Some(TranscriptItem::Tool(ToolExecution {
                status: ToolStatus::Denied,
                output,
                ..
            })) if output == "Denied by user"
        ));
    }

    #[test]
    fn discovered_commands_are_passed_through_and_unknown_commands_are_rejected() {
        let mut app = App::with_commands(
            state(),
            vec![DiscoveredCommand {
                name: "fix-tests".to_owned(),
                description: "Fix failing tests".to_owned(),
                source: "prompt".to_owned(),
            }],
        );
        app.state.editor.insert_text("/fix-tests src/parser");

        let effects = app.update(press(KeyCode::Enter));
        assert_eq!(
            effects,
            vec![AppEffect::Prompt("/fix-tests src/parser".to_owned())]
        );

        app.state.run_state = RunState::Idle;
        app.state.editor.insert_text("/missing");
        let effects = app.update(press(KeyCode::Enter));
        assert!(effects.is_empty());
        assert!(matches!(
            app.state().transcript.last(),
            Some(TranscriptItem::Notice(message)) if message.contains("Unknown command /missing")
        ));
    }

    #[test]
    fn tab_shift_tab_and_ctrl_np_navigate_command_candidates() {
        let mut app = App::with_commands(
            state(),
            vec![DiscoveredCommand {
                name: "fix-tests".to_owned(),
                description: "Fix failing tests".to_owned(),
                source: "prompt".to_owned(),
            }],
        );
        app.state.editor.insert_text("/");

        app.update(press(KeyCode::Tab));
        assert_eq!(
            app.state()
                .selected_command()
                .map(|command| command.name.as_str()),
            Some("compact")
        );
        assert_eq!(app.state().editor.text(), "/");

        app.update(press_with(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(
            app.state()
                .selected_command()
                .map(|command| command.name.as_str()),
            Some("login")
        );

        app.update(press_with(KeyCode::Char('n'), KeyModifiers::CONTROL));
        assert_eq!(
            app.state()
                .selected_command()
                .map(|command| command.name.as_str()),
            Some("compact")
        );

        app.update(press_with(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert_eq!(
            app.state()
                .selected_command()
                .map(|command| command.name.as_str()),
            Some("login")
        );

        let effects = app.update(press(KeyCode::Enter));
        assert!(effects.is_empty());
        assert_eq!(app.state().editor.text(), "/login ");
    }

    #[test]
    fn command_navigation_reaches_candidates_beyond_the_visible_window() {
        let mut app = App::new(state());
        app.state.editor.insert_text("/");

        assert_eq!(app.state().command_candidates().len(), 17);
        for _ in 0..16 {
            app.update(press(KeyCode::Tab));
        }
        assert_eq!(
            app.state()
                .selected_command()
                .map(|command| command.name.as_str()),
            Some("tree")
        );

        assert!(app.update(press(KeyCode::Enter)).is_empty());
        assert_eq!(app.state().editor.text(), "/tree ");
    }

    #[test]
    fn command_menu_supports_navigation_enter_and_escape() {
        let mut app = App::with_commands(
            state(),
            vec![DiscoveredCommand {
                name: "fix-tests".to_owned(),
                description: "Fix failing tests".to_owned(),
                source: "prompt".to_owned(),
            }],
        );
        app.state.editor.insert_text("/");
        app.state.reset_command_menu();

        app.update(press(KeyCode::Down));
        assert_eq!(
            app.state()
                .selected_command()
                .map(|command| command.name.as_str()),
            Some("compact")
        );

        let effects = app.update(press(KeyCode::Enter));
        assert!(effects.is_empty());
        assert_eq!(app.state().editor.text(), "/compact ");

        app.state.editor.clear();
        app.state.editor.insert_text("/");
        app.state.reset_command_menu();
        app.update(press(KeyCode::Esc));
        assert!(app.state().command_candidates().is_empty());
        assert_eq!(app.state().editor.text(), "/");

        app.update(press(KeyCode::Char('f')));
        assert_eq!(
            app.state()
                .selected_command()
                .map(|command| command.name.as_str()),
            Some("fix-tests")
        );
    }

    #[test]
    fn missing_credentials_enters_auth_required_and_blocks_more_prompts() {
        let mut app = App::new(state());
        app.state.editor.insert_text("hello");
        app.update(press(KeyCode::Enter));

        app.update(AppEvent::Command(CommandEvent::PromptFinished(Err(
            "Pi command prompt failed: No API key found. Use /login.".to_owned(),
        ))));

        assert_eq!(app.state().run_state, RunState::AuthRequired);
        assert!(matches!(
            &app.state().transcript[0],
            TranscriptItem::User(UserMessage {
                status: UserMessageStatus::Failed,
                ..
            })
        ));
        assert!(matches!(
            &app.state().transcript[2],
            TranscriptItem::Notice(message) if message.contains("Use /login")
        ));

        app.state.editor.insert_text("try again");
        let effects = app.update(press(KeyCode::Enter));
        assert!(effects.is_empty());
        assert_eq!(app.state().run_state, RunState::AuthRequired);
    }

    #[test]
    fn fatal_runtime_event_is_returned_as_an_effect() {
        let mut app = App::new(state());

        let effects = app.update(AppEvent::Runtime(RuntimeEvent::TerminalError(
            "input stream closed unexpectedly".to_owned(),
        )));

        assert_eq!(
            effects,
            vec![AppEffect::ExitWithError(
                "terminal input failed: input stream closed unexpectedly".to_owned()
            )]
        );
        assert_eq!(app.state().run_state, RunState::Error);
    }

    #[test]
    fn runtime_disconnect_maps_to_connection_and_error_state() {
        let mut app = App::new(state());

        app.update(AppEvent::Runtime(RuntimeEvent::PiDisconnected));

        assert_eq!(app.state().connection_state, ConnectionState::Disconnected);
        assert_eq!(app.state().run_state, RunState::Error);
        assert_eq!(
            app.state().last_error.as_deref(),
            Some("Pi process disconnected")
        );
    }

    #[test]
    fn compaction_events_keep_session_and_ui_phase_in_sync() {
        let mut app = App::new(state());

        app.update(AppEvent::Pi(RpcEvent {
            kind: "compaction_start".to_owned(),
            payload: json!({"type": "compaction_start"}),
        }));
        assert_eq!(app.state().run_state, RunState::Compacting);
        assert!(app.state().session.is_compacting);

        app.update(AppEvent::Pi(RpcEvent {
            kind: "compaction_end".to_owned(),
            payload: json!({
                "type": "compaction_end",
                "reason": "manual",
                "aborted": false,
                "willRetry": false,
                "result": {
                    "summary": "must not be rendered",
                    "firstKeptEntryId": "entry-4",
                    "tokensBefore": 82_000,
                    "estimatedTokensAfter": 31_000,
                    "details": {
                        "readFiles": ["src/a.rs", "src/b.rs"],
                        "modifiedFiles": ["src/b.rs", "src/c.rs"]
                    }
                }
            }),
        }));
        assert_eq!(app.state().run_state, RunState::Idle);
        assert!(!app.state().session.is_compacting);
        assert_eq!(
            app.state()
                .transcript
                .iter()
                .filter(|item| matches!(item, TranscriptItem::Compaction(_)))
                .count(),
            1
        );
        let Some(TranscriptItem::Compaction(record)) = app.state().transcript.last() else {
            panic!("expected compaction separator");
        };
        assert_eq!(record.tokens_saved, Some(51_000));
        assert_eq!(record.file_count, 3);
        assert!(
            !app.state()
                .transcript
                .iter()
                .any(|item| matches!(item, TranscriptItem::Notice(text) if text.contains("must not be rendered")))
        );

        app.update(AppEvent::Pi(RpcEvent {
            kind: "compaction_end".to_owned(),
            payload: json!({
                "type": "compaction_end",
                "reason": "manual",
                "aborted": false,
                "willRetry": false,
                "result": {
                    "firstKeptEntryId": "entry-4",
                    "tokensBefore": 82_000,
                    "estimatedTokensAfter": 31_000,
                    "details": {}
                }
            }),
        }));
        assert_eq!(
            app.state()
                .transcript
                .iter()
                .filter(|item| matches!(item, TranscriptItem::Compaction(_)))
                .count(),
            1,
            "duplicate lifecycle events must not add another separator"
        );
    }

    #[test]
    fn failed_aborted_and_overflow_compactions_are_explicit_without_success_separators() {
        for (reason, aborted, will_retry, error) in [
            ("manual", true, false, None),
            ("threshold", false, false, Some("summary provider failed")),
            ("overflow", false, true, Some("overflow recovery failed")),
        ] {
            let mut app = App::new(state());
            app.update(AppEvent::Pi(RpcEvent {
                kind: "compaction_start".to_owned(),
                payload: json!({"type": "compaction_start", "reason": reason}),
            }));
            app.update(AppEvent::Pi(RpcEvent {
                kind: "compaction_end".to_owned(),
                payload: json!({
                    "type": "compaction_end",
                    "reason": reason,
                    "aborted": aborted,
                    "willRetry": will_retry,
                    "result": null,
                    "errorMessage": error
                }),
            }));

            assert!(
                app.state()
                    .transcript
                    .iter()
                    .any(|item| matches!(item, TranscriptItem::Error(_)))
            );
            assert!(
                !app.state()
                    .transcript
                    .iter()
                    .any(|item| matches!(item, TranscriptItem::Compaction(_)))
            );
        }
    }

    #[test]
    fn lifecycle_completion_wins_over_the_later_compact_rpc_response() {
        let mut app = App::new(state());
        app.state.editor.insert_text("/compact");
        app.update(press(KeyCode::Enter));
        app.update(AppEvent::Pi(RpcEvent {
            kind: "compaction_start".to_owned(),
            payload: json!({"type": "compaction_start", "reason": "manual"}),
        }));
        app.update(AppEvent::Pi(RpcEvent {
            kind: "compaction_end".to_owned(),
            payload: json!({
                "type": "compaction_end",
                "reason": "manual",
                "aborted": false,
                "willRetry": false,
                "result": {
                    "firstKeptEntryId": "kept",
                    "tokensBefore": 50_000,
                    "details": {}
                }
            }),
        }));
        let transcript_len = app.state().transcript.len();

        app.update(AppEvent::Command(CommandEvent::CompactFinished(Err(
            "late RPC error".to_owned(),
        ))));

        assert_eq!(app.state().run_state, RunState::Idle);
        assert_eq!(app.state().transcript.len(), transcript_len);
        assert!(
            !app.state().transcript.iter().any(
                |item| matches!(item, TranscriptItem::Error(error) if error == "late RPC error")
            )
        );
    }

    #[test]
    fn context_budget_events_update_status_without_querying_or_disturbing_active_ui_state() {
        let mut app = App::new(state());
        app.state.plan_mode_active = false;
        app.state.approval = Some(ApprovalState {
            approval_id: "approval-1".to_owned(),
            tool_call_id: "call-1".to_owned(),
            tool_name: "write".to_owned(),
            input: json!({"path": "src/lib.rs"}),
            agent_id: None,
            agent_profile: None,
            model: None,
            goal_id: None,
            reason: None,
            risk: None,
            selected: 0,
            replying: false,
        });
        let snapshot = ContextSnapshot {
            usage_state: ContextUsageState::Actual,
            actual_tokens: Some(47_000),
            actual_percent: Some(47.0),
            context_window: Some(100_000),
            ..ContextSnapshot::default()
        };

        app.update(AppEvent::Host(RpcEvent {
            kind: "context_budget".to_owned(),
            payload: json!({
                "type": "context_budget",
                "snapshot": snapshot,
                "policyWarning": "Invalid context policy; defaults are active."
            }),
        }));

        assert_eq!(app.state().context.actual_percent, Some(47.0));
        assert!(!app.state().plan_mode_active);
        assert!(app.state().approval.is_some());
        assert!(matches!(
            app.state().transcript.last(),
            Some(TranscriptItem::Notice(message)) if message.contains("defaults")
        ));
    }

    #[test]
    fn new_resume_and_tree_are_local_commands_without_user_transcript_items() {
        for (command, expected) in [
            ("/new", AppEffect::NewSession),
            ("/resume", AppEffect::OpenSessionBrowser),
            (
                "/tree",
                AppEffect::GetTreeState {
                    filter_mode: TreeFilterMode::Default,
                    query: String::new(),
                    folded_entry_ids: Vec::new(),
                    generation: 0,
                },
            ),
        ] {
            let mut app = App::new(state());
            app.state.editor.insert_text(command);

            assert_eq!(app.update(press(KeyCode::Enter)), vec![expected]);
            assert!(
                !app.state
                    .transcript
                    .iter()
                    .any(|item| matches!(item, TranscriptItem::User(_))),
                "{command} must not enter the transcript"
            );
        }
    }

    #[test]
    fn session_browser_switches_scope_and_confirms_a_missing_working_directory() {
        let mut app = App::new(state());
        app.state.editor.insert_text("/resume");
        app.update(press(KeyCode::Enter));
        app.update(AppEvent::Command(CommandEvent::SessionBrowserOpened(Ok(
            Box::new(SessionBrowserSnapshot {
                browser_id: "browser-1".to_owned(),
                current_cwd: "/workspace/current".to_owned(),
                scope: SessionScope::Current,
                query: String::new(),
                sort_mode: SessionSortMode::Threaded,
                named_only: false,
                sessions: vec![
                    session_summary("/sessions/current.jsonl", "current", true, true),
                    session_summary("/sessions/old.jsonl", "old", false, false),
                ],
                total: 2,
                offset: 0,
                next_offset: None,
                truncated: false,
            }),
        ))));

        assert_eq!(
            app.update(press(KeyCode::Tab)),
            vec![AppEffect::QuerySessionBrowser {
                browser_id: "browser-1".to_owned(),
                scope: SessionScope::All,
                query: String::new(),
                sort_mode: SessionSortMode::Threaded,
                named_only: false,
                offset: 0,
                generation: 1,
            }]
        );
        app.update(press(KeyCode::Down));
        assert!(app.update(press(KeyCode::Enter)).is_empty());
        assert!(
            app.state
                .session_browser
                .as_ref()
                .is_some_and(|browser| browser.confirm_missing_cwd.is_some())
        );
        assert_eq!(
            app.update(press(KeyCode::Char('y'))),
            vec![AppEffect::ResumeSession {
                session_path: "/sessions/old.jsonl".to_owned(),
                cwd_override: Some("/workspace/current".to_owned()),
            }]
        );
        assert_eq!(app.state.run_state, RunState::SwitchingSession);
    }

    #[test]
    fn session_browser_fetches_and_appends_the_next_page_at_the_end() {
        let mut app = App::new(state());
        app.state.session_browser = Some(SessionBrowserState::loading());
        app.update(AppEvent::Command(CommandEvent::SessionBrowserOpened(Ok(
            Box::new(SessionBrowserSnapshot {
                browser_id: "browser-1".to_owned(),
                current_cwd: "/workspace/current".to_owned(),
                scope: SessionScope::Current,
                query: String::new(),
                sort_mode: SessionSortMode::Recent,
                named_only: false,
                sessions: vec![
                    session_summary("/sessions/0.jsonl", "session-0", false, true),
                    session_summary("/sessions/1.jsonl", "session-1", false, true),
                ],
                total: 3,
                offset: 0,
                next_offset: Some(2),
                truncated: true,
            }),
        ))));
        app.update(press(KeyCode::End));

        assert_eq!(
            app.update(press(KeyCode::Down)),
            vec![AppEffect::QuerySessionBrowser {
                browser_id: "browser-1".to_owned(),
                scope: SessionScope::Current,
                query: String::new(),
                sort_mode: SessionSortMode::Recent,
                named_only: false,
                offset: 2,
                generation: 1,
            }]
        );

        app.update(AppEvent::Command(
            CommandEvent::SessionBrowserQueryFinished {
                generation: 1,
                result: Ok(Box::new(SessionBrowserSnapshot {
                    browser_id: "browser-1".to_owned(),
                    current_cwd: "/workspace/current".to_owned(),
                    scope: SessionScope::Current,
                    query: String::new(),
                    sort_mode: SessionSortMode::Recent,
                    named_only: false,
                    sessions: vec![session_summary(
                        "/sessions/2.jsonl",
                        "session-2",
                        false,
                        true,
                    )],
                    total: 3,
                    offset: 2,
                    next_offset: None,
                    truncated: false,
                })),
            },
        ));

        let browser = app.state.session_browser.as_ref().expect("browser");
        assert_eq!(browser.sessions.len(), 3);
        assert_eq!(browser.selected, 2);
        assert_eq!(browser.next_offset, None);
    }

    #[test]
    fn resume_and_tree_paging_use_the_actual_selector_height() {
        let mut resume = App::new(state());
        resume.set_inline_viewport_height(30);
        resume.state.session_browser = Some(SessionBrowserState::loading());
        resume.update(AppEvent::Command(CommandEvent::SessionBrowserOpened(Ok(
            Box::new(SessionBrowserSnapshot {
                browser_id: "browser-1".to_owned(),
                current_cwd: "/workspace/current".to_owned(),
                scope: SessionScope::Current,
                query: String::new(),
                sort_mode: SessionSortMode::Recent,
                named_only: false,
                sessions: (0..30)
                    .map(|index| {
                        session_summary(
                            &format!("/sessions/{index}.jsonl"),
                            &format!("session-{index}"),
                            false,
                            true,
                        )
                    })
                    .collect(),
                total: 30,
                offset: 0,
                next_offset: None,
                truncated: false,
            }),
        ))));
        resume.update(press(KeyCode::PageDown));
        assert_eq!(
            resume
                .state
                .session_browser
                .as_ref()
                .map(|browser| browser.selected),
            Some(24)
        );
        resume.update(AppEvent::Terminal(TerminalEvent::Resize(120, 18)));
        resume.update(press(KeyCode::Home));
        resume.update(press(KeyCode::PageDown));
        assert_eq!(
            resume
                .state
                .session_browser
                .as_ref()
                .map(|browser| browser.selected),
            Some(12)
        );

        let mut tree = App::new(state());
        tree.set_inline_viewport_height(30);
        tree.state.tree_browser = Some(TreeBrowserState::loading());
        tree.update(AppEvent::Command(CommandEvent::TreeStateFinished {
            generation: 0,
            result: Ok(Box::new(TreeSnapshot {
                items: (0..30)
                    .map(|index| tree_item(&format!("entry-{index}"), None, index == 29, false))
                    .collect(),
                leaf_id: Some("entry-29".to_owned()),
                filter_mode: TreeFilterMode::Default,
                query: String::new(),
            })),
        }));
        tree.update(press(KeyCode::Home));
        tree.update(press(KeyCode::PageDown));
        let browser = tree.state.tree_browser.as_ref().expect("tree browser");
        assert_eq!(browser.selected, 24);
        assert_eq!(browser.selected_entry_id.as_deref(), Some("entry-24"));
    }

    #[test]
    fn session_activation_preserves_scrollback_and_replays_the_active_branch() {
        let mut app = App::new(state());
        app.state
            .transcript
            .push(TranscriptItem::Notice("existing scrollback".to_owned()));

        app.update(AppEvent::Command(CommandEvent::NewSessionFinished(Ok(
            Box::new(SessionCommandData {
                cancelled: false,
                activation: Some(activation("session-restored")),
            }),
        ))));

        assert_eq!(app.state.session.session_id, "session-restored");
        assert_eq!(app.state.run_state, RunState::Idle);
        assert_eq!(
            app.state.context.usage_state,
            ContextUsageState::Recalculating
        );
        assert_eq!(app.state.plan.as_ref().map(|plan| plan.revision), Some(2));
        assert!(matches!(
            app.state.transcript.first(),
            Some(TranscriptItem::Notice(text)) if text == "existing scrollback"
        ));
        assert!(app.state.transcript.iter().any(|item| matches!(
            item,
            TranscriptItem::SessionBoundary { action, label, cwd }
                if action == "new session"
                    && label == "Restored work"
                    && cwd == "/workspace/restored"
        )));
        assert!(app.state.transcript.iter().any(|item| matches!(
            item,
            TranscriptItem::User(UserMessage { text, status: UserMessageStatus::Accepted })
                if text == "restored question"
        )));
        assert!(app.state.transcript.iter().any(|item| matches!(
            item,
            TranscriptItem::Tool(ToolExecution {
                id,
                output,
                status: ToolStatus::Succeeded,
                ..
            }) if id == "tool-restored" && output == "restored source"
        )));
        assert!(app.state.transcript.iter().any(|item| matches!(
            item,
            TranscriptItem::Compaction(record)
                if record.reason == "restored" && record.tokens_before == 82_000
        )));
        assert!(app.state.transcript.iter().any(|item| matches!(
            item,
            TranscriptItem::BranchSummary(summary) if summary == "restored branch summary"
        )));
    }

    #[test]
    fn tree_browser_supports_pi_filters_copy_summary_and_abort_flow() {
        let mut app = App::new(state());
        app.state.editor.insert_text("/tree");
        app.update(press(KeyCode::Enter));
        app.update(AppEvent::Command(CommandEvent::TreeStateFinished {
            generation: 0,
            result: Ok(Box::new(TreeSnapshot {
                items: vec![
                    tree_item("branch", None, false, true),
                    tree_item("leaf", Some("branch"), true, false),
                ],
                leaf_id: Some("leaf".to_owned()),
                filter_mode: TreeFilterMode::Default,
                query: String::new(),
            })),
        }));

        assert_eq!(
            app.update(press_with(KeyCode::Char('t'), KeyModifiers::CONTROL)),
            vec![AppEffect::GetTreeState {
                filter_mode: TreeFilterMode::NoTools,
                query: String::new(),
                folded_entry_ids: Vec::new(),
                generation: 1,
            }]
        );
        assert_eq!(
            app.update(press_with(KeyCode::Char('t'), KeyModifiers::CONTROL)),
            vec![AppEffect::GetTreeState {
                filter_mode: TreeFilterMode::Default,
                query: String::new(),
                folded_entry_ids: Vec::new(),
                generation: 2,
            }]
        );

        app.update(press(KeyCode::Up));
        assert_eq!(
            app.update(press_with(KeyCode::Char('x'), KeyModifiers::CONTROL)),
            vec![AppEffect::CopyTreeEntry {
                entry_id: "branch".to_owned(),
            }]
        );
        assert!(app.update(press(KeyCode::Enter)).is_empty());
        app.update(press(KeyCode::Char('2')));
        assert_eq!(
            app.update(press(KeyCode::Enter)),
            vec![AppEffect::NavigateTree {
                entry_id: "branch".to_owned(),
                summarize: true,
                custom_instructions: None,
            }]
        );
        assert_eq!(app.state.run_state, RunState::SummarizingBranch);
        assert_eq!(
            app.update(press(KeyCode::Esc)),
            vec![AppEffect::AbortTreeNavigation]
        );
        assert!(matches!(
            app.state
                .tree_browser
                .as_ref()
                .map(|browser| &browser.phase),
            Some(TreePhase::Navigating {
                summarizing: true,
                aborting: true,
                ..
            })
        ));
    }

    #[test]
    fn successful_tree_navigation_restores_editor_and_appends_a_boundary() {
        let mut app = App::new(state());
        app.state
            .transcript
            .push(TranscriptItem::Notice("old scrollback".to_owned()));
        app.state.tree_browser = Some(TreeBrowserState::loading());
        app.state.run_state = RunState::NavigatingTree;

        app.update(AppEvent::Command(CommandEvent::TreeNavigateFinished(Ok(
            Box::new(TreeNavigateData {
                cancelled: false,
                aborted: false,
                editor_text: Some("recovered draft".to_owned()),
                activation: Some(activation("session-tree")),
            }),
        ))));

        assert!(app.state.tree_browser.is_none());
        assert_eq!(app.state.editor.text(), "recovered draft");
        assert!(matches!(
            app.state.transcript.first(),
            Some(TranscriptItem::Notice(text)) if text == "old scrollback"
        ));
        assert!(app.state.transcript.iter().any(|item| matches!(
            item,
            TranscriptItem::SessionBoundary { action, .. } if action == "tree navigation"
        )));
    }

    #[test]
    fn streaming_text_and_tool_lifecycle_update_transcript() {
        let mut app = App::new(state());
        app.update(AppEvent::Pi(RpcEvent {
            kind: "message_update".to_owned(),
            payload: json!({
                "type": "message_update",
                "assistantMessageEvent": {"type": "text_delta", "delta": "hello"}
            }),
        }));
        app.update(AppEvent::Pi(RpcEvent {
            kind: "tool_execution_start".to_owned(),
            payload: json!({
                "type": "tool_execution_start",
                "toolCallId": "tool-1",
                "toolName": "read"
            }),
        }));
        app.update(AppEvent::Pi(RpcEvent {
            kind: "tool_execution_end".to_owned(),
            payload: json!({
                "type": "tool_execution_end",
                "toolCallId": "tool-1",
                "isError": false
            }),
        }));

        assert!(matches!(
            &app.state().transcript[0],
            TranscriptItem::Assistant(message) if message.text == "hello"
        ));
        assert!(matches!(
            &app.state().transcript[1],
            TranscriptItem::Tool(tool) if tool.status == ToolStatus::Succeeded
        ));
    }

    #[test]
    fn transcript_viewer_is_local_preserves_editor_and_supports_modes_and_folding() {
        let mut app = App::new(state());
        app.state.editor.insert_text("unfinished draft");
        app.state
            .transcript
            .push(TranscriptItem::Tool(ToolExecution {
                id: "tool-1".to_owned(),
                name: "read".to_owned(),
                args: json!({"path": "src/lib.rs"}),
                output: "line one\nline two".to_owned(),
                status: ToolStatus::Succeeded,
            }));
        let transcript_len = app.state.transcript.len();

        assert!(
            app.update(press_with(KeyCode::Char('o'), KeyModifiers::CONTROL))
                .is_empty()
        );
        assert_eq!(app.state.active_modal_kind(), Some(UiModalKind::Transcript));
        assert_eq!(app.state.transcript.len(), transcript_len);

        app.update(press(KeyCode::Char('2')));
        assert_eq!(
            app.state
                .transcript_viewer
                .as_ref()
                .map(|viewer| viewer.mode),
            Some(TranscriptViewMode::Verbose)
        );
        app.update(press(KeyCode::Enter));
        assert_eq!(
            app.state
                .transcript_viewer
                .as_ref()
                .and_then(|viewer| viewer.tool_expansion_overrides.get("tool-1"))
                .copied(),
            Some(false)
        );

        app.update(press(KeyCode::Esc));
        assert!(app.state.transcript_viewer.is_none());
        assert_eq!(app.state.transcript_view_mode, TranscriptViewMode::Verbose);
        assert_eq!(app.state.editor.text(), "unfinished draft");
        assert_eq!(app.state.transcript.len(), transcript_len);
    }

    #[test]
    fn transcript_search_navigates_matches_without_mutating_history() {
        let mut app = App::new(state());
        app.state.transcript.extend([
            TranscriptItem::Notice("alpha first".to_owned()),
            TranscriptItem::Notice("unrelated".to_owned()),
            TranscriptItem::Notice("alpha second".to_owned()),
        ]);
        app.update(press_with(KeyCode::Char('o'), KeyModifiers::CONTROL));
        app.update(press(KeyCode::Char('/')));
        for character in "alpha".chars() {
            app.update(press(KeyCode::Char(character)));
        }

        let viewer = app.state.transcript_viewer.as_ref().unwrap();
        assert_eq!(viewer.search_matches, vec![0, 2]);
        assert_eq!(viewer.selected_item, Some(0));

        app.update(press(KeyCode::Enter));
        app.update(press(KeyCode::Char('n')));
        assert_eq!(
            app.state
                .transcript_viewer
                .as_ref()
                .and_then(|viewer| viewer.selected_item),
            Some(2)
        );
        assert_eq!(app.state.transcript.len(), 3);
    }
}
