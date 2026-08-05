use super::*;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityGrantSet {
    #[serde(default)]
    pub matchers: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalTask {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub profile: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub grants: CapabilityGrantSet,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    pub status: String,
    #[serde(default)]
    pub result: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalSpec {
    pub revision: u64,
    pub summary: String,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub grants: CapabilityGrantSet,
    #[serde(default)]
    pub source_plan: Option<Value>,
    #[serde(default)]
    pub tasks: Vec<GoalSpecTask>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalSpecTask {
    pub id: String,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub grants: CapabilityGrantSet,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalRecord {
    #[serde(default)]
    pub schema_version: u8,
    pub id: String,
    #[serde(default)]
    pub workspace: String,
    pub session_id: String,
    pub objective: String,
    pub stage: String,
    #[serde(default)]
    pub previous_stage: Option<String>,
    pub revision: u64,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub source_plan: Option<Value>,
    #[serde(default)]
    pub spec: Option<GoalSpec>,
    #[serde(default)]
    pub tasks: Vec<GoalTask>,
    #[serde(default)]
    pub lease: Option<Value>,
    #[serde(default)]
    pub reviews: Vec<Value>,
    #[serde(default)]
    pub verification: Vec<Value>,
    #[serde(default)]
    pub repair_cycles: u64,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    pub goal: Option<GoalRecord>,
    pub state_path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalsSnapshot {
    pub goals: Vec<GoalRecord>,
    pub state_directory: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfileSnapshot {
    pub name: String,
    pub description: String,
    pub source: String,
    pub model: Option<String>,
    pub thinking_level: Option<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    pub tools: Vec<String>,
    pub permission: String,
    pub max_parallel: u64,
    pub max_turns: u64,
    #[serde(default)]
    pub isolation: AgentIsolationSnapshot,
    pub disabled: bool,
    #[serde(default)]
    pub unavailable_reason: Option<String>,
}
