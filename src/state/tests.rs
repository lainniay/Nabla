use serde_json::json;

use super::*;

fn session(is_streaming: bool, is_compacting: bool) -> PiState {
    PiState {
        model: Some(json!({"provider": "test", "id": "model-1"})),
        thinking_level: "off".to_owned(),
        is_streaming,
        is_compacting,
        steering_mode: "one-at-a-time".to_owned(),
        follow_up_mode: "one-at-a-time".to_owned(),
        session_file: None,
        session_id: "session-1".to_owned(),
        session_name: None,
        auto_compaction_enabled: true,
        message_count: 0,
        pending_message_count: 0,
    }
}

#[test]
fn editor_preserves_paste_newlines_and_deletes_whole_graphemes() {
    let mut editor = EditorState::default();
    editor.insert_text("a\r\n你e\u{301}\r🙂");
    assert_eq!(editor.text(), "a\n你e\u{301}\n🙂");

    editor.backspace();
    assert_eq!(editor.text(), "a\n你e\u{301}\n");
    editor.backspace();
    editor.backspace();
    assert_eq!(editor.text(), "a\n你");
}

#[test]
fn maps_pi_session_flags_to_initial_run_state() {
    assert_eq!(
        AppState::new(session(false, false)).run_state,
        RunState::Idle
    );
    assert_eq!(
        AppState::new(session(true, false)).run_state,
        RunState::Running
    );
    assert_eq!(
        AppState::new(session(true, true)).run_state,
        RunState::Compacting
    );
}

#[test]
fn exposes_a_stable_model_label() {
    let state = AppState::new(session(false, false));

    assert_eq!(state.model_label(), "model-1");
    assert_eq!(state.connection_state, ConnectionState::Connected);
}

#[test]
fn context_snapshot_uses_the_camel_case_host_protocol() {
    let snapshot: ContextSnapshot = serde_json::from_value(json!({
        "usageState": "actual",
        "actualTokens": 47_000,
        "actualPercent": 47.0,
        "contextWindow": 100_000,
        "estimatedUnfilteredTokens": 55_000,
        "estimatedNextRequestTokens": 43_000,
        "categories": [{
            "category": "toolResult",
            "messageCount": 2,
            "estimatedTokens": 40_000
        }],
        "estimatedSystemToolOtherTokens": 8_000,
        "estimatedPrunedThisRequestTokens": 12_000,
        "estimatedCurrentlyPrunableTokens": 3_000,
        "estimatedCumulativeAvoidedTokens": 24_000,
        "pruning": [{
            "reason": "hard_limit",
            "count": 1,
            "estimatedTokensSaved": 12_000
        }],
        "topConsumers": [{
            "category": "toolResult",
            "label": "read result",
            "estimatedTokens": 40_000,
            "toolCallId": "call-1"
        }],
        "compactionCount": 1,
        "recentCompactions": [{
            "reason": "manual",
            "firstKeptEntryId": "entry-1",
            "tokensBefore": 82_000,
            "estimatedTokensAfter": 31_000,
            "tokensSaved": 51_000,
            "savedPercent": 62.2,
            "fileCount": 3,
            "readFileCount": 2,
            "modifiedFileCount": 2
        }],
        "policy": {
            "enabled": true,
            "recentToolResultTokens": 40_000,
            "minimumBatchSavingsTokens": 20_000,
            "minimumToolResultTokens": 50,
            "successToolResultLimitTokens": 12_000,
            "searchToolResultLimitTokens": 6_000,
            "errorToolResultLimitTokens": 8_000
        },
        "epoch": 2
    }))
    .unwrap();

    assert_eq!(snapshot.usage_state, ContextUsageState::Actual);
    assert_eq!(snapshot.categories[0].category, ContextCategory::ToolResult);
    assert_eq!(snapshot.pruning[0].reason, PruneReason::HardLimit);
    assert_eq!(snapshot.recent_compactions[0].file_count(), 3);
    let encoded = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(encoded["estimatedNextRequestTokens"], 43_000);
    assert!(encoded.get("estimated_next_request_tokens").is_none());
}

#[test]
fn agents_snapshot_uses_config_and_runtime_fields() {
    let snapshot: AgentsSnapshot = serde_json::from_value(json!({
        "maxParallel": 3,
        "profiles": [{
            "name": "reviewer",
            "description": "Review changes",
            "source": "builtin",
            "model": null,
            "thinkingLevel": "high",
            "skills": [],
            "tools": ["read"],
            "permission": "read:allow",
            "maxParallel": 1,
            "maxTurns": 12,
            "disabled": false,
            "unavailableReason": null
        }],
        "active": [{
            "id": "agent-1",
            "profile": "reviewer",
            "task": "Review",
            "lifecycle": "running",
            "startedAt": "2026-01-01T00:00:00Z",
            "turns": 2,
            "maxTurns": 12,
            "model": "test/model",
            "originSessionId": "session-1"
        }],
        "pending": [{
            "id": "agent-2",
            "profile": "worker",
            "task": "Implement",
            "lifecycle": "awaiting_integration",
            "startedAt": "2026-01-01T00:00:00Z",
            "turns": 3,
            "maxTurns": 32,
            "model": "test/model",
            "originSessionId": "session-1",
            "integrationStatus": "pending"
        }],
        "diagnostics": [{
            "type": "warning",
            "message": "example"
        }]
    }))
    .unwrap();

    assert_eq!(snapshot.profiles[0].max_turns, 12);
    assert_eq!(snapshot.active[0].origin_session_id, "session-1");
    assert_eq!(snapshot.pending[0].integration_status, "pending");
    assert_eq!(snapshot.diagnostics[0].kind, "warning");
}

#[test]
fn auth_choice_search_matches_provider_method_and_multiple_terms() {
    let choices = vec![
        AuthChoice {
            provider_id: "openai-codex".to_owned(),
            provider_name: "OpenAI Codex".to_owned(),
            auth_type: "oauth".to_owned(),
            label: "ChatGPT Plus/Pro".to_owned(),
            configured: false,
        },
        AuthChoice {
            provider_id: "github-copilot".to_owned(),
            provider_name: "GitHub Copilot".to_owned(),
            auth_type: "oauth".to_owned(),
            label: "Device login".to_owned(),
            configured: false,
        },
    ];

    assert_eq!(matching_auth_choice_indices(&choices, ""), vec![0, 1]);
    assert_eq!(
        matching_auth_choice_indices(&choices, "OPENAI plus"),
        vec![0]
    );
    assert_eq!(
        matching_auth_choice_indices(&choices, "github device"),
        vec![1]
    );
    assert!(matching_auth_choice_indices(&choices, "missing").is_empty());
}
