use std::{path::Path, sync::Arc, time::Duration};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::{
    net::{UnixStream, unix::OwnedWriteHalf},
    sync::{Mutex, mpsc},
    task::JoinHandle,
    time::sleep,
};

use crate::rpc::PiState;
use crate::rpc::{JsonLineRpcPeer, RPC_EVENT_BUFFER, RpcError, RpcEvent, RpcResponse};
use crate::state::{
    ActiveAgentSnapshot, AgentsSnapshot, ApprovalRulesSnapshot, ContextSnapshot, PlanArtifact,
    PlanExecutionContext, QuestionAnswer, ResourceSnapshot, SessionBrowserSnapshot,
    SessionHistoryItem, SessionScope, SessionSortMode, TreeFilterMode, TreeSnapshot,
    WorktreeIntegrationSnapshot,
};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentStartData {
    pub accepted: bool,
    pub agent: ActiveAgentSnapshot,
}

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECT_RETRY_DELAY: Duration = Duration::from_millis(25);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const SESSION_TIMEOUT: Duration = Duration::from_secs(60);
const TREE_NAVIGATION_TIMEOUT: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthProvider {
    pub id: String,
    pub name: String,
    pub configured: bool,
    pub configured_type: Option<String>,
    pub configured_source: Option<String>,
    pub methods: Vec<AuthMethod>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AuthMethod {
    #[serde(rename = "type")]
    pub kind: String,
    pub label: String,
    pub available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AuthProvidersData {
    pub providers: Vec<AuthProvider>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthLoginData {
    pub provider_id: String,
    pub credential_type: String,
    #[serde(default)]
    pub selected_model: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostPlanModeData {
    pub active: bool,
    pub active_tools: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct PlanStateData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    #[serde(default)]
    pub artifact: Option<PlanArtifact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingIntegrationData {
    pub agent: ActiveAgentSnapshot,
    pub integration: WorktreeIntegrationSnapshot,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapStateData {
    pub scope_id: String,
    pub plan_mode: HostPlanModeData,
    pub plan: PlanStateData,
    pub resources: ResourceSnapshot,
    pub agents: AgentsSnapshot,
    pub context: ContextSnapshot,
    #[serde(default)]
    pub pending_integrations: Vec<PendingIntegrationData>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanExecutionData {
    pub session_id: String,
    pub context: PlanExecutionContext,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionActivationData {
    pub state: PiState,
    pub cwd: String,
    pub plan_mode: bool,
    pub history: Vec<SessionHistoryItem>,
    pub plan: Option<PlanArtifact>,
    pub context: ContextSnapshot,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionCommandData {
    pub cancelled: bool,
    #[serde(default)]
    pub activation: Option<SessionActivationData>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeNavigateData {
    pub cancelled: bool,
    pub aborted: bool,
    #[serde(default)]
    pub editor_text: Option<String>,
    #[serde(default)]
    pub activation: Option<SessionActivationData>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueClearData {
    pub steering: Vec<String>,
    pub follow_up: Vec<String>,
    pub restored_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSummary {
    pub provider: String,
    pub id: String,
    pub name: String,
    pub reasoning: bool,
    pub context_window: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelListData {
    pub current: Option<Value>,
    pub models: Vec<ModelSummary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    AllowOnce,
    AllowSession,
    AllowWorkspace,
    Deny,
}

pub struct HostRuntime {
    pub client: HostClient,
    pub events: HostEventReceiver,
    pub guard: HostConnectionGuard,
}

impl HostRuntime {
    pub async fn connect(socket_path: &Path, request_timeout: Duration) -> Result<Self, RpcError> {
        let stream = connect_with_retry(socket_path).await?;
        let (reader, writer) = stream.into_split();
        let writer = Arc::new(Mutex::new(Some(writer)));
        let peer = JsonLineRpcPeer::new(writer.clone(), "nabla-host-", request_timeout);
        let (event_tx, events) = mpsc::channel(RPC_EVENT_BUFFER);

        let read_peer = peer.clone();
        let read_task = tokio::spawn(async move {
            read_peer.read_from(reader, event_tx).await;
        });

        Ok(Self {
            client: HostClient {
                peer,
                request_timeout,
            },
            events: HostEventReceiver { receiver: events },
            guard: HostConnectionGuard {
                writer,
                read_task: Some(read_task),
            },
        })
    }
}

#[derive(Clone)]
pub struct HostClient {
    peer: JsonLineRpcPeer<OwnedWriteHalf>,
    request_timeout: Duration,
}

impl HostClient {
    pub async fn bootstrap_state(&self) -> Result<BootstrapStateData, RpcError> {
        self.request_data("bootstrap_state", Map::new(), self.request_timeout)
            .await
    }

    pub async fn get_context_state(&self) -> Result<ContextSnapshot, RpcError> {
        self.request_data("context_state", Map::new(), self.request_timeout)
            .await
    }

    pub async fn get_plan_state(&self) -> Result<PlanStateData, RpcError> {
        self.request_data("get_plan_state", Map::new(), self.request_timeout)
            .await
    }

    pub async fn get_resources(&self) -> Result<ResourceSnapshot, RpcError> {
        self.request_data("resource_state", Map::new(), self.request_timeout)
            .await
    }

    pub async fn reload_resources(&self) -> Result<ResourceSnapshot, RpcError> {
        self.request_data("resource_reload", Map::new(), SESSION_TIMEOUT)
            .await
    }

    pub async fn set_workspace_trust(&self, trusted: bool) -> Result<ResourceSnapshot, RpcError> {
        let mut parameters = Map::new();
        parameters.insert("trusted".to_owned(), Value::Bool(trusted));
        self.request_data("workspace_trust", parameters, SESSION_TIMEOUT)
            .await
    }

    pub async fn get_approval_rules(&self) -> Result<ApprovalRulesSnapshot, RpcError> {
        self.request_data("approval_rules", Map::new(), self.request_timeout)
            .await
    }

    pub async fn revoke_approval_rule(
        &self,
        rule_id: String,
    ) -> Result<ApprovalRulesSnapshot, RpcError> {
        let mut parameters = Map::new();
        parameters.insert("ruleId".to_owned(), Value::String(rule_id));
        self.request_data("approval_rule_revoke", parameters, self.request_timeout)
            .await
    }

    pub async fn clear_approval_rules(&self) -> Result<ApprovalRulesSnapshot, RpcError> {
        self.request_data("approval_rules_clear", Map::new(), self.request_timeout)
            .await
    }

    pub async fn clear_queue(&self) -> Result<QueueClearData, RpcError> {
        self.request_data("queue_clear", Map::new(), self.request_timeout)
            .await
    }

    pub async fn list_models(&self) -> Result<ModelListData, RpcError> {
        self.request_data("model_list", Map::new(), self.request_timeout)
            .await
    }

    pub async fn set_model(&self, provider: String, model_id: String) -> Result<Value, RpcError> {
        let mut parameters = Map::new();
        parameters.insert("provider".to_owned(), Value::String(provider));
        parameters.insert("modelId".to_owned(), Value::String(model_id));
        self.request_data("model_set", parameters, self.request_timeout)
            .await
    }

    pub async fn set_thinking(&self, level: String) -> Result<Value, RpcError> {
        let mut parameters = Map::new();
        parameters.insert("level".to_owned(), Value::String(level));
        self.request_data("thinking_set", parameters, self.request_timeout)
            .await
    }

    pub async fn get_agents(&self) -> Result<AgentsSnapshot, RpcError> {
        self.request_data("agents_state", Map::new(), self.request_timeout)
            .await
    }

    pub async fn reload_agents(&self) -> Result<AgentsSnapshot, RpcError> {
        self.request_data("agents_reload", Map::new(), self.request_timeout)
            .await
    }

    pub async fn start_subagent(
        &self,
        profile: String,
        task: String,
    ) -> Result<SubagentStartData, RpcError> {
        let mut parameters = Map::new();
        parameters.insert("profile".to_owned(), Value::String(profile));
        parameters.insert("task".to_owned(), Value::String(task));
        self.request_data("subagent_start", parameters, self.request_timeout)
            .await
    }

    pub async fn cancel_subagent(&self, agent_id: String) -> Result<(), RpcError> {
        let mut parameters = Map::new();
        parameters.insert("agentId".to_owned(), Value::String(agent_id));
        self.request("subagent_cancel", parameters, self.request_timeout)
            .await?
            .ensure_success()
    }

    pub async fn integrate_subagent(
        &self,
        agent_id: String,
        action: String,
    ) -> Result<Value, RpcError> {
        let mut parameters = Map::new();
        parameters.insert("agentId".to_owned(), Value::String(agent_id));
        parameters.insert("action".to_owned(), Value::String(action));
        self.request_data("subagent_integrate", parameters, self.request_timeout)
            .await
    }

    pub async fn open_session_browser(&self) -> Result<SessionBrowserSnapshot, RpcError> {
        self.request_data("session_browser_open", Map::new(), SESSION_TIMEOUT)
            .await
    }

    pub async fn query_session_browser(
        &self,
        browser_id: String,
        scope: SessionScope,
        query: String,
        sort_mode: SessionSortMode,
        named_only: bool,
        offset: usize,
    ) -> Result<SessionBrowserSnapshot, RpcError> {
        let mut parameters = Map::new();
        parameters.insert("browserId".to_owned(), Value::String(browser_id));
        parameters.insert(
            "scope".to_owned(),
            serde_json::to_value(scope).map_err(|error| RpcError::Json(error.to_string()))?,
        );
        parameters.insert("query".to_owned(), Value::String(query));
        parameters.insert(
            "sortMode".to_owned(),
            serde_json::to_value(sort_mode).map_err(|error| RpcError::Json(error.to_string()))?,
        );
        parameters.insert("namedOnly".to_owned(), Value::Bool(named_only));
        parameters.insert("offset".to_owned(), Value::from(offset));
        self.request_data("session_browser_query", parameters, SESSION_TIMEOUT)
            .await
    }

    pub async fn close_session_browser(&self, browser_id: String) -> Result<(), RpcError> {
        let mut parameters = Map::new();
        parameters.insert("browserId".to_owned(), Value::String(browser_id));
        self.request("session_browser_close", parameters, self.request_timeout)
            .await?
            .ensure_success()
    }

    pub async fn new_session(&self) -> Result<SessionCommandData, RpcError> {
        self.request_data("session_new", Map::new(), SESSION_TIMEOUT)
            .await
    }

    pub async fn resume_session(
        &self,
        session_path: String,
        cwd_override: Option<String>,
    ) -> Result<SessionCommandData, RpcError> {
        let mut parameters = Map::new();
        parameters.insert("sessionPath".to_owned(), Value::String(session_path));
        if let Some(cwd_override) = cwd_override {
            parameters.insert("cwdOverride".to_owned(), Value::String(cwd_override));
        }
        self.request_data("session_resume", parameters, SESSION_TIMEOUT)
            .await
    }

    pub async fn get_tree_state(
        &self,
        filter_mode: TreeFilterMode,
        query: String,
        folded_entry_ids: Vec<String>,
    ) -> Result<TreeSnapshot, RpcError> {
        let mut parameters = Map::new();
        parameters.insert(
            "filterMode".to_owned(),
            serde_json::to_value(filter_mode).map_err(|error| RpcError::Json(error.to_string()))?,
        );
        parameters.insert("query".to_owned(), Value::String(query));
        parameters.insert(
            "foldedEntryIds".to_owned(),
            serde_json::to_value(folded_entry_ids)
                .map_err(|error| RpcError::Json(error.to_string()))?,
        );
        self.request_data("tree_state", parameters, self.request_timeout)
            .await
    }

    pub async fn set_tree_label(
        &self,
        entry_id: String,
        label: Option<String>,
    ) -> Result<(), RpcError> {
        let mut parameters = Map::new();
        parameters.insert("entryId".to_owned(), Value::String(entry_id));
        parameters.insert("label".to_owned(), label.map_or(Value::Null, Value::String));
        self.request("tree_label", parameters, self.request_timeout)
            .await?
            .ensure_success()
    }

    pub async fn copy_tree_entry(&self, entry_id: String) -> Result<(), RpcError> {
        let mut parameters = Map::new();
        parameters.insert("entryId".to_owned(), Value::String(entry_id));
        self.request("tree_copy", parameters, self.request_timeout)
            .await?
            .ensure_success()
    }

    pub async fn navigate_tree(
        &self,
        entry_id: String,
        summarize: bool,
        custom_instructions: Option<String>,
    ) -> Result<TreeNavigateData, RpcError> {
        let mut parameters = Map::new();
        parameters.insert("entryId".to_owned(), Value::String(entry_id));
        parameters.insert("summarize".to_owned(), Value::Bool(summarize));
        if let Some(instructions) = custom_instructions {
            parameters.insert("customInstructions".to_owned(), Value::String(instructions));
        }
        self.request_data("tree_navigate", parameters, TREE_NAVIGATION_TIMEOUT)
            .await
    }

    pub async fn abort_tree_navigation(&self) -> Result<(), RpcError> {
        self.request("tree_abort", Map::new(), self.request_timeout)
            .await?
            .ensure_success()
    }

    pub async fn set_plan_mode(&self, active: bool) -> Result<HostPlanModeData, RpcError> {
        let mut parameters = Map::new();
        parameters.insert("active".to_owned(), Value::Bool(active));
        self.request_data("set_plan_mode", parameters, self.request_timeout)
            .await
    }

    pub async fn reply_approval(
        &self,
        request_id: String,
        decision: ApprovalDecision,
    ) -> Result<(), RpcError> {
        let mut parameters = Map::new();
        parameters.insert("requestId".to_owned(), Value::String(request_id));
        parameters.insert(
            "decision".to_owned(),
            serde_json::to_value(decision).map_err(|error| RpcError::Json(error.to_string()))?,
        );
        self.request("approval_reply", parameters, self.request_timeout)
            .await?
            .ensure_success()
    }

    pub async fn reply_questions(
        &self,
        request_id: String,
        answers: Vec<QuestionAnswer>,
    ) -> Result<(), RpcError> {
        let mut parameters = Map::new();
        parameters.insert("requestId".to_owned(), Value::String(request_id));
        parameters.insert(
            "answers".to_owned(),
            serde_json::to_value(answers).map_err(|error| RpcError::Json(error.to_string()))?,
        );
        self.request("question_reply", parameters, self.request_timeout)
            .await?
            .ensure_success()
    }

    pub async fn execute_plan(
        &self,
        context: PlanExecutionContext,
    ) -> Result<PlanExecutionData, RpcError> {
        let mut parameters = Map::new();
        parameters.insert(
            "context".to_owned(),
            serde_json::to_value(context).map_err(|error| RpcError::Json(error.to_string()))?,
        );
        self.request_data("plan_execute", parameters, self.request_timeout)
            .await
    }

    pub async fn list_providers(&self) -> Result<Vec<AuthProvider>, RpcError> {
        let data: AuthProvidersData = self
            .request_data("auth_list", Map::new(), self.request_timeout)
            .await?;
        Ok(data.providers)
    }

    pub async fn login(
        &self,
        flow_id: String,
        provider_id: String,
        auth_type: String,
    ) -> Result<AuthLoginData, RpcError> {
        let mut parameters = Map::new();
        parameters.insert("flowId".to_owned(), Value::String(flow_id));
        parameters.insert("providerId".to_owned(), Value::String(provider_id));
        parameters.insert("authType".to_owned(), Value::String(auth_type));
        self.request_data("auth_login", parameters, LOGIN_TIMEOUT)
            .await
    }

    pub async fn reply(
        &self,
        flow_id: String,
        prompt_id: String,
        value: String,
    ) -> Result<(), RpcError> {
        let mut parameters = Map::new();
        parameters.insert("flowId".to_owned(), Value::String(flow_id));
        parameters.insert("promptId".to_owned(), Value::String(prompt_id));
        parameters.insert("value".to_owned(), Value::String(value));
        self.request("auth_reply", parameters, self.request_timeout)
            .await?
            .ensure_success()
    }

    pub async fn cancel(&self, flow_id: String) -> Result<(), RpcError> {
        let mut parameters = Map::new();
        parameters.insert("flowId".to_owned(), Value::String(flow_id));
        self.request("auth_cancel", parameters, self.request_timeout)
            .await?
            .ensure_success()
    }

    async fn request_data<T: for<'de> Deserialize<'de>>(
        &self,
        command: &str,
        parameters: Map<String, Value>,
        request_timeout: Duration,
    ) -> Result<T, RpcError> {
        self.peer
            .request_data_with_timeout(command, parameters, request_timeout)
            .await
    }

    async fn request(
        &self,
        command: &str,
        parameters: Map<String, Value>,
        request_timeout: Duration,
    ) -> Result<RpcResponse, RpcError> {
        self.peer
            .request_with_timeout(command, parameters, request_timeout)
            .await
    }
}

pub struct HostEventReceiver {
    receiver: mpsc::Receiver<Result<RpcEvent, RpcError>>,
}

impl HostEventReceiver {
    pub async fn recv(&mut self) -> Option<Result<RpcEvent, RpcError>> {
        self.receiver.recv().await
    }
}

pub struct HostConnectionGuard {
    writer: Arc<Mutex<Option<OwnedWriteHalf>>>,
    read_task: Option<JoinHandle<()>>,
}

impl HostConnectionGuard {
    pub async fn shutdown(&mut self) {
        self.writer.lock().await.take();
        if let Some(task) = self.read_task.take() {
            task.abort();
            let _ = task.await;
        }
    }
}

impl Drop for HostConnectionGuard {
    fn drop(&mut self) {
        if let Some(task) = self.read_task.take() {
            task.abort();
        }
    }
}

async fn connect_with_retry(socket_path: &Path) -> Result<UnixStream, RpcError> {
    let started = tokio::time::Instant::now();
    loop {
        match UnixStream::connect(socket_path).await {
            Ok(stream) => return Ok(stream),
            Err(error) if started.elapsed() < CONNECT_TIMEOUT => {
                let _ = error;
                sleep(CONNECT_RETRY_DELAY).await;
            }
            Err(error) => {
                return Err(RpcError::Io(format!(
                    "failed to connect to host control socket {}: {error}",
                    socket_path.display()
                )));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_provider_catalog_without_secret_material() {
        let data: AuthProvidersData = serde_json::from_value(serde_json::json!({
            "providers": [{
                "id": "openai-codex",
                "name": "OpenAI Codex",
                "configured": false,
                "methods": [{
                    "type": "oauth",
                    "label": "Sign in with ChatGPT",
                    "available": true
                }]
            }]
        }))
        .unwrap();

        assert_eq!(data.providers[0].id, "openai-codex");
        assert_eq!(data.providers[0].methods[0].kind, "oauth");
    }

    #[test]
    fn parses_plan_mode_response_and_serializes_approval_decision() {
        let data: HostPlanModeData = serde_json::from_value(serde_json::json!({
            "active": true,
            "activeTools": ["read", "edit", "bash"]
        }))
        .unwrap();

        assert!(data.active);
        assert_eq!(data.active_tools, ["read", "edit", "bash"]);
        assert_eq!(
            serde_json::to_value(ApprovalDecision::AllowOnce).unwrap(),
            serde_json::json!("allow_once")
        );
        assert_eq!(
            serde_json::to_value(ApprovalDecision::AllowSession).unwrap(),
            serde_json::json!("allow_session")
        );
        assert_eq!(
            serde_json::to_value(ApprovalDecision::AllowWorkspace).unwrap(),
            serde_json::json!("allow_workspace")
        );
        assert_eq!(
            serde_json::to_value(ApprovalDecision::Deny).unwrap(),
            serde_json::json!("deny")
        );
        assert!(
            serde_json::from_value::<ApprovalDecision>(serde_json::json!("allow_forever")).is_err()
        );
    }

    #[test]
    fn parses_plan_state_and_execution_responses() {
        let artifact = serde_json::json!({
            "id": "plan-1",
            "revision": 2,
            "title": "Plan",
            "summary": "Summary",
            "bodyMarkdown": "Implementation",
            "assumptions": [],
            "testPlan": ["cargo test"],
            "handoffMarkdown": "Handoff",
            "sourceSessionId": "session-1",
            "createdAt": "2026-01-01T00:00:00.000Z",
            "updatedAt": "2026-01-01T00:00:01.000Z"
        });
        let state: PlanStateData =
            serde_json::from_value(serde_json::json!({"artifact": artifact.clone()})).unwrap();
        let execution: PlanExecutionData = serde_json::from_value(serde_json::json!({
            "sessionId": "session-2",
            "context": "fresh"
        }))
        .unwrap();
        let current: PlanExecutionData = serde_json::from_value(serde_json::json!({
            "sessionId": "session-1",
            "context": "current"
        }))
        .unwrap();

        assert_eq!(state.artifact.unwrap().revision, 2);
        assert_eq!(execution.context, PlanExecutionContext::Fresh);
        assert_eq!(execution.session_id, "session-2");
        assert_eq!(current.context, PlanExecutionContext::Current);
        assert_eq!(
            serde_json::to_value(PlanExecutionContext::Current).unwrap(),
            serde_json::json!("current")
        );
        assert_eq!(
            serde_json::to_value(PlanExecutionContext::Fresh).unwrap(),
            serde_json::json!("fresh")
        );
    }

    #[test]
    fn parses_atomic_bootstrap_state_with_pending_integrations() {
        let data: BootstrapStateData = serde_json::from_value(serde_json::json!({
            "scopeId": "session-1",
            "planMode": {"active": false, "activeTools": ["read"]},
            "plan": {"artifact": null},
            "resources": {
                "trusted": false,
                "contextFiles": [],
                "skills": [],
                "prompts": [],
                "extensions": [],
                "commands": [],
                "diagnostics": [],
                "revision": 1
            },
            "agents": {
                "maxParallel": 3,
                "profiles": [],
                "active": [],
                "pending": [],
                "diagnostics": []
            },
            "context": serde_json::to_value(ContextSnapshot::default()).unwrap(),
            "pendingIntegrations": [{
                "agent": {
                    "id": "agent-1",
                    "profile": "worker",
                    "task": "Implement",
                    "lifecycle": "awaiting_integration",
                    "startedAt": "now",
                    "turns": 1,
                    "maxTurns": 4,
                    "model": "test/model",
                    "originSessionId": "session-1"
                },
                "integration": {
                    "backend": "worktree",
                    "status": "pending",
                    "changedPaths": ["src/lib.rs"],
                    "patchBytes": 12
                }
            }],
            "warnings": ["recovered"]
        }))
        .unwrap();

        assert_eq!(data.scope_id, "session-1");
        assert_eq!(data.pending_integrations.len(), 1);
        assert_eq!(data.pending_integrations[0].agent.id, "agent-1");
    }

    #[test]
    fn shared_bootstrap_fixture_round_trips_without_dropping_host_fields() {
        let fixture: Value =
            serde_json::from_str(include_str!("../protocol-fixtures/bootstrap-state.json"))
                .unwrap();
        let state: BootstrapStateData = serde_json::from_value(fixture.clone()).unwrap();
        let round_trip = serde_json::to_value(state).unwrap();

        assert!(fixture.get("goal").is_none());
        assert!(round_trip.get("goal").is_none());
        assert_eq!(round_trip, fixture);
    }

    #[test]
    fn shared_turn_boundary_fixture_accepts_future_fields() {
        let history: Vec<SessionHistoryItem> = serde_json::from_str(include_str!(
            "../protocol-fixtures/session-history-turn-boundary.json"
        ))
        .expect("turn boundary fixture");
        assert_eq!(
            history,
            vec![
                SessionHistoryItem::TurnBoundary {
                    turn_id: "turn-exact".to_owned(),
                    started_at: "2026-08-04T01:02:03.000Z".to_owned(),
                    ended_at: "2026-08-04T01:03:08.000Z".to_owned(),
                    duration_ms: 65_000,
                    estimated: false,
                },
                SessionHistoryItem::TurnBoundary {
                    turn_id: "legacy-entry-1".to_owned(),
                    started_at: "2026-08-04T02:00:00.000Z".to_owned(),
                    ended_at: "2026-08-04T02:00:12.000Z".to_owned(),
                    duration_ms: 12_000,
                    estimated: true,
                },
            ]
        );
    }

    #[test]
    fn shared_persistent_approval_fixture_matches_rust_contract() {
        let snapshot: ApprovalRulesSnapshot = serde_json::from_str(include_str!(
            "../protocol-fixtures/nabla.workspace-grants.v3.json"
        ))
        .unwrap();
        assert_eq!(snapshot.workspace, "/workspace");
        assert_eq!(snapshot.grants[0].proposal.scope, "workspace");
        assert_eq!(snapshot.grants[0].proposal.matchers[0]["kind"], "exec");
    }

    #[test]
    fn parses_session_browser_activation_and_tree_payloads() {
        let browser: SessionBrowserSnapshot = serde_json::from_value(serde_json::json!({
            "browserId": "browser-1",
            "currentCwd": "/workspace/current",
            "scope": "all",
            "query": "parser",
            "sortMode": "relevance",
            "namedOnly": true,
            "sessions": [{
                "path": "/sessions/old.jsonl",
                "id": "session-old",
                "cwd": "/workspace/old",
                "cwdAvailable": false,
                "name": "Old work",
                "parentSessionPath": "/sessions/parent.jsonl",
                "createdAt": "2026-01-01T00:00:00.000Z",
                "modifiedAt": "2026-01-02T00:00:00.000Z",
                "messageCount": 12,
                "firstMessage": "fix parser",
                "depth": 1,
                "isLast": true,
                "current": false
            }],
            "total": 1
        }))
        .unwrap();
        assert_eq!(browser.scope, SessionScope::All);
        assert_eq!(browser.sort_mode, SessionSortMode::Relevance);
        assert!(!browser.sessions[0].cwd_available);

        let command: SessionCommandData = serde_json::from_value(serde_json::json!({
            "cancelled": false,
            "activation": {
                "state": {
                    "model": {"provider": "test", "name": "fake"},
                    "thinkingLevel": "off",
                    "isStreaming": false,
                    "isCompacting": false,
                    "steeringMode": "one-at-a-time",
                    "followUpMode": "one-at-a-time",
                    "sessionFile": "/sessions/old.jsonl",
                    "sessionId": "session-old",
                    "sessionName": "Old work",
                    "autoCompactionEnabled": true,
                    "messageCount": 2,
                    "pendingMessageCount": 0
                },
                "cwd": "/workspace/old",
                "planMode": false,
                "history": [
                    {"kind": "user", "text": "fix parser"},
                    {
                        "kind": "toolResult",
                        "id": "tool-1",
                        "name": "read",
                        "output": "source",
                        "isError": false
                    }
                ],
                "plan": null,
                "context": serde_json::to_value(ContextSnapshot::default()).unwrap()
            }
        }))
        .unwrap();
        let activation = command.activation.expect("activation");
        assert_eq!(activation.state.session_id, "session-old");
        assert!(matches!(
            &activation.history[1],
            SessionHistoryItem::ToolResult {
                id,
                is_error: false,
                ..
            } if id == "tool-1"
        ));

        let tree: TreeSnapshot = serde_json::from_value(serde_json::json!({
            "items": [{
                "entryId": "entry-1",
                "parentId": null,
                "kind": "message",
                "role": "user",
                "preview": "user: fix parser",
                "visualDepth": 0,
                "showConnector": false,
                "gutterPositions": [],
                "isLast": true,
                "isActivePath": true,
                "isLeaf": true,
                "foldable": false,
                "folded": false
            }],
            "leafId": "entry-1",
            "filterMode": "no-tools",
            "query": ""
        }))
        .unwrap();
        assert_eq!(tree.filter_mode, TreeFilterMode::NoTools);
        assert_eq!(tree.leaf_id.as_deref(), Some("entry-1"));
    }
}
