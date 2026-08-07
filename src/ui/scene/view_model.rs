use crate::{
    command::CommandSpec,
    host::SandboxStatusData,
    rpc::PiState,
    state::{
        AgentPickerState, AppState, ApprovalState, AuthState, ConnectionState, ContextSnapshot,
        EditorState, FileCompletionState, IntegrationPromptState, PermissionManagerState,
        PlanReviewState, QuestionFlowState, RunState, SelectionPanelState, SessionBrowserState,
        TranscriptItem, TranscriptViewerState, TreeBrowserState, UiModalKind,
    },
};

/// Projection of `AppState` consumed by scene renderers. Only this module
/// touches the full application state; panels/composer/status see a view.
pub struct SceneViewModel<'a> {
    domain: &'a AppState,
    pub run_state: &'a RunState,
    pub transcript: &'a [TranscriptItem],
    pub session: &'a PiState,
    pub connection_state: &'a ConnectionState,
    pub auth_state: &'a AuthState,
    pub plan_mode_active: &'a bool,
    pub sandbox_status: &'a SandboxStatusData,
    pub approval: &'a Option<ApprovalState>,
    pub permission_manager: &'a Option<PermissionManagerState>,
    pub question: &'a Option<QuestionFlowState>,
    pub plan_review: &'a Option<PlanReviewState>,
    pub session_browser: &'a Option<SessionBrowserState>,
    pub tree_browser: &'a Option<TreeBrowserState>,
    pub transcript_viewer: &'a Option<TranscriptViewerState>,
    pub context: &'a ContextSnapshot,
    pub selection_panel: &'a Option<SelectionPanelState>,
    pub agent_picker: &'a Option<AgentPickerState>,
    pub integration_prompt: &'a Option<IntegrationPromptState>,
    pub editor: &'a EditorState,
    pub file_completion: &'a Option<FileCompletionState>,
    pub selection_page_size: usize,
}

impl<'a> SceneViewModel<'a> {
    pub fn from_domain(domain: &'a AppState) -> Self {
        Self {
            domain,
            run_state: &domain.run_state,
            transcript: &domain.transcript,
            session: &domain.session,
            connection_state: &domain.connection_state,
            auth_state: &domain.auth_state,
            plan_mode_active: &domain.plan_mode_active,
            sandbox_status: &domain.sandbox_status,
            approval: &domain.approval,
            permission_manager: &domain.permission_manager,
            question: &domain.question,
            plan_review: &domain.plan_review,
            session_browser: &domain.session_browser,
            tree_browser: &domain.tree_browser,
            transcript_viewer: &domain.transcript_viewer,
            context: &domain.context,
            selection_panel: &domain.selection_panel,
            agent_picker: &domain.agent_picker,
            integration_prompt: &domain.integration_prompt,
            editor: &domain.editor,
            file_completion: &domain.file_completion,
            selection_page_size: domain.selection_page_size,
        }
    }

    pub fn active_modal_kind(&self) -> Option<UiModalKind> {
        self.domain.active_modal_kind()
    }

    pub fn command_menu_selected(&self) -> usize {
        self.domain.command_menu_selected()
    }

    pub fn model_label(&self) -> String {
        self.domain.model_label()
    }

    pub fn command_candidates(&self) -> Vec<&'a CommandSpec> {
        self.domain.command_candidates()
    }
}
