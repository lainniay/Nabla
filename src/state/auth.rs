use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthChoice {
    pub provider_id: String,
    pub provider_name: String,
    pub auth_type: String,
    pub label: String,
    pub configured: bool,
}

pub fn matching_auth_choice_indices(choices: &[AuthChoice], query: &str) -> Vec<usize> {
    let terms = query
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    choices
        .iter()
        .enumerate()
        .filter_map(|(index, choice)| {
            let searchable = format!(
                "{} {} {} {}",
                choice.provider_name, choice.provider_id, choice.label, choice.auth_type
            )
            .to_lowercase();
            terms
                .iter()
                .all(|term| searchable.contains(term))
                .then_some(index)
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthPromptKind {
    Text,
    Secret,
    Select,
    ManualCode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthPromptOption {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
}

#[derive(Clone)]
pub struct AuthPromptState {
    pub id: String,
    pub kind: AuthPromptKind,
    pub message: String,
    pub placeholder: Option<String>,
    pub options: Vec<AuthPromptOption>,
    pub selected: usize,
    pub editor: EditorState,
}

#[derive(Clone)]
pub struct AuthFlowState {
    pub id: String,
    pub provider_name: String,
    pub status: String,
    pub url: Option<String>,
    pub device_code: Option<String>,
    pub prompt: Option<AuthPromptState>,
}

#[derive(Clone)]
pub enum AuthState {
    Inactive,
    LoadingProviders,
    Selecting {
        choices: Vec<AuthChoice>,
        selected: usize,
        filter: EditorState,
        search_active: bool,
    },
    Running(Box<AuthFlowState>),
}

impl AuthState {
    pub fn is_active(&self) -> bool {
        !matches!(self, Self::Inactive)
    }
}
