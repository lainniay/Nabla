use super::*;

// INFO: Modal-local key handling is isolated from the primary command composer.
impl App {
    pub(super) fn update_selection_panel_key(&mut self, key: KeyEvent) -> Vec<AppEffect> {
        let Some(panel) = self.state.selection_panel.as_mut() else {
            return Vec::new();
        };
        if panel.loading {
            if key.code == KeyCode::Esc {
                self.state.selection_panel = None;
            }
            return Vec::new();
        }
        let enabled = vec![true; panel.options.len()];
        match update_choice_navigation(key, &mut panel.selected, &enabled) {
            ChoiceNavAction::Cancel => {
                self.state.selection_panel = None;
                Vec::new()
            }
            ChoiceNavAction::Confirm(_) => {
                let action = panel.selected_action().cloned();
                self.state.selection_panel = None;
                match action {
                    Some(SelectionPanelAction::SetModel { provider, model_id }) => {
                        vec![AppEffect::SetModel { provider, model_id }]
                    }
                    Some(SelectionPanelAction::SetThinking(level)) => {
                        vec![AppEffect::SetThinking(level)]
                    }
                    None => Vec::new(),
                }
            }
            ChoiceNavAction::Handled | ChoiceNavAction::Unhandled => Vec::new(),
        }
    }

    pub(super) fn update_permissions_key(&mut self, key: KeyEvent) -> Vec<AppEffect> {
        let Some(manager) = self.state.permission_manager.as_mut() else {
            return Vec::new();
        };
        match key.code {
            KeyCode::Esc => {
                self.state.permission_manager = None;
                Vec::new()
            }
            KeyCode::Up | KeyCode::BackTab => {
                manager.selected =
                    previous_wrapped(manager.selected, manager.snapshot.grants.len());
                Vec::new()
            }
            KeyCode::Down | KeyCode::Tab => {
                manager.selected = next_wrapped(manager.selected, manager.snapshot.grants.len());
                Vec::new()
            }
            KeyCode::Delete | KeyCode::Char('d' | 'D') => manager
                .snapshot
                .grants
                .get(manager.selected)
                .map(|rule| vec![AppEffect::RevokeApprovalRule(rule.id.clone())])
                .unwrap_or_default(),
            KeyCode::Char('c' | 'C') => vec![AppEffect::ClearApprovalRules],
            _ => Vec::new(),
        }
    }

    pub(super) fn update_integration_prompt_key(&mut self, key: KeyEvent) -> Vec<AppEffect> {
        let Some(prompt) = self.state.integration_prompt.as_mut() else {
            return Vec::new();
        };
        if prompt.submitting {
            return Vec::new();
        }
        let plain_character = !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER);
        let direct_action = match key.code {
            KeyCode::Char('a' | 'A') if plain_character => Some("apply"),
            KeyCode::Char('r' | 'R') if plain_character => Some("resolve"),
            KeyCode::Char('k' | 'K') if plain_character => Some("keep"),
            KeyCode::Char('d' | 'D') if plain_character => Some("discard"),
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

    pub(super) fn update_agent_picker_key(&mut self, key: KeyEvent) -> Vec<AppEffect> {
        let Some(picker) = self.state.agent_picker.as_mut() else {
            return Vec::new();
        };
        let enabled = vec![true; picker.profiles.len()];
        match update_choice_navigation(key, &mut picker.selected, &enabled) {
            ChoiceNavAction::Cancel => {
                self.state.agent_picker = None;
            }
            ChoiceNavAction::Confirm(_) => {
                let selected = picker
                    .selected_profile()
                    .map(|profile| profile.name.clone());
                self.state.agent_picker = None;
                if let Some(name) = selected {
                    self.state.editor.replace(format!("/agent {name} "));
                    self.state.reset_command_menu();
                }
            }
            ChoiceNavAction::Handled | ChoiceNavAction::Unhandled => {}
        }
        Vec::new()
    }

    pub(super) fn update_transcript_viewer_key(&mut self, key: KeyEvent) -> Vec<AppEffect> {
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
                        viewer.search_query.clear();
                        viewer.search_active = false;
                        close_search = true;
                        refresh = true;
                    }
                    KeyCode::Enter => {
                        viewer.search_active = false;
                        close_search = true;
                    }
                    KeyCode::Backspace => {
                        viewer.search_query.backspace();
                        refresh = true;
                    }
                    KeyCode::Delete => {
                        viewer.search_query.delete();
                        refresh = true;
                    }
                    KeyCode::Left => viewer.search_query.move_left(),
                    KeyCode::Right => viewer.search_query.move_right(),
                    KeyCode::Home => viewer.search_query.move_home(),
                    KeyCode::End => viewer.search_query.move_end(),
                    KeyCode::Char('u' | 'U') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        viewer.search_query.delete_to_line_start();
                        refresh = true;
                    }
                    KeyCode::Char(character)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::SUPER) =>
                    {
                        viewer.search_query.insert_char(character);
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

        if matches!(key.code, KeyCode::Esc)
            && self
                .state
                .transcript_viewer
                .as_ref()
                .is_some_and(|viewer| !viewer.search_query.text().is_empty())
        {
            if let Some(viewer) = self.state.transcript_viewer.as_mut() {
                viewer.search_query.clear();
            }
            self.refresh_transcript_search();
            return Vec::new();
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
            }
            KeyCode::Char('n')
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !viewer.search_matches.is_empty() =>
            {
                let next = viewer
                    .current_match
                    .map_or(0, |current| (current + 1) % viewer.search_matches.len());
                viewer.current_match = Some(next);
                viewer.selected_item = Some(viewer.search_matches[next]);
                viewer.scroll_to_selected = true;
                viewer.follow_tail = false;
            }
            KeyCode::Char('N')
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !viewer.search_matches.is_empty() =>
            {
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
                    .saturating_add(self.state.selection_page_size);
            }
            KeyCode::PageDown => {
                viewer.scroll_to_selected = false;
                viewer.scroll_from_bottom = viewer
                    .scroll_from_bottom
                    .saturating_sub(self.state.selection_page_size);
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
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Char('n' | 'N' | 'p' | 'P')
                if !tool_items.is_empty()
                    && (matches!(key.code, KeyCode::Tab | KeyCode::BackTab)
                        || key.modifiers.contains(KeyModifiers::CONTROL)) =>
            {
                let current = viewer
                    .selected_item
                    .and_then(|selected| tool_items.iter().position(|index| *index == selected))
                    .unwrap_or(0);
                let previous = matches!(key.code, KeyCode::BackTab)
                    || (key.code == KeyCode::Tab && key.modifiers.contains(KeyModifiers::SHIFT))
                    || (key.modifiers.contains(KeyModifiers::CONTROL)
                        && matches!(key.code, KeyCode::Char('p' | 'P')));
                let next = if previous {
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

    pub(super) fn refresh_transcript_search(&mut self) {
        let query = self
            .state
            .transcript_viewer
            .as_ref()
            .map(|viewer| viewer.search_query.text().to_lowercase())
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
}
