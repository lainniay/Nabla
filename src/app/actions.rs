use super::*;

// INFO: User intents are translated into declarative effects here. I/O remains
// in adapters, keeping command routing and error transitions deterministic.
impl App {
    pub(super) fn submit(&mut self, message: String) -> Vec<AppEffect> {
        match self.state.command_catalog.route(&message) {
            CommandRoute::Local(command) => {
                self.state.editor.clear();
                self.state.file_completion = None;
                let transcript_len = self.state.transcript.len();
                let effects = self.run_local_command(message, command);
                if let Some(completion) = effects.iter().find_map(local_command_completion) {
                    self.begin_local_command_timing(completion);
                } else if effects.is_empty() && self.state.transcript.len() > transcript_len {
                    // Purely local informational commands still receive the
                    // same visual completion boundary as asynchronous ones.
                    self.record_immediate_local_command_timing();
                }
                return effects;
            }
            CommandRoute::Unknown { name, suggestions } => {
                self.state.editor.clear();
                self.state.file_completion = None;
                self.push_user(message, UserMessageStatus::Failed);
                let suggestion = if suggestions.is_empty() {
                    " Use /help to list available commands.".to_owned()
                } else {
                    format!(
                        " Did you mean {}?",
                        suggestions
                            .iter()
                            .map(|name| format!("/{name}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                self.state.transcript.push(TranscriptItem::Notice(format!(
                    "Unknown command /{name}.{suggestion}"
                )));
                return Vec::new();
            }
            CommandRoute::Prompt => {}
        }

        if self.state.run_state == RunState::AuthRequired {
            self.state.editor.clear();
            self.push_user(message, UserMessageStatus::Failed);
            self.state
                .transcript
                .push(TranscriptItem::Notice(self.login_instructions()));
            return Vec::new();
        }

        self.state.last_error = None;
        self.prepare_delivery(message, PromptDelivery::Prompt)
    }

    pub(super) fn run_local_command(
        &mut self,
        source: String,
        command: LocalCommand,
    ) -> Vec<AppEffect> {
        if !matches!(
            &command,
            LocalCommand::Plan(_)
                | LocalCommand::Compact(_)
                | LocalCommand::Context
                | LocalCommand::Resources
                | LocalCommand::Reload
                | LocalCommand::Trust(_)
                | LocalCommand::Permissions(_)
                | LocalCommand::Model(_)
                | LocalCommand::Thinking(_)
                | LocalCommand::Agents(_)
                | LocalCommand::Agent(_)
                | LocalCommand::New(_)
                | LocalCommand::Resume(_)
                | LocalCommand::Tree(_)
        ) {
            self.push_user(source, UserMessageStatus::Accepted);
        }
        match command {
            LocalCommand::Login => {
                self.state.auth_state = AuthState::LoadingProviders;
                self.state.run_state = RunState::Authenticating;
                vec![AppEffect::AuthList]
            }
            LocalCommand::New(argument) => {
                if argument.is_some() {
                    self.state
                        .transcript
                        .push(TranscriptItem::Error("Usage: /new".to_owned()));
                    return Vec::new();
                }
                self.state.run_state = RunState::SwitchingSession;
                self.state.last_error = None;
                vec![AppEffect::NewSession]
            }
            LocalCommand::Resume(argument) => {
                if argument.is_some() {
                    self.state
                        .transcript
                        .push(TranscriptItem::Error("Usage: /resume".to_owned()));
                    return Vec::new();
                }
                self.state.session_browser = Some(SessionBrowserState::loading());
                self.state.last_error = None;
                vec![AppEffect::OpenSessionBrowser]
            }
            LocalCommand::Tree(argument) => {
                if argument.is_some() {
                    self.state
                        .transcript
                        .push(TranscriptItem::Error("Usage: /tree".to_owned()));
                    return Vec::new();
                }
                self.state.tree_browser = Some(TreeBrowserState::loading());
                self.state.last_error = None;
                vec![AppEffect::GetTreeState {
                    filter_mode: TreeFilterMode::Default,
                    query: String::new(),
                    folded_entry_ids: Vec::new(),
                    generation: 0,
                }]
            }
            LocalCommand::Plan(argument) => match argument {
                None => {
                    if self.state.plan_mode_active {
                        if self.state.plan.is_some() {
                            self.state.plan_review = Some(PlanReviewState {
                                selected: 0,
                                submitting: false,
                            });
                        } else {
                            self.state.transcript.push(TranscriptItem::Notice(
                                "Plan mode is active; no Plan is awaiting review.".to_owned(),
                            ));
                        }
                        Vec::new()
                    } else {
                        self.toggle_plan_mode(true)
                    }
                }
                Some(prompt) => {
                    if self.state.plan_mode_active {
                        self.prepare_delivery(prompt, PromptDelivery::Prompt)
                    } else {
                        self.state.pending_plan_prompt = Some(prompt);
                        self.toggle_plan_mode(true)
                    }
                }
            },
            LocalCommand::Compact(instructions) => {
                self.state.run_state = RunState::Compacting;
                self.state.last_error = None;
                self.state.compact_lifecycle_finished = false;
                vec![AppEffect::Compact(instructions)]
            }
            LocalCommand::Context => vec![AppEffect::GetContextState],
            LocalCommand::Resources => vec![AppEffect::GetResources],
            LocalCommand::Reload => vec![AppEffect::ReloadResources],
            LocalCommand::Trust(argument) => {
                match argument.as_deref().map(str::to_ascii_lowercase).as_deref() {
                    None | Some("status") => vec![AppEffect::GetResources],
                    Some("on" | "true" | "yes") => vec![AppEffect::SetWorkspaceTrust(true)],
                    Some("off" | "false" | "no") => vec![AppEffect::SetWorkspaceTrust(false)],
                    _ => {
                        self.state.transcript.push(TranscriptItem::Error(
                            "Usage: /trust [on|off|status]".to_owned(),
                        ));
                        Vec::new()
                    }
                }
            }
            LocalCommand::Permissions(argument) => match argument.as_deref() {
                None | Some("list" | "status") => vec![AppEffect::GetApprovalRules],
                Some("clear") => vec![AppEffect::ClearApprovalRules],
                Some(argument) if argument.starts_with("revoke ") => {
                    let rule_id = argument["revoke ".len()..].trim();
                    if rule_id.is_empty() {
                        self.set_error("Usage: /permissions revoke <rule-id>".to_owned());
                        Vec::new()
                    } else {
                        vec![AppEffect::RevokeApprovalRule(rule_id.to_owned())]
                    }
                }
                Some(_) => {
                    self.set_error("Usage: /permissions [list|clear|revoke <rule-id>]".to_owned());
                    Vec::new()
                }
            },
            LocalCommand::Model(argument) => {
                let Some(reference) = argument else {
                    self.state.selection_panel = Some(SelectionPanelState::loading_models());
                    return vec![AppEffect::ListModels];
                };
                let Some((provider, model_id)) = reference.split_once('/') else {
                    self.state.transcript.push(TranscriptItem::Error(
                        "Usage: /model [provider/model]".to_owned(),
                    ));
                    return Vec::new();
                };
                if provider.is_empty() || model_id.is_empty() {
                    self.state.transcript.push(TranscriptItem::Error(
                        "Usage: /model [provider/model]".to_owned(),
                    ));
                    return Vec::new();
                }
                vec![AppEffect::SetModel {
                    provider: provider.to_owned(),
                    model_id: model_id.to_owned(),
                }]
            }
            LocalCommand::Thinking(argument) => {
                let Some(level) = argument else {
                    self.state.selection_panel = Some(SelectionPanelState::thinking(
                        &self.state.session.thinking_level,
                    ));
                    return Vec::new();
                };
                vec![AppEffect::SetThinking(level)]
            }
            LocalCommand::Agents(argument) => {
                let Some(argument) = argument else {
                    return vec![AppEffect::GetAgents];
                };
                if argument == "reload" {
                    return vec![AppEffect::ReloadAgents];
                }
                if let Some((action, agent_id)) = argument.split_once(' ')
                    && matches!(action, "apply" | "resolve" | "keep" | "discard")
                    && !agent_id.trim().is_empty()
                {
                    return vec![AppEffect::IntegrateSubagent {
                        agent_id: agent_id.trim().to_owned(),
                        action: action.to_owned(),
                    }];
                }
                let Some(agent_id) = argument.strip_prefix("cancel ").map(str::trim) else {
                    self.state.transcript.push(TranscriptItem::Error(
                        "Usage: /agents [reload|cancel|apply|resolve|keep|discard <agent-id>]"
                            .to_owned(),
                    ));
                    return Vec::new();
                };
                if agent_id.is_empty() {
                    self.state.transcript.push(TranscriptItem::Error(
                        "Usage: /agents [reload|cancel|apply|resolve|keep|discard <agent-id>]"
                            .to_owned(),
                    ));
                    return Vec::new();
                }
                vec![AppEffect::CancelSubagent(agent_id.to_owned())]
            }
            LocalCommand::Agent(argument) => {
                let Some(argument) = argument else {
                    if self.state.agents.profiles.is_empty() {
                        self.state.open_agent_picker_on_agents = true;
                        return vec![AppEffect::GetAgents];
                    }
                    self.state.agent_picker = Some(AgentPickerState::new(&self.state.agents));
                    return Vec::new();
                };
                let Some((profile, task)) = argument.split_once(char::is_whitespace) else {
                    self.state.transcript.push(TranscriptItem::Error(
                        "Usage: /agent <name> <task>".to_owned(),
                    ));
                    return Vec::new();
                };
                let task = task.trim();
                if profile.is_empty() || task.is_empty() {
                    self.state.transcript.push(TranscriptItem::Error(
                        "Usage: /agent <name> <task>".to_owned(),
                    ));
                    return Vec::new();
                }
                vec![AppEffect::StartSubagent {
                    profile: profile.to_owned(),
                    task: task.to_owned(),
                }]
            }
            LocalCommand::Help => {
                let help = self.state.command_catalog.help_text();
                self.state.transcript.push(TranscriptItem::Notice(help));
                Vec::new()
            }
        }
    }

    pub(super) fn push_user(&mut self, text: String, status: UserMessageStatus) {
        self.state
            .transcript
            .push(TranscriptItem::User(UserMessage { text, status }));
    }

    pub(super) fn receive_plan(&mut self, artifact: PlanArtifact, show_review: bool) {
        let stale = self.state.plan.as_ref().is_some_and(|current| {
            (current.id == artifact.id
                && (current.revision > artifact.revision
                    || (current.revision == artifact.revision
                        && current.updated_at > artifact.updated_at)))
                || (current.id != artifact.id && current.updated_at > artifact.updated_at)
        });
        if stale {
            return;
        }
        let already_rendered = self.state.transcript.iter().any(|item| {
            matches!(
                item,
                TranscriptItem::Plan(existing)
                    if existing.id == artifact.id && existing.revision == artifact.revision
            )
        });
        if !already_rendered {
            self.state
                .transcript
                .push(TranscriptItem::Plan(artifact.clone()));
        }
        self.state.plan = Some(artifact);
        if show_review {
            self.state.plan_review = Some(PlanReviewState {
                selected: 0,
                submitting: false,
            });
        }
    }

    pub(super) fn enqueue_integration_prompt(&mut self, prompt: IntegrationPromptState) {
        if let Some(current) = self.state.integration_prompt.as_mut()
            && current.agent.id == prompt.agent.id
        {
            *current = prompt;
            return;
        }
        if let Some(existing) = self
            .state
            .integration_prompt_queue
            .iter_mut()
            .find(|existing| existing.agent.id == prompt.agent.id)
        {
            *existing = prompt;
            return;
        }
        if self.state.integration_prompt.is_none() {
            self.state.integration_prompt = Some(prompt);
        } else {
            self.state.integration_prompt_queue.push_back(prompt);
        }
    }

    pub(super) fn finish_current_integration_prompt(&mut self) {
        self.state.integration_prompt = self.state.integration_prompt_queue.pop_front();
    }

    pub(super) fn remove_integration_prompt(&mut self, agent_id: &str) {
        if self
            .state
            .integration_prompt
            .as_ref()
            .is_some_and(|prompt| prompt.agent.id == agent_id)
        {
            self.finish_current_integration_prompt();
        } else {
            self.state
                .integration_prompt_queue
                .retain(|prompt| prompt.agent.id != agent_id);
        }
    }

    pub(super) fn snapshot_scope_matches(&self, scope_id: Option<&str>) -> bool {
        scope_id.is_none_or(|scope_id| scope_id == self.state.session.session_id)
    }

    pub(super) fn set_pi_error(&mut self, error: String) {
        if is_missing_credentials(&error) {
            self.state.run_state = RunState::AuthRequired;
            self.state.last_error = Some(error);
            self.state.transcript.push(TranscriptItem::Error(
                "No credentials are configured for the selected model.".to_owned(),
            ));
            self.state
                .transcript
                .push(TranscriptItem::Notice(self.login_instructions()));
        } else {
            self.set_error(error);
        }
    }

    pub(super) fn login_instructions(&self) -> String {
        "Use /login to authenticate inside Nabla.".to_owned()
    }

    pub(super) fn set_auth_error(&mut self, error: String) {
        if let AuthState::Running(flow) = &mut self.state.auth_state {
            flow.prompt = None;
            flow.status = error;
        } else {
            self.set_error(error);
        }
    }

    pub(super) fn run_state_after_auth(&self) -> RunState {
        if self
            .state
            .last_error
            .as_deref()
            .is_some_and(is_missing_credentials)
        {
            RunState::AuthRequired
        } else {
            RunState::Idle
        }
    }

    pub(super) fn set_error(&mut self, error: String) {
        self.state.run_state = RunState::Error;
        self.state.last_error = Some(error.clone());
        self.state.transcript.push(TranscriptItem::Error(error));
    }
}

fn local_command_completion(effect: &AppEffect) -> Option<LocalCommandCompletion> {
    match effect {
        AppEffect::Compact(_) => Some(LocalCommandCompletion::Compact),
        AppEffect::GetContextState => Some(LocalCommandCompletion::Context),
        AppEffect::GetResources => Some(LocalCommandCompletion::Resources),
        AppEffect::ReloadResources => Some(LocalCommandCompletion::ResourceReload),
        AppEffect::SetWorkspaceTrust(_) => Some(LocalCommandCompletion::WorkspaceTrust),
        AppEffect::GetApprovalRules
        | AppEffect::RevokeApprovalRule(_)
        | AppEffect::ClearApprovalRules => None,
        AppEffect::ListModels => None,
        AppEffect::SetModel { .. } => Some(LocalCommandCompletion::ModelSet),
        AppEffect::SetThinking(_) => Some(LocalCommandCompletion::ThinkingSet),
        AppEffect::GetAgents => Some(LocalCommandCompletion::Agents),
        AppEffect::ReloadAgents => Some(LocalCommandCompletion::AgentsReload),
        _ => None,
    }
}
