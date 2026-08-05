use super::*;

// INFO: Command completions are reduced here instead of mutating state inside
// async tasks, preserving ordering with Pi and host lifecycle events.
impl App {
    pub(super) fn update_command(&mut self, event: CommandEvent) -> Vec<AppEffect> {
        match event {
            CommandEvent::FileSearchFinished { generation, result } => {
                let Some(completion) = self.state.file_completion.as_mut() else {
                    return Vec::new();
                };
                if completion.generation != generation {
                    return Vec::new();
                }
                completion.loading = false;
                match result {
                    Ok(candidates) => {
                        let selected_path = completion
                            .candidates
                            .get(completion.selected)
                            .map(|candidate| candidate.path.as_str());
                        completion.selected = selected_path
                            .and_then(|path| {
                                candidates
                                    .iter()
                                    .position(|candidate| candidate.path == path)
                            })
                            .unwrap_or(0);
                        completion.candidates = candidates;
                        completion.error = None;
                    }
                    Err(error) => {
                        completion.candidates.clear();
                        completion.error = Some(error);
                    }
                }
            }
            CommandEvent::ReferencesPrepared { delivery, result } => match result {
                Ok(prompt) => {
                    if self.state.editor.text() == prompt.original_message {
                        self.state.editor.clear();
                    }
                    self.push_user(prompt.original_message.clone(), UserMessageStatus::Pending);
                    self.state.run_state = if delivery == PromptDelivery::Prompt {
                        RunState::Submitting
                    } else {
                        RunState::Running
                    };
                    return vec![AppEffect::DeliverPrepared { prompt, delivery }];
                }
                Err(error) => {
                    self.state.run_state = if self.state.session.is_streaming {
                        RunState::Running
                    } else {
                        RunState::Idle
                    };
                    self.state.last_error = Some(error.clone());
                    self.state.transcript.push(TranscriptItem::Error(format!(
                        "Unable to prepare file references: {error}"
                    )));
                }
            },
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
            CommandEvent::ApprovalRulesFinished(result)
            | CommandEvent::ApprovalRuleRevoked(result)
            | CommandEvent::ApprovalRulesCleared(result) => match result {
                Ok(snapshot) => {
                    let selected = self
                        .state
                        .permission_manager
                        .as_ref()
                        .map_or(0, |manager| manager.selected)
                        .min(snapshot.grants.len().saturating_sub(1));
                    self.state.permission_manager = Some(PermissionManagerState {
                        snapshot: *snapshot,
                        selected,
                    });
                }
                Err(error) => self.set_error(format!("Unable to update permissions: {error}")),
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
                    if !self.state.selection_panel.as_ref().is_some_and(|panel| {
                        panel.kind == SelectionPanelKind::Model && panel.loading
                    }) {
                        return Vec::new();
                    }
                    let current = data.current.as_ref().and_then(|value| {
                        Some((value.get("provider")?.as_str()?, value.get("id")?.as_str()?))
                    });
                    let mut selected = 0;
                    let options = data
                        .models
                        .into_iter()
                        .enumerate()
                        .map(|(index, model)| {
                            let is_current = current.is_some_and(|(provider, id)| {
                                provider == model.provider && id == model.id
                            });
                            if is_current {
                                selected = index;
                            }
                            let mut description = format!(
                                "{}/{} · ctx {}{}",
                                model.provider,
                                model.id,
                                model.context_window,
                                if model.reasoning { " · reasoning" } else { "" }
                            );
                            if is_current {
                                description.push_str(" · current");
                            }
                            SelectionPanelOption {
                                label: model.name,
                                description,
                                action: SelectionPanelAction::SetModel {
                                    provider: model.provider,
                                    model_id: model.id,
                                },
                            }
                        })
                        .collect();
                    self.state.selection_panel =
                        Some(SelectionPanelState::models(options, selected));
                }
                Err(error) => {
                    self.state.selection_panel = None;
                    self.set_error(format!("Unable to list models: {error}"));
                }
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
                        search_active: false,
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
                                    ApprovalDecision::AllowOnce
                                    | ApprovalDecision::AllowSession
                                    | ApprovalDecision::AllowWorkspace => ToolStatus::Running,
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
}
