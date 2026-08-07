pub mod client;
pub mod connection;
pub mod dto;
pub mod timeout;

#[cfg(test)]
mod tests;

pub use client::HostClient;
pub use connection::{HostConnectionGuard, HostEventReceiver, HostRuntime};
pub use dto::{
    ApprovalDecision, AuthLoginData, AuthMethod, AuthProvider, AuthProvidersData,
    BootstrapStateData, HostPlanModeData, ModelListData, ModelSummary, PendingIntegrationData,
    PlanExecutionData, PlanStateData, QueueClearData, SandboxStatusData, SessionActivationData,
    SessionCommandData, SubagentStartData, TreeNavigateData,
};
pub use timeout::{
    CONNECT_RETRY_DELAY, CONNECT_TIMEOUT, LOGIN_TIMEOUT, SESSION_TIMEOUT, TREE_NAVIGATION_TIMEOUT,
};
