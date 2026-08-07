use std::time::Duration;

use serde::de::Deserialize;
use serde_json::{Map, Value};
use tokio::net::unix::OwnedWriteHalf;

use super::dto::{
    ApprovalDecision, AuthLoginData, AuthProvider, AuthProvidersData, BootstrapStateData,
    HostPlanModeData, ModelListData, PlanExecutionData, PlanStateData, QueueClearData,
    SessionCommandData, SubagentStartData, TreeNavigateData,
};
use super::timeout::{LOGIN_TIMEOUT, SESSION_TIMEOUT, TREE_NAVIGATION_TIMEOUT};
use crate::rpc::{JsonLineRpcPeer, RpcError, RpcResponse};
use crate::state::{
    AgentsSnapshot, ApprovalRulesSnapshot, ContextSnapshot, PlanExecutionContext, QuestionAnswer,
    ResourceSnapshot, SessionBrowserSnapshot, SessionScope, SessionSortMode, TreeFilterMode,
    TreeSnapshot,
};

#[derive(Clone)]
pub struct HostClient {
    peer: JsonLineRpcPeer<OwnedWriteHalf>,
    request_timeout: Duration,
}

impl HostClient {
    pub(crate) fn new(peer: JsonLineRpcPeer<OwnedWriteHalf>, request_timeout: Duration) -> Self {
        Self {
            peer,
            request_timeout,
        }
    }

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
