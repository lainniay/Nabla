use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlanStatus {
    #[serde(alias = "ready")]
    Submitted,
    Executing,
    Completed,
}

impl PlanStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Submitted => "submitted",
            Self::Executing => "executing",
            Self::Completed => "completed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanArtifact {
    pub schema_version: u8,
    pub id: String,
    pub revision: u64,
    pub status: PlanStatus,
    pub title: String,
    pub summary: String,
    pub body_markdown: String,
    pub assumptions: Vec<String>,
    pub test_plan: Vec<String>,
    pub source_session_id: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub last_execution_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanExecutionTarget {
    Current,
    Fresh,
}

impl PlanExecutionTarget {
    pub fn label(self) -> &'static str {
        match self {
            Self::Current => "current context",
            Self::Fresh => "fresh context",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanReviewState {
    Menu {
        selected: usize,
    },
    Confirm {
        target: PlanExecutionTarget,
        selected: usize,
        submitting: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalApprovalState {
    pub selected: usize,
    pub submitting: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionOption {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanQuestion {
    pub id: String,
    pub prompt: String,
    pub options: Vec<QuestionOption>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionAnswer {
    pub question_id: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub option_id: Option<String>,
}

#[derive(Clone)]
pub struct QuestionFlowState {
    pub request_id: String,
    pub questions: Vec<PlanQuestion>,
    pub current: usize,
    pub selected: usize,
    pub custom_answer: bool,
    pub editor: EditorState,
    pub answers: Vec<QuestionAnswer>,
    pub replying: bool,
}

impl QuestionFlowState {
    pub fn current_question(&self) -> Option<&PlanQuestion> {
        self.questions.get(self.current)
    }

    pub fn choice_count(&self) -> usize {
        self.current_question()
            .map_or(0, |question| question.options.len() + 1)
    }
}
