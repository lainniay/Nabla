use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentIsolationSnapshot {
    pub mode: String,
    pub integration: String,
}

impl Default for AgentIsolationSnapshot {
    fn default() -> Self {
        Self {
            mode: "none".to_owned(),
            integration: "source".to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveAgentSnapshot {
    pub id: String,
    pub profile: String,
    pub task: String,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub goal_id: Option<String>,
    pub lifecycle: String,
    pub started_at: String,
    pub turns: u64,
    pub max_turns: u64,
    pub model: String,
    pub origin_session_id: String,
    #[serde(default = "default_shared_backend")]
    pub isolation_backend: String,
    #[serde(default = "default_integration_status")]
    pub integration_status: String,
    #[serde(default)]
    pub isolation_warning: Option<String>,
}

fn default_shared_backend() -> String {
    "shared".to_owned()
}

fn default_integration_status() -> String {
    "none".to_owned()
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeIntegrationSnapshot {
    pub backend: String,
    pub status: String,
    #[serde(default)]
    pub warning: Option<String>,
    #[serde(default)]
    pub artifact_id: Option<String>,
    #[serde(default)]
    pub changed_paths: Vec<String>,
    #[serde(default)]
    pub patch_bytes: u64,
    #[serde(default)]
    pub excluded_paths: Vec<String>,
    #[serde(default = "default_true")]
    pub resolver_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationPromptState {
    pub agent: ActiveAgentSnapshot,
    pub integration: WorktreeIntegrationSnapshot,
    pub selected: usize,
    pub submitting: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentConfigDiagnostic {
    #[serde(rename = "type")]
    pub kind: String,
    pub message: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub profile: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentsSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    #[serde(default)]
    pub revision: u64,
    pub max_parallel: u64,
    pub profiles: Vec<AgentProfileSnapshot>,
    pub active: Vec<ActiveAgentSnapshot>,
    #[serde(default)]
    pub pending: Vec<ActiveAgentSnapshot>,
    #[serde(default)]
    pub diagnostics: Vec<AgentConfigDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentTranscript {
    pub event: String,
    pub agent: ActiveAgentSnapshot,
    pub result: Option<Value>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentPickerState {
    pub profiles: Vec<AgentProfileSnapshot>,
    pub selected: usize,
}

impl AgentPickerState {
    pub fn new(snapshot: &AgentsSnapshot) -> Self {
        let profiles = snapshot
            .profiles
            .iter()
            .filter(|profile| !profile.disabled && profile.unavailable_reason.is_none())
            .cloned()
            .collect();
        Self {
            profiles,
            selected: 0,
        }
    }

    pub fn selected_profile(&self) -> Option<&AgentProfileSnapshot> {
        self.profiles.get(self.selected)
    }
}
