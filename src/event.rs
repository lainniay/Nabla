use crossterm::event::Event as TerminalEvent;
use serde_json::Value;

use crate::{
    file_references::{FileCandidate, PreparedPrompt, PromptDelivery},
    host::{
        ApprovalDecision, AuthLoginData, AuthProvider, HostPlanModeData, ModelListData,
        PlanExecutionData, PlanStateData, QueueClearData, SessionCommandData, SubagentStartData,
        TreeNavigateData,
    },
    rpc::RpcEvent,
    state::{
        AgentsSnapshot, ApprovalRulesSnapshot, ContextSnapshot, PlanExecutionTarget,
        ResourceSnapshot, SessionBrowserSnapshot, TreeSnapshot,
    },
};

/// The only event type allowed to enter the application reducer.
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// Keyboard, paste, resize, focus, and mouse events from Crossterm.
    Terminal(TerminalEvent),
    /// Asynchronous lifecycle and streaming events emitted by Pi.
    Pi(RpcEvent),
    /// Asynchronous host-control events such as authentication and approvals.
    Host(RpcEvent),
    /// Completion of a command previously requested through an `AppEffect`.
    Command(CommandEvent),
    /// Process and transport events that are not Pi protocol events.
    Runtime(RuntimeEvent),
}

#[derive(Debug, Clone, PartialEq)]
pub enum CommandEvent {
    FileSearchFinished {
        generation: u64,
        result: Result<Vec<FileCandidate>, String>,
    },
    ReferencesPrepared {
        delivery: PromptDelivery,
        result: Result<PreparedPrompt, String>,
    },
    PromptFinished(Result<(), String>),
    AbortFinished(Result<(), String>),
    CompactFinished(Result<Value, String>),
    ContextStateFinished(Result<Box<ContextSnapshot>, String>),
    ResourcesFinished(Result<Box<ResourceSnapshot>, String>),
    ResourceReloadFinished(Result<Box<ResourceSnapshot>, String>),
    WorkspaceTrustFinished(Result<Box<ResourceSnapshot>, String>),
    ApprovalRulesFinished(Result<Box<ApprovalRulesSnapshot>, String>),
    ApprovalRuleRevoked(Result<Box<ApprovalRulesSnapshot>, String>),
    ApprovalRulesCleared(Result<Box<ApprovalRulesSnapshot>, String>),
    QueueCleared(Result<Box<QueueClearData>, String>),
    AbortAndQueueCleared(Result<Box<QueueClearData>, String>),
    ModelsFinished(Result<Box<ModelListData>, String>),
    ModelSetFinished(Result<Value, String>),
    ThinkingSetFinished(Result<Value, String>),
    AgentsFinished(Result<Box<AgentsSnapshot>, String>),
    AgentsReloaded(Result<Box<AgentsSnapshot>, String>),
    SubagentStarted(Result<Box<SubagentStartData>, String>),
    SubagentCancelled(Result<(), String>),
    SubagentIntegrated(Result<Value, String>),
    SessionBrowserOpened(Result<Box<SessionBrowserSnapshot>, String>),
    SessionBrowserQueryFinished {
        generation: u64,
        result: Result<Box<SessionBrowserSnapshot>, String>,
    },
    SessionBrowserClosed(Result<(), String>),
    NewSessionFinished(Result<Box<SessionCommandData>, String>),
    ResumeSessionFinished(Result<Box<SessionCommandData>, String>),
    TreeStateFinished {
        generation: u64,
        result: Result<Box<TreeSnapshot>, String>,
    },
    TreeLabelFinished(Result<(), String>),
    TreeCopyFinished(Result<(), String>),
    TreeNavigateFinished(Result<Box<TreeNavigateData>, String>),
    TreeAbortFinished(Result<(), String>),
    AuthProvidersFinished(Result<Vec<AuthProvider>, String>),
    AuthLoginFinished(Result<AuthLoginData, String>),
    AuthReplyFinished(Result<(), String>),
    AuthCancelFinished(Result<(), String>),
    OpenUrlFinished(Result<(), String>),
    SetPlanModeFinished {
        requested: bool,
        result: Result<HostPlanModeData, String>,
    },
    PlanStateFinished(Result<Box<PlanStateData>, String>),
    QuestionReplyFinished(Result<(), String>),
    PlanExecutionFinished {
        target: PlanExecutionTarget,
        result: Result<Box<PlanExecutionData>, String>,
    },
    ApprovalReplyFinished {
        approval_id: String,
        decision: ApprovalDecision,
        result: Result<(), String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeEvent {
    PiStderr(String),
    PiRpcError(String),
    PiDisconnected,
    HostDisconnected,
    TerminalError(String),
    TerminalClosed,
}
