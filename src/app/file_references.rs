use super::*;

impl App {
    pub(super) fn refresh_file_completion(&mut self) -> Option<AppEffect> {
        let Some(token) = token_at_cursor(self.state.editor.text(), self.state.editor.cursor())
        else {
            self.state.file_completion = None;
            return None;
        };
        self.state.file_completion_generation =
            self.state.file_completion_generation.saturating_add(1);
        let generation = self.state.file_completion_generation;
        let query = token.path;
        if self
            .state
            .file_completion
            .as_ref()
            .is_some_and(|completion| {
                completion.query == query && completion.token_range == token.range
            })
        {
            return None;
        }
        let (candidates, selected) = self
            .state
            .file_completion
            .take()
            .filter(|completion| completion.token_range.start == token.range.start)
            .map_or_else(
                || (Vec::new(), 0),
                |completion| (completion.candidates, completion.selected),
            );
        self.state.file_completion = Some(FileCompletionState {
            query: query.clone(),
            token_range: token.range,
            generation,
            candidates,
            selected,
            loading: true,
            error: None,
        });
        Some(AppEffect::SearchFiles { query, generation })
    }

    pub(super) fn select_previous_file(&mut self) {
        if let Some(completion) = self.state.file_completion.as_mut()
            && !completion.candidates.is_empty()
        {
            completion.selected =
                previous_wrapped(completion.selected, completion.candidates.len());
        }
    }

    pub(super) fn select_next_file(&mut self) {
        if let Some(completion) = self.state.file_completion.as_mut()
            && !completion.candidates.is_empty()
        {
            completion.selected = next_wrapped(completion.selected, completion.candidates.len());
        }
    }

    pub(super) fn accept_file_completion(&mut self) {
        let Some(completion) = self.state.file_completion.take() else {
            return;
        };
        let Some(candidate) = completion.candidates.get(completion.selected) else {
            return;
        };
        let mut token_range = completion.token_range;
        if self.state.editor.text()[token_range.end..].starts_with(' ') {
            token_range.end += 1;
        }
        let replacement = format!("{} ", completion_text(&candidate.path));
        self.state
            .editor
            .replace_byte_range(token_range, &replacement);
        self.state.reset_command_menu();
    }

    pub(super) fn prepare_delivery(
        &mut self,
        message: String,
        delivery: PromptDelivery,
    ) -> Vec<AppEffect> {
        if !reference_tokens(&message)
            .iter()
            .any(|token| token.braced || !token.path.is_empty())
        {
            self.state.editor.clear();
            self.state.file_completion = None;
            self.push_user(message.clone(), UserMessageStatus::Pending);
            if delivery == PromptDelivery::Prompt {
                self.state.run_state = RunState::Submitting;
            }
            return vec![match delivery {
                PromptDelivery::Prompt => AppEffect::Prompt(message),
                PromptDelivery::Steer => AppEffect::Steer(message),
                PromptDelivery::FollowUp => AppEffect::FollowUp(message),
            }];
        }
        self.state.run_state = RunState::PreparingReferences;
        self.state.file_completion = None;
        vec![AppEffect::PrepareReferences { message, delivery }]
    }
}
