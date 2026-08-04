use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalState {
    pub approval_id: String,
    pub tool_call_id: String,
    pub tool_name: String,
    pub input: Value,
    pub agent_id: Option<String>,
    pub agent_profile: Option<String>,
    pub model: Option<String>,
    pub goal_id: Option<String>,
    pub reason: Option<String>,
    pub risk: Option<String>,
    pub selected: usize,
    pub replying: bool,
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
#[serde(rename_all = "camelCase")]
pub struct PersistentApprovalRule {
    pub id: String,
    pub workspace: String,
    pub tool_name: String,
    pub kind: String,
    pub value: String,
    pub recursive: bool,
    pub summary: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRulesSnapshot {
    pub workspace: String,
    pub rules: Vec<PersistentApprovalRule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionManagerState {
    pub snapshot: ApprovalRulesSnapshot,
    pub selected: usize,
}
