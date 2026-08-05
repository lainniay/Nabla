use super::*;

// INFO: Session and tree navigation share activation/history restoration invariants.
impl App {
    pub(super) fn update_session_browser_key(&mut self, key: KeyEvent) -> Vec<AppEffect> {
        let page_rows = self.state.selection_page_size;
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

        if browser.search_active {
            let mut refresh = false;
            match key.code {
                KeyCode::Esc => {
                    refresh = !browser.query.text().is_empty();
                    browser.query.clear();
                    browser.search_active = false;
                }
                KeyCode::Enter => browser.search_active = false,
                KeyCode::Backspace => {
                    browser.query.backspace();
                    refresh = true;
                }
                KeyCode::Delete => {
                    browser.query.delete();
                    refresh = true;
                }
                KeyCode::Left => browser.query.move_left(),
                KeyCode::Right => browser.query.move_right(),
                KeyCode::Home => browser.query.move_home(),
                KeyCode::End => browser.query.move_end(),
                KeyCode::Char('u' | 'U') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    browser.query.clear();
                    refresh = true;
                }
                KeyCode::Char(character)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::SUPER) =>
                {
                    browser.query.insert_char(character);
                    refresh = true;
                }
                _ => return Vec::new(),
            }
            if refresh {
                browser.selected = 0;
                return self.refresh_session_browser_effect().into_iter().collect();
            }
            return Vec::new();
        }

        if is_previous_selection_key(key) {
            if !browser.sessions.is_empty() {
                browser.selected = previous_wrapped(browser.selected, browser.sessions.len());
            }
            return Vec::new();
        }
        if is_next_selection_key(key) {
            if !browser.sessions.is_empty() {
                if browser.selected + 1 < browser.sessions.len() {
                    browser.selected += 1;
                } else if browser.next_offset.is_some() {
                    return self.load_more_sessions_effect().into_iter().collect();
                } else {
                    browser.selected = 0;
                }
            }
            return Vec::new();
        }

        match key.code {
            KeyCode::Esc => {
                if !browser.query.text().is_empty() {
                    browser.query.clear();
                    browser.selected = 0;
                    return self.refresh_session_browser_effect().into_iter().collect();
                }
                let browser_id = browser.browser_id.clone();
                self.state.session_browser = None;
                return browser_id.map_or_else(Vec::new, |browser_id| {
                    vec![AppEffect::CloseSessionBrowser { browser_id }]
                });
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
            KeyCode::Char('/') => browser.search_active = true,
            KeyCode::Char('w' | 'W') => {
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
            KeyCode::Char('m' | 'M') => {
                browser.named_only = !browser.named_only;
                browser.selected = 0;
                return self.refresh_session_browser_effect().into_iter().collect();
            }
            KeyCode::Char('p' | 'P') => {
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
            _ => {}
        }
        Vec::new()
    }

    pub(super) fn update_tree_browser_key(&mut self, key: KeyEvent) -> Vec<AppEffect> {
        let page_rows = self.state.selection_page_size;
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
                if is_previous_selection_key(key) {
                    selected = previous_wrapped(selected, 3);
                } else if is_next_selection_key(key) {
                    selected = next_wrapped(selected, 3);
                } else {
                    match key.code {
                        KeyCode::Esc => {
                            if let Some(browser) = self.state.tree_browser.as_mut() {
                                browser.phase = TreePhase::Browse;
                            }
                            return Vec::new();
                        }
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
        if browser.search_active {
            let mut refresh = false;
            match key.code {
                KeyCode::Esc => {
                    refresh = !browser.query.text().is_empty();
                    browser.query.clear();
                    browser.search_active = false;
                }
                KeyCode::Enter => browser.search_active = false,
                KeyCode::Backspace => {
                    browser.query.backspace();
                    refresh = true;
                }
                KeyCode::Delete => {
                    browser.query.delete();
                    refresh = true;
                }
                KeyCode::Left => browser.query.move_left(),
                KeyCode::Right => browser.query.move_right(),
                KeyCode::Home => browser.query.move_home(),
                KeyCode::End => browser.query.move_end(),
                KeyCode::Char('u' | 'U') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    browser.query.clear();
                    refresh = true;
                }
                KeyCode::Char(character)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::SUPER) =>
                {
                    browser.query.insert_char(character);
                    refresh = true;
                }
                _ => return Vec::new(),
            }
            if refresh {
                browser.selected = 0;
                return self.refresh_tree_effect().into_iter().collect();
            }
            return Vec::new();
        }
        let branch_modifier = key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        if is_previous_selection_key(key) {
            if !browser.items.is_empty() {
                browser.selected = previous_wrapped(browser.selected, browser.items.len());
                browser.selected_entry_id =
                    browser.selected_item().map(|item| item.entry_id.clone());
            }
            return Vec::new();
        }
        if is_next_selection_key(key) {
            if !browser.items.is_empty() {
                browser.selected = next_wrapped(browser.selected, browser.items.len());
                browser.selected_entry_id =
                    browser.selected_item().map(|item| item.entry_id.clone());
            }
            return Vec::new();
        }
        match key.code {
            KeyCode::Esc => {
                if !browser.query.text().is_empty() {
                    browser.query.clear();
                    browser.selected = 0;
                    return self.refresh_tree_effect().into_iter().collect();
                }
                self.state.tree_browser = None;
            }
            KeyCode::Char('/') => browser.search_active = true,
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
            _ => {}
        }
        Vec::new()
    }

    pub(super) fn refresh_session_browser_effect(&mut self) -> Option<AppEffect> {
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

    pub(super) fn load_more_sessions_effect(&mut self) -> Option<AppEffect> {
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

    pub(super) fn refresh_tree_effect(&mut self) -> Option<AppEffect> {
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

    pub(super) fn start_tree_navigation(
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

    pub(super) fn apply_session_browser_snapshot(&mut self, snapshot: SessionBrowserSnapshot) {
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

    pub(super) fn apply_tree_snapshot(&mut self, snapshot: TreeSnapshot) {
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

    pub(super) fn apply_activation(&mut self, _action: &str, activation: SessionActivationData) {
        let SessionActivationData {
            state,
            cwd: _,
            plan_mode,
            goal,
            history,
            plan,
            context,
        } = activation;
        let epoch = self.state.session_epoch.saturating_add(1);
        let mut next_assistant_message_id = 1u64;
        let mut transcript = Vec::with_capacity(history.len());
        for item in history {
            append_history_item_to(&mut transcript, item, epoch, &mut next_assistant_message_id);
        }

        // Construct the target transcript before publishing any session fields.
        // Observers therefore see either the previous canonical session or the
        // complete replacement, never an append-only mixture.
        self.state.session = state;
        self.state.session_epoch = epoch;
        self.state.next_assistant_message_id = next_assistant_message_id;
        self.state.plan_mode_active = plan_mode;
        self.state.context = context;
        self.state.plan = plan;
        self.state.goal = Some(goal);
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
        self.state.seen_compactions = transcript
            .iter()
            .filter_map(|item| match item {
                TranscriptItem::Compaction(record) => Some(record.deduplication_key()),
                _ => None,
            })
            .collect();
        self.state.compact_lifecycle_finished = false;
        self.state.run_state = RunState::Idle;
        self.state.last_error = None;
        self.state.transcript = transcript;
        self.state.transcript_viewer = None;
    }

    #[cfg(test)]
    pub(super) fn append_history_item(&mut self, item: SessionHistoryItem) {
        append_history_item_to(
            &mut self.state.transcript,
            item,
            self.state.session_epoch,
            &mut self.state.next_assistant_message_id,
        );
        if let Some(TranscriptItem::Compaction(record)) = self.state.transcript.last() {
            self.state
                .seen_compactions
                .insert(record.deduplication_key());
        }
    }
}

fn append_history_item_to(
    transcript: &mut Vec<TranscriptItem>,
    item: SessionHistoryItem,
    session_epoch: u64,
    next_assistant_message_id: &mut u64,
) {
    match item {
        SessionHistoryItem::User { text } => {
            transcript.push(TranscriptItem::User(UserMessage {
                text,
                status: UserMessageStatus::Accepted,
            }));
        }
        SessionHistoryItem::Assistant { text, thinking } => {
            let id = *next_assistant_message_id;
            *next_assistant_message_id = next_assistant_message_id.saturating_add(1);
            transcript.push(TranscriptItem::Assistant(AssistantMessage {
                id,
                session_epoch,
                text_revision: u64::from(!text.is_empty()),
                thinking_revision: u64::from(!thinking.is_empty()),
                text,
                thinking,
                complete: true,
            }));
        }
        SessionHistoryItem::ToolCall { id, name, args } => {
            transcript.push(TranscriptItem::Tool(ToolExecution {
                id,
                name,
                args,
                output: String::new(),
                diff: None,
                status: ToolStatus::Running,
            }));
        }
        SessionHistoryItem::ToolResult {
            id,
            name,
            output,
            details,
            is_error,
        } => {
            if let Some(tool) = transcript.iter_mut().rev().find_map(|item| match item {
                TranscriptItem::Tool(tool) if tool.id == id => Some(tool),
                _ => None,
            }) {
                tool.output = output;
                tool.diff = (!is_error)
                    .then(|| {
                        details
                            .as_ref()
                            .and_then(|details| parse_tool_diff(&tool.args, details))
                    })
                    .flatten();
                tool.status = if is_error {
                    ToolStatus::Failed
                } else {
                    ToolStatus::Succeeded
                };
            } else {
                transcript.push(TranscriptItem::Tool(ToolExecution {
                    id,
                    name,
                    args: serde_json::Value::Null,
                    output,
                    diff: (!is_error)
                        .then(|| {
                            details.as_ref().and_then(|details| {
                                parse_tool_diff(&serde_json::Value::Null, details)
                            })
                        })
                        .flatten(),
                    status: if is_error {
                        ToolStatus::Failed
                    } else {
                        ToolStatus::Succeeded
                    },
                }));
            }
        }
        SessionHistoryItem::Notice { text } => {
            transcript.push(TranscriptItem::Notice(text));
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
            transcript.push(TranscriptItem::Compaction(record));
        }
        SessionHistoryItem::TurnBoundary {
            turn_id,
            started_at,
            ended_at,
            duration_ms,
            estimated,
        } => {
            transcript.push(TranscriptItem::TurnSeparator(TurnSeparator {
                turn_id,
                started_at,
                ended_at,
                duration_ms,
                estimated,
            }));
        }
        SessionHistoryItem::BranchSummary { summary } => {
            transcript.push(TranscriptItem::BranchSummary(summary));
        }
    }
}
