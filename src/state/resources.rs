use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ApprovalState {
    pub approval_id: String,
    pub tool_call_id: String,
    pub session_id: String,
    pub workspace_id: String,
    pub tool_name: String,
    pub input: Value,
    pub agent_id: Option<String>,
    pub agent_profile: Option<String>,
    pub model: Option<String>,
    pub goal_id: Option<String>,
    pub reason: Option<String>,
    pub risk: Option<String>,
    pub summary: String,
    pub intent_digest: String,
    pub available_decisions: Vec<ApprovalDecision>,
    pub session_grant: Option<GrantProposal>,
    pub workspace_grant: Option<GrantProposal>,
    pub selected: usize,
    pub replying: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantProposal {
    pub scope: String,
    pub workspace_id: String,
    #[serde(default)]
    pub session_id: Option<String>,
    pub matchers: Vec<Value>,
    #[serde(default)]
    pub invalidation_keys: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceDescriptor {
    pub name: String,
    pub path: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceDiagnostic {
    #[serde(rename = "type")]
    pub kind: String,
    pub message: String,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ResourceSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    pub trusted: bool,
    pub context_files: Vec<String>,
    pub skills: Vec<ResourceDescriptor>,
    pub prompts: Vec<ResourceDescriptor>,
    pub extensions: Vec<String>,
    #[serde(default)]
    pub commands: Vec<DiscoveredCommand>,
    pub diagnostics: Vec<ResourceDiagnostic>,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceGrantRecord {
    pub id: String,
    #[serde(flatten)]
    pub proposal: GrantProposal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRulesSnapshot {
    pub workspace: String,
    pub grants: Vec<WorkspaceGrantRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionManagerState {
    pub snapshot: ApprovalRulesSnapshot,
    pub selected: usize,
}
