use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::rpc::PiState;
use crate::state::{
    ActiveAgentSnapshot, AgentsSnapshot, ContextSnapshot, PlanArtifact, PlanExecutionContext,
    ResourceSnapshot, SessionHistoryItem, WorktreeIntegrationSnapshot,
};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentStartData {
    pub accepted: bool,
    pub agent: ActiveAgentSnapshot,
}

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
#[serde(rename_all = "camelCase")]
pub struct SandboxStatusData {
    pub mode: String,
    pub backend: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub filesystem: String,
    pub network: String,
}

impl Default for SandboxStatusData {
    fn default() -> Self {
        Self {
            mode: "disabled".to_owned(),
            backend: "none".to_owned(),
            reason: None,
            filesystem: "full-access".to_owned(),
            network: "allowed".to_owned(),
        }
    }
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
    #[serde(default)]
    pub sandbox: SandboxStatusData,
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
