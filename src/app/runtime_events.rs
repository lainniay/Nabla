use super::*;

impl App {
    pub(super) fn update_runtime(&mut self, event: RuntimeEvent) -> Vec<AppEffect> {
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
                self.state.session_browser = None;
                self.state.tree_browser = None;
                self.state.selection_panel = None;
                self.state.agent_picker = None;
                self.state.integration_prompt = None;
                self.state.integration_prompt_queue.clear();
                self.state.open_agent_picker_on_agents = false;
                self.state.pending_plan_mode = None;
                self.state.pending_plan_prompt = None;
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
}
