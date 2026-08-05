use super::*;

// INFO: Approval, planning, question, and authentication flows are explicit state machines.
impl App {
    pub(super) fn update_question_key(&mut self, key: KeyEvent) -> Vec<AppEffect> {
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

    pub(super) fn answer_current_question(
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

    pub(super) fn update_plan_review_key(&mut self, key: KeyEvent) -> Vec<AppEffect> {
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
                let plain_character = !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER);
                if plain_character && matches!(key.code, KeyCode::Char('n' | 'N')) {
                    let selected = usize::from(target == PlanExecutionTarget::Fresh);
                    self.state.plan_review = Some(PlanReviewState::Menu { selected });
                    return Vec::new();
                }
                if !plain_character || !matches!(key.code, KeyCode::Char('y' | 'Y')) {
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

    pub(super) fn update_goal_approval_key(&mut self, key: KeyEvent) -> Vec<AppEffect> {
        let Some(approval) = self.state.goal_approval.as_mut() else {
            return Vec::new();
        };
        if approval.submitting {
            return Vec::new();
        }
        let plain_character = !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER);
        if plain_character && matches!(key.code, KeyCode::Char('n' | 'N')) {
            self.state.goal_approval = None;
            return Vec::new();
        }
        if plain_character && matches!(key.code, KeyCode::Char('y' | 'Y')) {
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

    pub(super) fn choose_plan_review(&mut self, selected: usize) -> Vec<AppEffect> {
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

    pub(super) fn update_approval_key(&mut self, key: KeyEvent) -> Vec<AppEffect> {
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('o' | 'O'))
        {
            self.state.transcript_viewer = Some(TranscriptViewerState::new(
                self.state.transcript_view_mode,
                &self.state.transcript,
            ));
            return Vec::new();
        }
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
        let plain_character = !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER);
        let direct_decision = match key.code {
            KeyCode::Char('y' | 'Y')
                if plain_character
                    && approval
                        .available_decisions
                        .contains(&ApprovalDecision::AllowOnce) =>
            {
                Some(ApprovalDecision::AllowOnce)
            }
            KeyCode::Char('s' | 'S')
                if plain_character
                    && approval
                        .available_decisions
                        .contains(&ApprovalDecision::AllowSession) =>
            {
                Some(ApprovalDecision::AllowSession)
            }
            KeyCode::Char('a' | 'A')
                if plain_character
                    && approval
                        .available_decisions
                        .contains(&ApprovalDecision::AllowWorkspace) =>
            {
                Some(ApprovalDecision::AllowWorkspace)
            }
            KeyCode::Char('n' | 'N')
                if plain_character
                    && approval
                        .available_decisions
                        .contains(&ApprovalDecision::Deny) =>
            {
                Some(ApprovalDecision::Deny)
            }
            _ => None,
        };
        let enabled = vec![true; approval.available_decisions.len()];
        let decision = if let Some(decision) = direct_decision {
            Some(decision)
        } else {
            match update_choice_navigation(key, &mut approval.selected, &enabled) {
                ChoiceNavAction::Handled => return Vec::new(),
                ChoiceNavAction::Confirm(selected) => {
                    approval.available_decisions.get(selected).copied()
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

    pub(super) fn toggle_plan_mode(&mut self, active: bool) -> Vec<AppEffect> {
        if !self.state.can_toggle_plan_mode() {
            return Vec::new();
        }
        if active == self.state.plan_mode_active {
            return Vec::new();
        }

        self.state.pending_plan_mode = Some(active);
        vec![AppEffect::SetPlanMode(active)]
    }

    pub(super) fn update_auth_key(&mut self, key: KeyEvent) -> Vec<AppEffect> {
        if let AuthState::Selecting {
            selected,
            filter,
            search_active,
            ..
        } = &mut self.state.auth_state
            && *search_active
        {
            match key.code {
                KeyCode::Esc => {
                    filter.clear();
                    *selected = 0;
                    *search_active = false;
                }
                KeyCode::Enter => {
                    *search_active = false;
                }
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
            return Vec::new();
        }

        let cancel = matches!(key.code, KeyCode::Esc)
            || (key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('c' | 'C')));
        if cancel {
            if let AuthState::Selecting {
                selected, filter, ..
            } = &mut self.state.auth_state
                && !filter.text().is_empty()
            {
                filter.clear();
                *selected = 0;
                return Vec::new();
            }
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
                search_active,
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
                if key.code == KeyCode::Char('/') {
                    *search_active = true;
                }
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
}
