use super::*;

/// Complete state managed by the application reducer.
// INFO: This aggregate owns data only; protocol handling and side effects stay
// in `App`, which keeps state serialization and transitions independently testable.
pub struct AppState {
    pub editor: EditorState,
    pub file_completion: Option<FileCompletionState>,
    pub transcript: Vec<TranscriptItem>,
    /// Changes whenever canonical session state is replaced.
    pub session_epoch: u64,
    pub(crate) next_assistant_message_id: u64,
    pub session: PiState,
    pub run_state: RunState,
    pub connection_state: ConnectionState,
    pub last_error: Option<String>,
    pub command_catalog: CommandCatalog,
    pub auth_state: AuthState,
    pub plan_mode_active: bool,
    pub pending_plan_mode: Option<bool>,
    pub pending_plan_prompt: Option<String>,
    pub approval: Option<ApprovalState>,
    pub permission_manager: Option<PermissionManagerState>,
    pub question: Option<QuestionFlowState>,
    pub plan: Option<PlanArtifact>,
    pub plan_review: Option<PlanReviewState>,
    pub session_browser: Option<SessionBrowserState>,
    pub tree_browser: Option<TreeBrowserState>,
    pub transcript_viewer: Option<TranscriptViewerState>,
    pub transcript_view_mode: TranscriptViewMode,
    pub context: ContextSnapshot,
    pub resources: ResourceSnapshot,
    pub agents: AgentsSnapshot,
    pub selection_panel: Option<SelectionPanelState>,
    pub agent_picker: Option<AgentPickerState>,
    pub integration_prompt: Option<IntegrationPromptState>,
    pub integration_prompt_queue: VecDeque<IntegrationPromptState>,
    pub open_agent_picker_on_agents: bool,
    pub next_auth_flow_id: u64,
    pub selection_page_size: usize,
    pub(crate) seen_compactions: HashSet<String>,
    pub(crate) compact_lifecycle_finished: bool,
    command_menu_selected: usize,
    command_menu_dismissed: bool,
    pub(crate) file_completion_generation: u64,
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
            file_completion: None,
            transcript: Vec::new(),
            session_epoch: 0,
            next_assistant_message_id: 1,
            session,
            run_state,
            connection_state: ConnectionState::Connected,
            last_error: None,
            command_catalog: CommandCatalog::new(commands),
            auth_state: AuthState::Inactive,
            plan_mode_active: false,
            pending_plan_mode: None,
            pending_plan_prompt: None,
            approval: None,
            permission_manager: None,
            question: None,
            plan: None,
            plan_review: None,
            session_browser: None,
            tree_browser: None,
            transcript_viewer: None,
            transcript_view_mode: TranscriptViewMode::Normal,
            context: ContextSnapshot::default(),
            resources: ResourceSnapshot::default(),
            agents: AgentsSnapshot::default(),
            selection_panel: None,
            agent_picker: None,
            integration_prompt: None,
            integration_prompt_queue: VecDeque::new(),
            open_agent_picker_on_agents: false,
            next_auth_flow_id: 1,
            selection_page_size: 8,
            seen_compactions: HashSet::new(),
            compact_lifecycle_finished: false,
            command_menu_selected: 0,
            command_menu_dismissed: false,
            file_completion_generation: 0,
        }
    }

    pub fn model_label(&self) -> String {
        let Some(model) = self.session.model.as_ref() else {
            return "no model".to_owned();
        };
        let name = model
            .get("name")
            .or_else(|| model.get("id"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        name.to_owned()
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
        } else if self.selection_panel.is_some() {
            Some(UiModalKind::Selection)
        } else if self.agent_picker.is_some() {
            Some(UiModalKind::AgentPicker)
        } else if self.question.is_some() {
            Some(UiModalKind::Question)
        } else if self.auth_state.is_active() {
            Some(UiModalKind::Auth)
        } else if self.transcript_viewer.is_some() {
            Some(UiModalKind::Transcript)
        } else if self.approval.is_some() {
            Some(UiModalKind::Approval)
        } else if self.permission_manager.is_some() {
            Some(UiModalKind::Permissions)
        } else if self.integration_prompt.is_some() {
            Some(UiModalKind::Integration)
        } else if self.plan_review.is_some() {
            Some(UiModalKind::PlanReview)
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

    pub(crate) fn reset_command_menu(&mut self) {
        self.command_menu_selected = 0;
        self.command_menu_dismissed = false;
    }

    pub(crate) fn dismiss_command_menu(&mut self) {
        self.command_menu_dismissed = true;
    }
}
