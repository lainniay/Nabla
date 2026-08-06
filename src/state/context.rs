use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContextUsageState {
    Actual,
    Estimated,
    Recalculating,
}

impl ContextUsageState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Actual => "actual",
            Self::Estimated => "estimated",
            Self::Recalculating => "recalculating",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContextCategory {
    User,
    Assistant,
    ToolResult,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextCategoryEstimate {
    pub category: ContextCategory,
    pub message_count: u64,
    pub estimated_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextConsumer {
    pub category: ContextCategory,
    pub label: String,
    pub estimated_tokens: u64,
    #[serde(default)]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PruneReason {
    HardLimit,
    HistoryBudget,
    Superseded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextPruneEstimate {
    pub reason: PruneReason,
    pub count: u64,
    pub estimated_tokens_saved: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextPolicy {
    pub enabled: bool,
    pub recent_tool_result_tokens: u64,
    pub minimum_batch_savings_tokens: u64,
    pub minimum_tool_result_tokens: u64,
    pub success_tool_result_limit_tokens: u64,
    pub search_tool_result_limit_tokens: u64,
    pub error_tool_result_limit_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionRecord {
    pub reason: String,
    pub first_kept_entry_id: String,
    pub tokens_before: u64,
    pub estimated_tokens_after: Option<u64>,
    pub tokens_saved: Option<u64>,
    pub saved_percent: Option<f64>,
    pub file_count: u64,
    pub read_file_count: u64,
    pub modified_file_count: u64,
}

impl CompactionRecord {
    pub fn file_count(&self) -> u64 {
        self.file_count
    }

    pub fn deduplication_key(&self) -> String {
        format!(
            "{}\0{}\0{}",
            self.reason, self.first_kept_entry_id, self.tokens_before
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    #[serde(default)]
    pub revision: u64,
    pub usage_state: ContextUsageState,
    pub actual_tokens: Option<u64>,
    pub actual_percent: Option<f64>,
    pub context_window: Option<u64>,
    pub estimated_unfiltered_tokens: u64,
    pub estimated_next_request_tokens: u64,
    pub categories: Vec<ContextCategoryEstimate>,
    pub estimated_system_tool_other_tokens: Option<u64>,
    pub estimated_pruned_this_request_tokens: u64,
    pub estimated_currently_prunable_tokens: u64,
    pub estimated_cumulative_avoided_tokens: u64,
    pub pruning: Vec<ContextPruneEstimate>,
    pub top_consumers: Vec<ContextConsumer>,
    pub compaction_count: u64,
    pub recent_compactions: Vec<CompactionRecord>,
    pub policy: ContextPolicy,
    pub epoch: u64,
}

impl Default for ContextSnapshot {
    fn default() -> Self {
        Self {
            scope_id: None,
            revision: 0,
            usage_state: ContextUsageState::Estimated,
            actual_tokens: None,
            actual_percent: None,
            context_window: None,
            estimated_unfiltered_tokens: 0,
            estimated_next_request_tokens: 0,
            categories: Vec::new(),
            estimated_system_tool_other_tokens: None,
            estimated_pruned_this_request_tokens: 0,
            estimated_currently_prunable_tokens: 0,
            estimated_cumulative_avoided_tokens: 0,
            pruning: Vec::new(),
            top_consumers: Vec::new(),
            compaction_count: 0,
            recent_compactions: Vec::new(),
            policy: ContextPolicy {
                enabled: true,
                recent_tool_result_tokens: 40_000,
                minimum_batch_savings_tokens: 20_000,
                minimum_tool_result_tokens: 50,
                success_tool_result_limit_tokens: 12_000,
                search_tool_result_limit_tokens: 6_000,
                error_tool_result_limit_tokens: 8_000,
            },
            epoch: 0,
        }
    }
}

impl ContextSnapshot {
    pub fn remaining_percent(&self) -> Option<f64> {
        let used_percent = self.actual_percent.or_else(|| {
            let used_tokens = self
                .actual_tokens
                .or(Some(self.estimated_next_request_tokens))?;
            let window = self.context_window?;
            if window == 0 {
                return None;
            }
            Some((used_tokens as f64 / window as f64) * 100.0)
        })?;
        Some((100.0 - used_percent).max(0.0))
    }
}
