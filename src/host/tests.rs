use crate::host::{
    ApprovalDecision, AuthProvidersData, BootstrapStateData, HostPlanModeData, PlanExecutionData,
    PlanStateData, SessionCommandData,
};
use crate::state::{
    ApprovalRulesSnapshot, ContextSnapshot, PlanExecutionContext, SessionBrowserSnapshot,
    SessionHistoryItem, SessionScope, SessionSortMode, TreeFilterMode, TreeSnapshot,
};
use serde_json::Value;
#[test]
fn parses_provider_catalog_without_secret_material() {
    let data: AuthProvidersData = serde_json::from_value(serde_json::json!({
        "providers": [{
            "id": "openai-codex",
            "name": "OpenAI Codex",
            "configured": false,
            "methods": [{
                "type": "oauth",
                "label": "Sign in with ChatGPT",
                "available": true
            }]
        }]
    }))
    .unwrap();
    assert_eq!(data.providers[0].id, "openai-codex");
    assert_eq!(data.providers[0].methods[0].kind, "oauth");
}
#[test]
fn parses_plan_mode_response_and_serializes_approval_decision() {
    let data: HostPlanModeData = serde_json::from_value(serde_json::json!({
        "active": true,
        "activeTools": ["read", "edit", "bash"]
    }))
    .unwrap();
    assert!(data.active);
    assert_eq!(data.active_tools, ["read", "edit", "bash"]);
    assert_eq!(
        serde_json::to_value(ApprovalDecision::AllowOnce).unwrap(),
        serde_json::json!("allow_once")
    );
    assert_eq!(
        serde_json::to_value(ApprovalDecision::AllowSession).unwrap(),
        serde_json::json!("allow_session")
    );
    assert_eq!(
        serde_json::to_value(ApprovalDecision::AllowWorkspace).unwrap(),
        serde_json::json!("allow_workspace")
    );
    assert_eq!(
        serde_json::to_value(ApprovalDecision::Deny).unwrap(),
        serde_json::json!("deny")
    );
    assert!(
        serde_json::from_value::<ApprovalDecision>(serde_json::json!("allow_forever")).is_err()
    );
}
#[test]
fn parses_plan_state_and_execution_responses() {
    let artifact = serde_json::json!({
        "id": "plan-1",
        "revision": 2,
        "title": "Plan",
        "summary": "Summary",
        "bodyMarkdown": "Implementation",
        "assumptions": [],
        "testPlan": ["cargo test"],
        "handoffMarkdown": "Handoff",
        "sourceSessionId": "session-1",
        "createdAt": "2026-01-01T00:00:00.000Z",
        "updatedAt": "2026-01-01T00:00:01.000Z"
    });
    let state: PlanStateData =
        serde_json::from_value(serde_json::json!({"artifact": artifact.clone()})).unwrap();
    let execution: PlanExecutionData = serde_json::from_value(serde_json::json!({
        "sessionId": "session-2",
        "context": "fresh"
    }))
    .unwrap();
    let current: PlanExecutionData = serde_json::from_value(serde_json::json!({
        "sessionId": "session-1",
        "context": "current"
    }))
    .unwrap();
    assert_eq!(state.artifact.unwrap().revision, 2);
    assert_eq!(execution.context, PlanExecutionContext::Fresh);
    assert_eq!(execution.session_id, "session-2");
    assert_eq!(current.context, PlanExecutionContext::Current);
    assert_eq!(
        serde_json::to_value(PlanExecutionContext::Current).unwrap(),
        serde_json::json!("current")
    );
    assert_eq!(
        serde_json::to_value(PlanExecutionContext::Fresh).unwrap(),
        serde_json::json!("fresh")
    );
}
#[test]
fn parses_atomic_bootstrap_state_with_pending_integrations() {
    let data: BootstrapStateData = serde_json::from_value(serde_json::json!({
        "scopeId": "session-1",
        "planMode": {"active": false, "activeTools": ["read"]},
        "plan": {"artifact": null},
        "resources": {
            "trusted": false,
            "contextFiles": [],
            "skills": [],
            "prompts": [],
            "extensions": [],
            "commands": [],
            "diagnostics": [],
            "revision": 1
        },
        "agents": {
            "maxParallel": 3,
            "profiles": [],
            "active": [],
            "pending": [],
            "diagnostics": []
        },
        "context": serde_json::to_value(ContextSnapshot::default()).unwrap(),
        "pendingIntegrations": [{
            "agent": {
                "id": "agent-1",
                "profile": "worker",
                "task": "Implement",
                "lifecycle": "awaiting_integration",
                "startedAt": "now",
                "turns": 1,
                "maxTurns": 4,
                "model": "test/model",
                "originSessionId": "session-1"
            },
            "integration": {
                "backend": "worktree",
                "status": "pending",
                "changedPaths": ["src/lib.rs"],
                "patchBytes": 12
            }
        }],
        "warnings": ["recovered"]
    }))
    .unwrap();
    assert_eq!(data.scope_id, "session-1");
    assert_eq!(data.pending_integrations.len(), 1);
    assert_eq!(data.pending_integrations[0].agent.id, "agent-1");
}
#[test]
fn shared_bootstrap_fixture_round_trips_without_dropping_host_fields() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../protocol-fixtures/bootstrap-state.json")).unwrap();
    let state: BootstrapStateData = serde_json::from_value(fixture.clone()).unwrap();
    let round_trip = serde_json::to_value(state).unwrap();
    assert!(fixture.get("goal").is_none());
    assert!(round_trip.get("goal").is_none());
    assert_eq!(round_trip, fixture);
}
#[test]
fn shared_turn_boundary_fixture_accepts_future_fields() {
    let history: Vec<SessionHistoryItem> = serde_json::from_str(include_str!(
        "../../protocol-fixtures/session-history-turn-boundary.json"
    ))
    .expect("turn boundary fixture");
    assert_eq!(
        history,
        vec![
            SessionHistoryItem::TurnBoundary {
                turn_id: "turn-exact".to_owned(),
                started_at: "2026-08-04T01:02:03.000Z".to_owned(),
                ended_at: "2026-08-04T01:03:08.000Z".to_owned(),
                duration_ms: 65_000,
                estimated: false,
            },
            SessionHistoryItem::TurnBoundary {
                turn_id: "legacy-entry-1".to_owned(),
                started_at: "2026-08-04T02:00:00.000Z".to_owned(),
                ended_at: "2026-08-04T02:00:12.000Z".to_owned(),
                duration_ms: 12_000,
                estimated: true,
            },
        ]
    );
}
#[test]
fn shared_persistent_approval_fixture_matches_rust_contract() {
    let snapshot: ApprovalRulesSnapshot = serde_json::from_str(include_str!(
        "../../protocol-fixtures/nabla.workspace-grants.v3.json"
    ))
    .unwrap();
    assert_eq!(snapshot.workspace, "/workspace");
    assert_eq!(snapshot.grants[0].proposal.scope, "workspace");
    assert_eq!(snapshot.grants[0].proposal.matchers[0]["kind"], "exec");
}
#[test]
fn parses_session_browser_activation_and_tree_payloads() {
    let browser: SessionBrowserSnapshot = serde_json::from_value(serde_json::json!({
        "browserId": "browser-1",
        "currentCwd": "/workspace/current",
        "scope": "all",
        "query": "parser",
        "sortMode": "relevance",
        "namedOnly": true,
        "sessions": [{
            "path": "/sessions/old.jsonl",
            "id": "session-old",
            "cwd": "/workspace/old",
            "cwdAvailable": false,
            "name": "Old work",
            "parentSessionPath": "/sessions/parent.jsonl",
            "createdAt": "2026-01-01T00:00:00.000Z",
            "modifiedAt": "2026-01-02T00:00:00.000Z",
            "messageCount": 12,
            "firstMessage": "fix parser",
            "depth": 1,
            "isLast": true,
            "current": false
        }],
        "total": 1
    }))
    .unwrap();
    assert_eq!(browser.scope, SessionScope::All);
    assert_eq!(browser.sort_mode, SessionSortMode::Relevance);
    assert!(!browser.sessions[0].cwd_available);
    let command: SessionCommandData = serde_json::from_value(serde_json::json!({
        "cancelled": false,
        "activation": {
            "state": {
                "model": {"provider": "test", "name": "fake"},
                "thinkingLevel": "off",
                "isStreaming": false,
                "isCompacting": false,
                "steeringMode": "one-at-a-time",
                "followUpMode": "one-at-a-time",
                "sessionFile": "/sessions/old.jsonl",
                "sessionId": "session-old",
                "sessionName": "Old work",
                "autoCompactionEnabled": true,
                "messageCount": 2,
                "pendingMessageCount": 0
            },
            "cwd": "/workspace/old",
            "planMode": false,
            "history": [
                {"kind": "user", "text": "fix parser"},
                {
                    "kind": "toolResult",
                    "id": "tool-1",
                    "name": "read",
                    "output": "source",
                    "isError": false
                }
            ],
            "plan": null,
            "context": serde_json::to_value(ContextSnapshot::default()).unwrap()
        }
    }))
    .unwrap();
    let activation = command.activation.expect("activation");
    assert_eq!(activation.state.session_id, "session-old");
    assert!(matches!(
        &activation.history[1],
        SessionHistoryItem::ToolResult {
            id,
            is_error: false,
            ..
        } if id == "tool-1"
    ));
    let tree: TreeSnapshot = serde_json::from_value(serde_json::json!({
        "items": [{
            "entryId": "entry-1",
            "parentId": null,
            "kind": "message",
            "role": "user",
            "preview": "user: fix parser",
            "visualDepth": 0,
            "showConnector": false,
            "gutterPositions": [],
            "isLast": true,
            "isActivePath": true,
            "isLeaf": true,
            "foldable": false,
            "folded": false
        }],
        "leafId": "entry-1",
        "filterMode": "no-tools",
        "query": ""
    }))
    .unwrap();
    assert_eq!(tree.filter_mode, TreeFilterMode::NoTools);
    assert_eq!(tree.leaf_id.as_deref(), Some("entry-1"));
}
