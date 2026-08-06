use super::*;

// INFO: Host events are scope-checked before reduction so an old session cannot
// overwrite the active plan, resources, or approval workflow.
impl App {
    pub(super) fn update_host(&mut self, event: RpcEvent) -> Vec<AppEffect> {
        let mut effects = Vec::new();
        if event
            .payload
            .get("scopeId")
            .and_then(Value::as_str)
            .is_some_and(|scope_id| scope_id != self.state.session.session_id)
        {
            return effects;
        }
        match event.kind.as_str() {
            "turn_timing" => {
                if event.payload["phase"].as_str() != Some("completed") {
                    return effects;
                }
                if let Ok(separator) =
                    serde_json::from_value::<TurnSeparator>(event.payload.clone())
                {
                    if let Some(current) =
                        self.state
                            .transcript
                            .iter_mut()
                            .find_map(|item| match item {
                                TranscriptItem::TurnSeparator(current)
                                    if current.turn_id == separator.turn_id =>
                                {
                                    Some(current)
                                }
                                _ => None,
                            })
                    {
                        *current = separator;
                    } else {
                        self.state
                            .transcript
                            .push(TranscriptItem::TurnSeparator(separator));
                    }
                }
            }
            "approval_request" => {
                let Some(approval_id) = string_field(&event.payload, "requestId") else {
                    return effects;
                };
                let Some(tool_call_id) = string_field(&event.payload, "toolCallId") else {
                    return effects;
                };
                let Some(session_id) = string_field(&event.payload, "sessionId") else {
                    self.set_error("Host sent approval without a sessionId".to_owned());
                    return effects;
                };
                let Some(workspace_id) = string_field(&event.payload, "workspaceId") else {
                    self.set_error("Host sent approval without a workspaceId".to_owned());
                    return effects;
                };
                let Some(summary) = string_field(&event.payload, "summary") else {
                    self.set_error("Host sent approval without a summary".to_owned());
                    return effects;
                };
                let Some(intent_digest) = string_field(&event.payload, "intentDigest") else {
                    self.set_error("Host sent approval without an intentDigest".to_owned());
                    return effects;
                };
                let available_decisions = match serde_json::from_value::<Vec<ApprovalDecision>>(
                    event.payload["availableDecisions"].clone(),
                ) {
                    Ok(decisions)
                        if !decisions.is_empty()
                            && decisions.contains(&ApprovalDecision::Deny)
                            && decisions.contains(&ApprovalDecision::AllowOnce) =>
                    {
                        decisions
                    }
                    _ => {
                        self.set_error(
                            "Host sent invalid or unsupported approval decisions".to_owned(),
                        );
                        return effects;
                    }
                };
                let session_grant = match event.payload.get("sessionGrant") {
                    Some(value) => match serde_json::from_value(value.clone()) {
                        Ok(proposal) => Some(proposal),
                        Err(_) => {
                            self.set_error("Host sent an invalid session grant".to_owned());
                            return effects;
                        }
                    },
                    None => None,
                };
                let workspace_grant = match event.payload.get("workspaceGrant") {
                    Some(value) => match serde_json::from_value(value.clone()) {
                        Ok(proposal) => Some(proposal),
                        Err(_) => {
                            self.set_error("Host sent an invalid workspace grant".to_owned());
                            return effects;
                        }
                    },
                    None => None,
                };
                if available_decisions.contains(&ApprovalDecision::AllowSession)
                    != session_grant.is_some()
                    || available_decisions.contains(&ApprovalDecision::AllowWorkspace)
                        != workspace_grant.is_some()
                {
                    self.set_error(
                        "Host approval decisions do not match grant proposals".to_owned(),
                    );
                    return effects;
                }
                let tool_name = string_field(&event.payload, "toolName")
                    .unwrap_or_else(|| "unknown".to_owned());
                let input = event.payload["input"].clone();
                if let Some(tool) = self.find_tool_mut(Some(&tool_call_id)) {
                    tool.status = ToolStatus::WaitingApproval;
                    tool.args = input.clone();
                }
                self.state.approval = Some(ApprovalState {
                    approval_id,
                    tool_call_id,
                    session_id,
                    workspace_id,
                    tool_name,
                    input,
                    agent_id: string_field(&event.payload, "agentId"),
                    agent_profile: string_field(&event.payload, "agentProfile"),
                    model: string_field(&event.payload, "model"),
                    reason: string_field(&event.payload, "reason"),
                    risk: string_field(&event.payload, "risk"),
                    summary,
                    intent_digest,
                    available_decisions,
                    session_grant,
                    workspace_grant,
                    selected: 0,
                    replying: false,
                });
            }
            "question_request" => {
                let Some(request_id) = string_field(&event.payload, "requestId") else {
                    return effects;
                };
                let Ok(questions) =
                    serde_json::from_value::<Vec<PlanQuestion>>(event.payload["questions"].clone())
                else {
                    self.set_error("Host sent invalid clarification questions".to_owned());
                    return effects;
                };
                if questions.is_empty() {
                    self.set_error("Host sent an empty clarification request".to_owned());
                    return effects;
                }
                self.state.question = Some(QuestionFlowState {
                    request_id,
                    questions,
                    current: 0,
                    selected: 0,
                    custom_answer: false,
                    editor: EditorState::default(),
                    answers: Vec::new(),
                    replying: false,
                });
            }
            "question_cancelled" => {
                let request_id = string_field(&event.payload, "requestId");
                if self
                    .state
                    .question
                    .as_ref()
                    .is_some_and(|question| request_id.as_deref() == Some(&question.request_id))
                {
                    self.state.question = None;
                }
            }
            "plan_ready" => {
                if let Ok(artifact) =
                    serde_json::from_value::<PlanArtifact>(event.payload["artifact"].clone())
                {
                    self.receive_plan(artifact, true);
                }
            }
            "plan_mode_state" => {
                if let Some(active) = event.payload["active"].as_bool() {
                    self.state.plan_mode_active = active;
                    self.state.pending_plan_mode = None;
                }
            }
            "plan_state" => {
                if event.payload["artifact"].is_null() {
                    self.state.plan = None;
                    self.state.plan_review = None;
                } else if let Ok(artifact) =
                    serde_json::from_value::<PlanArtifact>(event.payload["artifact"].clone())
                {
                    self.receive_plan(artifact, false);
                }
            }
            "session_list_progress" => {
                let browser_id = string_field(&event.payload, "browserId");
                let scope = string_field(&event.payload, "scope");
                if let Some(browser) = self.state.session_browser.as_mut()
                    && browser_id.as_deref() == browser.browser_id.as_deref()
                    && scope.as_deref()
                        == Some(match browser.scope {
                            SessionScope::Current => "current",
                            SessionScope::All => "all",
                        })
                {
                    let loaded = event.payload["loaded"].as_u64().unwrap_or(0);
                    let total = event.payload["total"].as_u64().unwrap_or(0);
                    browser.loaded = Some((loaded, total));
                    browser.loading = true;
                }
            }
            "context_budget" => {
                if let Ok(snapshot) =
                    serde_json::from_value::<ContextSnapshot>(event.payload["snapshot"].clone())
                    && self.snapshot_scope_matches(snapshot.scope_id.as_deref())
                    && snapshot.revision >= self.state.context.revision
                {
                    self.state.context = snapshot;
                }
                if let Some(warning) = string_field(&event.payload, "policyWarning") {
                    self.state.transcript.push(TranscriptItem::Notice(warning));
                }
            }
            "workspace_state" => {
                let resources =
                    serde_json::from_value::<ResourceSnapshot>(event.payload["resources"].clone());
                let agents =
                    serde_json::from_value::<AgentsSnapshot>(event.payload["agents"].clone());
                if let (Ok(resources), Ok(agents)) = (resources, agents)
                    && self.snapshot_scope_matches(resources.scope_id.as_deref())
                    && self.snapshot_scope_matches(agents.scope_id.as_deref())
                    && resources.revision >= self.state.resources.revision
                    && agents.revision >= self.state.agents.revision
                {
                    self.state.command_catalog =
                        crate::command::CommandCatalog::new(resources.commands.clone());
                    self.state.resources = resources;
                    self.state.agents = agents;
                }
            }
            "resource_state" => {
                if let Ok(snapshot) =
                    serde_json::from_value::<ResourceSnapshot>(event.payload["snapshot"].clone())
                    && self.snapshot_scope_matches(snapshot.scope_id.as_deref())
                    && snapshot.revision >= self.state.resources.revision
                {
                    self.state.command_catalog =
                        crate::command::CommandCatalog::new(snapshot.commands.clone());
                    self.state.resources = snapshot;
                }
            }
            "agents_state" => {
                if let Ok(snapshot) =
                    serde_json::from_value::<AgentsSnapshot>(event.payload["snapshot"].clone())
                    && self.snapshot_scope_matches(snapshot.scope_id.as_deref())
                    && snapshot.revision >= self.state.agents.revision
                {
                    self.state.agents = snapshot;
                }
            }
            "subagent_state" => {
                let lifecycle =
                    string_field(&event.payload, "event").unwrap_or_else(|| "updated".to_owned());
                if let Ok(agent) =
                    serde_json::from_value::<ActiveAgentSnapshot>(event.payload["agent"].clone())
                {
                    if matches!(
                        lifecycle.as_str(),
                        "queued"
                            | "preparing_isolation"
                            | "isolated"
                            | "shared"
                            | "shared_fallback"
                            | "started"
                            | "resolving"
                    ) {
                        if let Some(current) = self
                            .state
                            .agents
                            .active
                            .iter_mut()
                            .find(|current| current.id == agent.id)
                        {
                            *current = agent.clone();
                        } else {
                            self.state.agents.active.push(agent.clone());
                        }
                    } else {
                        self.state
                            .agents
                            .active
                            .retain(|current| current.id != agent.id);
                    }
                    self.state
                        .transcript
                        .push(TranscriptItem::Subagent(SubagentTranscript {
                            event: lifecycle,
                            agent,
                            result: event.payload.get("result").cloned(),
                            error: string_field(&event.payload, "error"),
                        }));
                }
            }
            "subagent_integration" => {
                let lifecycle =
                    string_field(&event.payload, "event").unwrap_or_else(|| "updated".to_owned());
                let agent =
                    serde_json::from_value::<ActiveAgentSnapshot>(event.payload["agent"].clone());
                let integration = serde_json::from_value::<WorktreeIntegrationSnapshot>(
                    event.payload["integration"].clone(),
                );
                if let (Ok(agent), Ok(integration)) = (agent, integration) {
                    if matches!(
                        lifecycle.as_str(),
                        "pending" | "conflicted" | "needs_reconciliation"
                    ) {
                        self.enqueue_integration_prompt(IntegrationPromptState {
                            agent: agent.clone(),
                            integration: integration.clone(),
                            selected: if lifecycle == "conflicted" {
                                if integration.resolver_available { 1 } else { 2 }
                            } else if lifecycle == "needs_reconciliation" {
                                2
                            } else {
                                0
                            },
                            submitting: false,
                        });
                    } else if matches!(lifecycle.as_str(), "resolving" | "applied" | "discarded") {
                        self.remove_integration_prompt(&agent.id);
                    }
                    let changed = integration.changed_paths.len();
                    let detail = string_field(&event.payload, "error")
                        .or(integration.warning.clone())
                        .unwrap_or_default();
                    self.state.transcript.push(TranscriptItem::Notice(format!(
                        "Subagent {} integration {} · {} files · {} bytes{}",
                        agent.id,
                        lifecycle,
                        changed,
                        integration.patch_bytes,
                        if detail.is_empty() {
                            String::new()
                        } else {
                            format!(" · {detail}")
                        }
                    )));
                }
            }
            "host_warning" => {
                let warning = string_field(&event.payload, "message")
                    .unwrap_or_else(|| "Host reported a recoverable warning".to_owned());
                self.state.transcript.push(TranscriptItem::Error(warning));
            }
            "approval_blocked" => {
                let tool_call_id = string_field(&event.payload, "toolCallId");
                let reason = string_field(&event.payload, "reason")
                    .unwrap_or_else(|| "Mutation was blocked by the host".to_owned());
                if let Some(tool) = self.find_tool_mut(tool_call_id.as_deref()) {
                    tool.status = ToolStatus::Denied;
                    tool.output = reason.clone();
                }
                self.state.transcript.push(TranscriptItem::Error(reason));
            }
            "auth_prompt" => {
                let Some(flow_id) = string_field(&event.payload, "flowId") else {
                    return effects;
                };
                let Some(prompt_id) = string_field(&event.payload, "promptId") else {
                    return effects;
                };
                let Some(prompt_type) = string_field(&event.payload, "promptType") else {
                    return effects;
                };
                let kind = match prompt_type.as_str() {
                    "text" => AuthPromptKind::Text,
                    "secret" => AuthPromptKind::Secret,
                    "select" => AuthPromptKind::Select,
                    "manual_code" => AuthPromptKind::ManualCode,
                    _ => return effects,
                };
                let options = event.payload["options"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|option| {
                        Some(AuthPromptOption {
                            id: string_field(option, "id")?,
                            label: string_field(option, "label")?,
                            description: string_field(option, "description"),
                        })
                    })
                    .collect();
                if let AuthState::Running(flow) = &mut self.state.auth_state
                    && flow.id == flow_id
                {
                    flow.status = "Input required".to_owned();
                    flow.prompt = Some(AuthPromptState {
                        id: prompt_id,
                        kind,
                        message: string_field(&event.payload, "message")
                            .unwrap_or_else(|| "Authentication input".to_owned()),
                        placeholder: string_field(&event.payload, "placeholder"),
                        options,
                        selected: 0,
                        editor: EditorState::default(),
                    });
                }
            }
            "auth_prompt_cancelled" => {
                let flow_id = string_field(&event.payload, "flowId");
                let prompt_id = string_field(&event.payload, "promptId");
                if let AuthState::Running(flow) = &mut self.state.auth_state
                    && flow_id.as_deref() == Some(flow.id.as_str())
                    && flow
                        .prompt
                        .as_ref()
                        .is_some_and(|prompt| prompt_id.as_deref() == Some(prompt.id.as_str()))
                {
                    flow.prompt = None;
                    flow.status = "Waiting for authentication…".to_owned();
                }
            }
            "auth_notify" => {
                let Some(flow_id) = string_field(&event.payload, "flowId") else {
                    return effects;
                };
                let notification = &event.payload["event"];
                let Some(kind) = notification["type"].as_str() else {
                    return effects;
                };
                if let AuthState::Running(flow) = &mut self.state.auth_state
                    && flow.id == flow_id
                {
                    let mut candidate_url = None;
                    match kind {
                        "auth_url" => {
                            candidate_url = string_field(notification, "url");
                            flow.status = string_field(notification, "instructions")
                                .unwrap_or_else(|| "Open the URL to continue login.".to_owned());
                        }
                        "device_code" => {
                            candidate_url = string_field(notification, "verificationUri");
                            flow.device_code = string_field(notification, "userCode");
                            flow.status = "Waiting for device authorization…".to_owned();
                        }
                        "progress" | "info" => {
                            flow.status = string_field(notification, "message")
                                .unwrap_or_else(|| "Authenticating…".to_owned());
                            if kind == "info"
                                && flow.url.is_none()
                                && let Some(url) = notification["links"]
                                    .as_array()
                                    .and_then(|links| links.first())
                                    .and_then(|link| link["url"].as_str())
                            {
                                candidate_url = Some(url.to_owned());
                            }
                        }
                        _ => {}
                    }
                    if let Some(url) = candidate_url.filter(|url| is_safe_web_url(url)) {
                        let should_open = flow.url.as_deref() != Some(url.as_str());
                        flow.url = Some(url.clone());
                        if should_open {
                            effects.push(AppEffect::OpenUrl(url));
                        }
                    }
                }
            }
            "auth_complete" => {
                if let AuthState::Running(flow) = &mut self.state.auth_state {
                    flow.prompt = None;
                    flow.status = "Authentication complete.".to_owned();
                }
            }
            "auth_protocol_error" | "host_protocol_error" => {
                let error = string_field(&event.payload, "error")
                    .unwrap_or_else(|| "Authentication protocol failed".to_owned());
                self.set_auth_error(error);
            }
            _ => {}
        }
        effects
    }
}
