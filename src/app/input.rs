use super::*;

impl App {
    pub(super) fn update_key(&mut self, key: KeyEvent) -> Vec<AppEffect> {
        if let Some(modal) = self.state.active_modal_kind() {
            return match modal {
                UiModalKind::SessionBrowser => self.update_session_browser_key(key),
                UiModalKind::TreeBrowser => self.update_tree_browser_key(key),
                UiModalKind::Selection => self.update_selection_panel_key(key),
                UiModalKind::AgentPicker => self.update_agent_picker_key(key),
                UiModalKind::Transcript => self.update_transcript_viewer_key(key),
                UiModalKind::Question => self.update_question_key(key),
                UiModalKind::Auth => self.update_auth_key(key),
                UiModalKind::Approval => self.update_approval_key(key),
                UiModalKind::Permissions => self.update_permissions_key(key),
                UiModalKind::Integration => self.update_integration_prompt_key(key),
                UiModalKind::PlanReview => self.update_plan_review_key(key),
            };
        }

        if self.state.run_state == RunState::PreparingReferences {
            return Vec::new();
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

        if self.state.file_completion.is_some() {
            if is_previous_selection_key(key) {
                self.select_previous_file();
                return Vec::new();
            }
            if is_next_selection_key(key) {
                self.select_next_file();
                return Vec::new();
            }
            match key.code {
                KeyCode::Enter => {
                    self.accept_file_completion();
                    return Vec::new();
                }
                KeyCode::Esc => {
                    self.state.file_completion = None;
                    return Vec::new();
                }
                _ => {}
            }
        }

        if !self.state.command_candidates().is_empty() {
            if is_previous_selection_key(key) {
                self.state.select_previous_command();
                return Vec::new();
            }
            if is_next_selection_key(key) {
                self.state.select_next_command();
                return Vec::new();
            }
        }

        match key.code {
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.state.editor.insert_newline();
                self.state.reset_command_menu();
                self.refresh_file_completion().into_iter().collect()
            }
            KeyCode::Char('j' | 'J') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.state.editor.insert_newline();
                self.state.reset_command_menu();
                self.refresh_file_completion().into_iter().collect()
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
                let message = self.state.editor.text().to_owned();
                self.state.reset_command_menu();
                if message.trim().is_empty() {
                    return Vec::new();
                }
                if key.modifiers.contains(KeyModifiers::ALT) {
                    self.prepare_delivery(message, PromptDelivery::FollowUp)
                } else {
                    self.prepare_delivery(message, PromptDelivery::Steer)
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
                let message = self.state.editor.text().to_owned();
                self.state.reset_command_menu();
                if message.trim().is_empty() {
                    return Vec::new();
                }
                self.submit(message)
            }
            KeyCode::BackTab => self.toggle_plan_mode(!self.state.plan_mode_active),
            KeyCode::Up => {
                self.state.editor.move_up(usize::MAX);
                self.refresh_file_completion().into_iter().collect()
            }
            KeyCode::Down => {
                self.state.editor.move_down(usize::MAX);
                self.refresh_file_completion().into_iter().collect()
            }
            KeyCode::Backspace => {
                self.state.editor.backspace();
                self.state.reset_command_menu();
                self.refresh_file_completion().into_iter().collect()
            }
            KeyCode::Delete => {
                self.state.editor.delete();
                self.state.reset_command_menu();
                self.refresh_file_completion().into_iter().collect()
            }
            KeyCode::Left => {
                self.state.editor.move_left();
                self.state.reset_command_menu();
                self.refresh_file_completion().into_iter().collect()
            }
            KeyCode::Right => {
                self.state.editor.move_right();
                self.state.reset_command_menu();
                self.refresh_file_completion().into_iter().collect()
            }
            KeyCode::Home => {
                self.state.editor.move_home();
                self.state.reset_command_menu();
                self.refresh_file_completion().into_iter().collect()
            }
            KeyCode::End => {
                self.state.editor.move_end();
                self.state.reset_command_menu();
                self.refresh_file_completion().into_iter().collect()
            }
            KeyCode::Char('u' | 'U') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.state.editor.clear();
                self.state.file_completion = None;
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
                self.refresh_file_completion().into_iter().collect()
            }
            _ => Vec::new(),
        }
    }
}
