use crate::state::{AppState, UiModalKind};

use super::types::SurfaceKind;

#[derive(Debug, Clone, Copy, Default)]
pub struct SurfaceManager;

impl SurfaceManager {
    pub fn route(self, state: &AppState) -> SurfaceKind {
        match state.active_modal_kind() {
            Some(
                UiModalKind::SessionBrowser
                | UiModalKind::TreeBrowser
                | UiModalKind::Transcript
                | UiModalKind::Auth,
            ) => SurfaceKind::Alternate,
            _ => SurfaceKind::Primary,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{
        rpc::PiState,
        state::{ApprovalState, SessionBrowserState},
    };

    use super::*;

    fn state() -> AppState {
        AppState::new(PiState {
            model: Some(json!({"provider": "test", "id": "model"})),
            thinking_level: "off".to_owned(),
            is_streaming: false,
            is_compacting: false,
            steering_mode: "one-at-a-time".to_owned(),
            follow_up_mode: "one-at-a-time".to_owned(),
            session_file: None,
            session_id: "session".to_owned(),
            session_name: None,
            auto_compaction_enabled: true,
            message_count: 0,
            pending_message_count: 0,
        })
    }

    #[test]
    fn only_large_browsers_use_alternate_while_all_approvals_stay_inline() {
        let mut state = state();
        assert_eq!(SurfaceManager.route(&state), SurfaceKind::Primary);
        state.session_browser = Some(SessionBrowserState::loading());
        assert_eq!(SurfaceManager.route(&state), SurfaceKind::Alternate);
        state.session_browser = None;
        state.approval = Some(ApprovalState {
            approval_id: "approval".to_owned(),
            tool_call_id: "call".to_owned(),
            tool_name: "bash".to_owned(),
            input: json!({}),
            agent_id: None,
            agent_profile: None,
            model: None,
            goal_id: None,
            reason: None,
            risk: Some("credential".to_owned()),
            selected: 0,
            replying: false,
        });
        assert_eq!(SurfaceManager.route(&state), SurfaceKind::Primary);
    }
}
