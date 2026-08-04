use crossterm::event::{KeyEvent, KeyModifiers};
use serde_json::json;

use super::*;
use crate::host::{
    AuthLoginData, AuthMethod, AuthProvider, HostPlanModeData, ModelListData, ModelSummary,
    PlanExecutionData, QueueClearData, SessionCommandData, TreeNavigateData,
};

fn state() -> PiState {
    PiState {
        model: Some(json!({"provider": "test", "name": "fake"})),
        thinking_level: "off".to_owned(),
        is_streaming: false,
        is_compacting: false,
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

fn press(code: KeyCode) -> AppEvent {
    AppEvent::Terminal(TerminalEvent::Key(KeyEvent::new(code, KeyModifiers::NONE)))
}

fn press_with(code: KeyCode, modifiers: KeyModifiers) -> AppEvent {
    AppEvent::Terminal(TerminalEvent::Key(KeyEvent::new(code, modifiers)))
}

fn plan(status: PlanStatus) -> PlanArtifact {
    PlanArtifact {
        schema_version: 2,
        id: "plan-1".to_owned(),
        revision: 2,
        status,
        title: "Structured planning".to_owned(),
        summary: "Make plans first-class.".to_owned(),
        body_markdown: "1. Ask questions.\n2. Submit a plan.".to_owned(),
        assumptions: vec!["Single-select questions".to_owned()],
        test_plan: vec!["Run cargo test".to_owned()],
        source_session_id: "session-1".to_owned(),
        created_at: "2026-01-01T00:00:00.000Z".to_owned(),
        updated_at: "2026-01-01T00:00:01.000Z".to_owned(),
        last_execution_error: None,
    }
}

fn session_summary(path: &str, id: &str, current: bool, cwd_available: bool) -> SessionSummary {
    SessionSummary {
        path: path.to_owned(),
        id: id.to_owned(),
        cwd: format!("/workspace/{id}"),
        cwd_available,
        name: Some(format!("Session {id}")),
        parent_session_path: None,
        created_at: "2026-01-01T00:00:00.000Z".to_owned(),
        modified_at: "2026-01-01T00:00:01.000Z".to_owned(),
        message_count: 2,
        first_message: format!("first message in {id}"),
        depth: 0,
        is_last: false,
        current,
    }
}

fn tree_item(entry_id: &str, parent_id: Option<&str>, is_leaf: bool, foldable: bool) -> TreeItem {
    TreeItem {
        entry_id: entry_id.to_owned(),
        parent_id: parent_id.map(ToOwned::to_owned),
        kind: "message".to_owned(),
        role: Some("user".to_owned()),
        preview: format!("user: {entry_id}"),
        label: None,
        label_timestamp: None,
        visual_depth: usize::from(parent_id.is_some()),
        show_connector: parent_id.is_some(),
        gutter_positions: Vec::new(),
        is_last: true,
        is_active_path: true,
        is_leaf,
        foldable,
        folded: false,
    }
}

fn activation(session_id: &str) -> SessionActivationData {
    let mut pi_state = state();
    pi_state.session_id = session_id.to_owned();
    pi_state.session_name = Some("Restored work".to_owned());
    pi_state.session_file = Some(format!("/sessions/{session_id}.jsonl"));
    pi_state.message_count = 4;
    SessionActivationData {
        state: pi_state,
        cwd: "/workspace/restored".to_owned(),
        plan_mode: false,
        goal: GoalSnapshot {
            scope_id: Some(session_id.to_owned()),
            goal: None,
            state_path: "/state/session.json".to_owned(),
        },
        history: vec![
            SessionHistoryItem::User {
                text: "restored question".to_owned(),
            },
            SessionHistoryItem::Assistant {
                text: "restored answer".to_owned(),
                thinking: "restored reasoning".to_owned(),
            },
            SessionHistoryItem::ToolCall {
                id: "tool-restored".to_owned(),
                name: "edit".to_owned(),
                args: json!({"path": "src/lib.rs"}),
            },
            SessionHistoryItem::ToolResult {
                id: "tool-restored".to_owned(),
                name: "edit".to_owned(),
                output: "restored source".to_owned(),
                details: Some(json!({
                    "diff": " 9 before\n-10 old\n+10 new\n 11 after",
                    "patch": "--- src/lib.rs\n+++ src/lib.rs\n@@ -9,3 +9,3 @@\n before\n-old\n+new\n after\n"
                })),
                is_error: false,
            },
            SessionHistoryItem::Compaction {
                first_kept_entry_id: "entry-kept".to_owned(),
                tokens_before: 82_000,
                file_count: 3,
            },
            SessionHistoryItem::BranchSummary {
                summary: "restored branch summary".to_owned(),
            },
        ],
        plan: Some(plan(PlanStatus::Executing)),
        context: ContextSnapshot {
            usage_state: ContextUsageState::Recalculating,
            epoch: 4,
            ..ContextSnapshot::default()
        },
    }
}

#[test]
fn clarification_questions_are_answered_sequentially_with_custom_input() {
    let mut app = App::new(state());
    app.state.run_state = RunState::Running;
    app.update(AppEvent::Host(RpcEvent {
        kind: "question_request".to_owned(),
        payload: json!({
            "requestId": "question-1",
            "questions": [
                {
                    "id": "scope",
                    "prompt": "Which scope?",
                    "options": [
                        {"id": "small", "label": "Small"},
                        {"id": "complete", "label": "Complete"}
                    ]
                },
                {
                    "id": "compat",
                    "prompt": "Compatibility target?",
                    "options": [
                        {"id": "current", "label": "Current"},
                        {"id": "legacy", "label": "Legacy"}
                    ]
                }
            ]
        }),
    }));

    app.update(press(KeyCode::Down));
    assert!(app.update(press(KeyCode::Enter)).is_empty());
    let flow = app.state.question.as_ref().expect("question flow");
    assert_eq!(flow.current, 1);
    assert_eq!(flow.answers[0].option_id.as_deref(), Some("complete"));

    app.update(press(KeyCode::BackTab));
    app.update(press(KeyCode::Enter));
    app.update(AppEvent::Terminal(TerminalEvent::Paste(
        "Rust 1.85+".to_owned(),
    )));
    let effects = app.update(press(KeyCode::Enter));

    assert!(matches!(
        effects.as_slice(),
        [AppEffect::ReplyQuestions {
            request_id,
            answers,
        }] if request_id == "question-1"
            && answers.len() == 2
            && answers[1].value == "Rust 1.85+"
            && answers[1].option_id.is_none()
    ));
}

#[test]
fn plan_review_current_context_requires_confirmation_and_leaves_plan_mode() {
    let mut app = App::new(state());
    app.state.plan_mode_active = true;
    let ready = plan(PlanStatus::Submitted);
    app.update(AppEvent::Host(RpcEvent {
        kind: "plan_ready".to_owned(),
        payload: json!({"artifact": ready}),
    }));

    assert!(matches!(
        app.state.plan_review,
        Some(PlanReviewState::Menu { selected: 0 })
    ));
    assert!(app.update(press(KeyCode::Enter)).is_empty());
    assert!(matches!(
        app.state.plan_review,
        Some(PlanReviewState::Confirm {
            target: PlanExecutionTarget::Current,
            submitting: false,
            ..
        })
    ));
    assert!(
        app.update(press_with(KeyCode::Char('n'), KeyModifiers::CONTROL,))
            .is_empty()
    );
    assert!(matches!(
        app.state.plan_review,
        Some(PlanReviewState::Confirm { selected: 1, .. })
    ));
    assert!(
        app.update(press_with(KeyCode::Char('p'), KeyModifiers::CONTROL,))
            .is_empty()
    );
    assert!(matches!(
        app.state.plan_review,
        Some(PlanReviewState::Confirm { selected: 0, .. })
    ));

    assert_eq!(
        app.update(press(KeyCode::Enter)),
        vec![AppEffect::ExecutePlan(PlanExecutionTarget::Current)]
    );
    let executing = plan(PlanStatus::Executing);
    app.update(AppEvent::Command(CommandEvent::PlanExecutionFinished {
        target: PlanExecutionTarget::Current,
        result: Ok(Box::new(PlanExecutionData {
            artifact: executing,
            session_id: "session-1".to_owned(),
            fresh: false,
        })),
    }));

    assert!(!app.state.plan_mode_active);
    assert_eq!(app.state.session.session_id, "session-1");
    assert!(app.state.plan_review.is_none());
}

#[test]
fn plan_review_fresh_context_is_a_distinct_execution_effect() {
    let mut app = App::new(state());
    app.update(AppEvent::Host(RpcEvent {
        kind: "plan_ready".to_owned(),
        payload: json!({"artifact": plan(PlanStatus::Submitted)}),
    }));

    app.update(press(KeyCode::Down));
    app.update(press(KeyCode::Enter));
    assert!(matches!(
        app.state.plan_review,
        Some(PlanReviewState::Confirm {
            target: PlanExecutionTarget::Fresh,
            submitting: false,
            ..
        })
    ));
    assert_eq!(
        app.update(press(KeyCode::Char('y'))),
        vec![AppEffect::ExecutePlan(PlanExecutionTarget::Fresh)]
    );
}

#[test]
fn editor_moves_and_deletes_on_unicode_boundaries() {
    let mut editor = EditorState::default();
    editor.insert_text("你a");
    editor.move_left();
    editor.backspace();

    assert_eq!(editor.text(), "a");
    assert_eq!(editor.cursor(), 0);
}

#[test]
fn multiline_input_keys_take_priority_over_send_while_streaming() {
    let mut app = App::new(state());
    app.state.run_state = RunState::Running;
    app.state.session.is_streaming = true;
    app.state.editor.insert_text("first");

    assert!(
        app.update(press_with(KeyCode::Enter, KeyModifiers::SHIFT))
            .is_empty()
    );
    app.update(press_with(KeyCode::Char('j'), KeyModifiers::CONTROL));

    assert_eq!(app.state.editor.text(), "first\n\n");
    assert!(
        app.state
            .transcript
            .iter()
            .all(|item| !matches!(item, TranscriptItem::User(_)))
    );
}

#[test]
fn enter_creates_pending_user_and_prompt_effect() {
    let mut app = App::new(state());
    app.update(press(KeyCode::Char('h')));
    app.update(press(KeyCode::Char('i')));
    let effects = app.update(press(KeyCode::Enter));

    assert_eq!(effects, vec![AppEffect::Prompt("hi".to_owned())]);
    assert_eq!(app.state().run_state, RunState::Submitting);
    assert!(matches!(
        app.state().transcript.last(),
        Some(TranscriptItem::User(UserMessage {
            status: UserMessageStatus::Pending,
            ..
        }))
    ));
}

#[test]
fn file_references_prepare_before_transcript_or_editor_commit() {
    let mut app = App::new(state());
    app.state.editor.insert_text("Review @src/lib.rs");
    let effects = app.update(press(KeyCode::Enter));
    assert_eq!(
        effects,
        vec![AppEffect::PrepareReferences {
            message: "Review @src/lib.rs".to_owned(),
            delivery: PromptDelivery::Prompt,
        }]
    );
    assert_eq!(app.state.run_state, RunState::PreparingReferences);
    assert_eq!(app.state.editor.text(), "Review @src/lib.rs");
    assert!(app.state.transcript.is_empty());
    app.update(press(KeyCode::Char('x')));
    assert_eq!(app.state.editor.text(), "Review @src/lib.rs");

    let prepared = PreparedPrompt {
        original_message: "Review @src/lib.rs".to_owned(),
        message: format!("{}{{}}", crate::file_references::ENVELOPE_PREFIX),
        images: Vec::new(),
    };
    let effects = app.update(AppEvent::Command(CommandEvent::ReferencesPrepared {
        delivery: PromptDelivery::Prompt,
        result: Ok(prepared.clone()),
    }));
    assert_eq!(
        effects,
        vec![AppEffect::DeliverPrepared {
            prompt: prepared,
            delivery: PromptDelivery::Prompt,
        }]
    );
    assert!(app.state.editor.text().is_empty());
    assert!(matches!(
        app.state.transcript.last(),
        Some(TranscriptItem::User(UserMessage { text, .. }))
            if text == "Review @src/lib.rs"
    ));
}

#[test]
fn failed_reference_preparation_preserves_input_and_stale_search_is_ignored() {
    let mut app = App::new(state());
    app.state.editor.insert_text("Read @missing.txt");
    app.update(press(KeyCode::Enter));
    app.update(AppEvent::Command(CommandEvent::ReferencesPrepared {
        delivery: PromptDelivery::Prompt,
        result: Err("Unable to resolve missing.txt".to_owned()),
    }));
    assert_eq!(app.state.editor.text(), "Read @missing.txt");
    assert_eq!(app.state.run_state, RunState::Idle);
    assert!(
        app.state.transcript.iter().any(
            |item| matches!(item, TranscriptItem::Error(error) if error.contains("missing.txt"))
        )
    );

    app.state.editor.clear();
    let first = app.update(press(KeyCode::Char('@')));
    let second = app.update(press(KeyCode::Char('s')));
    let first_generation = match first.as_slice() {
        [AppEffect::SearchFiles { generation, .. }] => *generation,
        other => panic!("unexpected effects: {other:?}"),
    };
    let second_generation = match second.as_slice() {
        [AppEffect::SearchFiles { generation, .. }] => *generation,
        other => panic!("unexpected effects: {other:?}"),
    };
    assert!(second_generation > first_generation);
    app.update(AppEvent::Command(CommandEvent::FileSearchFinished {
        generation: first_generation,
        result: Ok(vec![crate::file_references::FileCandidate {
            path: "stale".to_owned(),
            basename: "stale".to_owned(),
            parent: String::new(),
            size: 0,
        }]),
    }));
    assert!(
        app.state
            .file_completion
            .as_ref()
            .is_some_and(|completion| completion.candidates.is_empty())
    );
}

#[test]
fn accepting_file_completion_adds_exactly_one_space_and_preserves_suffix() {
    let mut app = App::new(state());
    app.state.editor.insert_text("@src");
    let completion = FileCompletionState {
        query: "src".to_owned(),
        token_range: 0..4,
        generation: 1,
        candidates: vec![crate::file_references::FileCandidate {
            path: "src/lib.rs".to_owned(),
            basename: "lib.rs".to_owned(),
            parent: "src".to_owned(),
            size: 0,
        }],
        selected: 0,
        loading: false,
        error: None,
    };
    app.state.file_completion = Some(completion.clone());

    app.accept_file_completion();

    assert_eq!(app.state.editor.text(), "@src/lib.rs ");
    assert_eq!(app.state.editor.cursor(), "@src/lib.rs ".chars().count());

    app.state.editor.replace("@src next".to_owned());
    app.state.file_completion = Some(completion);
    app.accept_file_completion();

    assert_eq!(app.state.editor.text(), "@src/lib.rs next");
    assert_eq!(app.state.editor.cursor(), "@src/lib.rs ".chars().count());
}

#[test]
fn file_completion_keeps_visible_candidates_while_refreshing() {
    let mut app = App::new(state());
    app.state.editor.insert_text("@s");
    app.state.file_completion_generation = 1;
    let selected = crate::file_references::FileCandidate {
        path: "src/lib.rs".to_owned(),
        basename: "lib.rs".to_owned(),
        parent: "src".to_owned(),
        size: 10,
    };
    app.state.file_completion = Some(FileCompletionState {
        query: "s".to_owned(),
        token_range: 0..2,
        generation: 1,
        candidates: vec![selected.clone()],
        selected: 0,
        loading: false,
        error: None,
    });

    let effects = app.update(press(KeyCode::Char('r')));
    let generation = match effects.as_slice() {
        [AppEffect::SearchFiles { generation, query }] if query == "sr" => *generation,
        other => panic!("unexpected effects: {other:?}"),
    };
    let completion = app.state.file_completion.as_ref().unwrap();
    assert!(completion.loading);
    assert_eq!(completion.candidates, vec![selected.clone()]);

    app.update(AppEvent::Command(CommandEvent::FileSearchFinished {
        generation,
        result: Ok(vec![
            crate::file_references::FileCandidate {
                path: "scripts/run.rs".to_owned(),
                basename: "run.rs".to_owned(),
                parent: "scripts".to_owned(),
                size: 20,
            },
            selected.clone(),
        ]),
    }));
    let completion = app.state.file_completion.as_ref().unwrap();
    assert!(!completion.loading);
    assert_eq!(completion.selected, 1);
    assert_eq!(completion.candidates[completion.selected], selected);
}

#[test]
fn local_commands_do_not_expand_file_references() {
    let mut app = App::new(state());
    app.state.editor.insert_text("/goal inspect @missing.txt");
    let effects = app.update(press(KeyCode::Enter));
    assert_eq!(
        effects,
        vec![AppEffect::StartGoal {
            objective: Some("inspect @missing.txt".to_owned()),
            from_plan: false,
        }]
    );
    assert!(app.state.editor.text().is_empty());
}

#[test]
fn running_input_uses_pi_steer_follow_up_and_restores_cleared_queue() {
    let mut app = App::new(state());
    app.state.run_state = RunState::Running;
    app.state.session.is_streaming = true;
    app.state.editor.insert_text("steer now");

    assert_eq!(
        app.update(press(KeyCode::Enter)),
        vec![AppEffect::Steer("steer now".to_owned())]
    );

    app.state.editor.insert_text("after completion");
    assert_eq!(
        app.update(press_with(KeyCode::Enter, KeyModifiers::ALT)),
        vec![AppEffect::FollowUp("after completion".to_owned())]
    );
    app.update(AppEvent::Pi(RpcEvent {
        kind: "queue_update".to_owned(),
        payload: json!({
            "type": "queue_update",
            "steering": ["steer now"],
            "followUp": ["after completion"]
        }),
    }));
    assert_eq!(app.state().session.pending_message_count, 2);
    assert_eq!(
        app.update(press_with(KeyCode::Up, KeyModifiers::ALT)),
        vec![AppEffect::ClearQueue]
    );

    app.update(AppEvent::Command(CommandEvent::QueueCleared(Ok(Box::new(
        QueueClearData {
            steering: vec!["steer now".to_owned()],
            follow_up: vec!["after completion".to_owned()],
            restored_text: "steer now\n\nafter completion".to_owned(),
        },
    )))));
    assert_eq!(app.state().editor.text(), "steer now\n\nafter completion");
}

#[test]
fn harness_commands_are_local_and_goal_is_explicit() {
    let mut app = App::new(state());
    assert!(!app.state().plan_mode_active);

    for (source, expected) in [
        ("/resources", AppEffect::GetResources),
        ("/reload", AppEffect::ReloadResources),
        ("/goals", AppEffect::GetGoals),
        ("/agents", AppEffect::GetAgents),
        ("/agents reload", AppEffect::ReloadAgents),
        (
            "/agents apply agent-7",
            AppEffect::IntegrateSubagent {
                agent_id: "agent-7".to_owned(),
                action: "apply".to_owned(),
            },
        ),
    ] {
        app.state.editor.replace(source.to_owned());
        assert_eq!(app.update(press(KeyCode::Enter)), vec![expected]);
    }
    app.state
        .editor
        .replace("/goal implement leases".to_owned());
    assert_eq!(
        app.update(press(KeyCode::Enter)),
        vec![AppEffect::StartGoal {
            objective: Some("implement leases".to_owned()),
            from_plan: false,
        }]
    );
    app.state.editor.replace("/goal from-plan".to_owned());
    assert_eq!(
        app.update(press(KeyCode::Enter)),
        vec![AppEffect::StartGoal {
            objective: None,
            from_plan: true,
        }]
    );
    assert!(
        !app.state()
            .transcript
            .iter()
            .any(|item| matches!(item, TranscriptItem::User(_)))
    );
}

#[test]
fn direct_subagent_command_is_local_and_backgrounded() {
    let mut app = App::new(state());
    app.state
        .editor
        .replace("/agent reviewer review the diff".to_owned());

    assert_eq!(
        app.update(press(KeyCode::Enter)),
        vec![AppEffect::StartSubagent {
            profile: "reviewer".to_owned(),
            task: "review the diff".to_owned(),
        }]
    );
    assert!(
        !app.state()
            .transcript
            .iter()
            .any(|item| matches!(item, TranscriptItem::User(_)))
    );
}

#[test]
fn agent_picker_completes_a_profile_without_starting_it() {
    let mut app = App::new(state());
    app.state.agents = serde_json::from_value(json!({
        "maxParallel": 3,
        "profiles": [
            {
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
            },
            {
                "name": "explorer",
                "description": "Explore the codebase",
                "source": "builtin",
                "model": null,
                "thinkingLevel": "medium",
                "skills": [],
                "tools": ["read"],
                "permission": "read:allow",
                "maxParallel": 1,
                "maxTurns": 12,
                "disabled": false,
                "unavailableReason": null
            },
            {
                "name": "worker",
                "description": "Implement changes",
                "source": "builtin",
                "model": null,
                "thinkingLevel": "high",
                "skills": [],
                "tools": ["read", "write"],
                "permission": "write:allow",
                "maxParallel": 1,
                "maxTurns": 12,
                "disabled": false,
                "unavailableReason": null
            }
        ],
        "active": [],
        "diagnostics": []
    }))
    .unwrap();
    app.state.editor.replace("/agent".to_owned());

    assert!(app.update(press(KeyCode::Enter)).is_empty());
    assert!(app.state.agent_picker.is_some());
    app.update(press_with(KeyCode::Char('n'), KeyModifiers::CONTROL));
    assert_eq!(app.state.agent_picker.as_ref().unwrap().selected, 1);
    app.update(press(KeyCode::Tab));
    assert_eq!(app.state.agent_picker.as_ref().unwrap().selected, 2);
    app.update(press_with(KeyCode::Tab, KeyModifiers::SHIFT));
    assert_eq!(app.state.agent_picker.as_ref().unwrap().selected, 1);
    app.update(press_with(KeyCode::Char('p'), KeyModifiers::CONTROL));
    assert_eq!(app.state.agent_picker.as_ref().unwrap().selected, 0);
    app.update(press(KeyCode::BackTab));
    assert_eq!(app.state.agent_picker.as_ref().unwrap().selected, 2);
    assert!(app.update(press(KeyCode::Enter)).is_empty());
    assert!(app.state.agent_picker.is_none());
    assert_eq!(app.state.editor.text(), "/agent worker ");
}

#[test]
fn model_and_thinking_commands_use_the_shared_selection_panel() {
    let mut app = App::new(state());
    app.state.editor.replace("/thinking".to_owned());

    assert!(app.update(press(KeyCode::Enter)).is_empty());
    assert_eq!(app.state.active_modal_kind(), Some(UiModalKind::Selection));
    let thinking = app.state.selection_panel.as_ref().unwrap();
    assert_eq!(thinking.title, "Select thinking level");
    assert_eq!(thinking.options.len(), THINKING_LEVELS.len());
    assert_eq!(thinking.selected, 0);

    app.update(press(KeyCode::Tab));
    assert_eq!(app.state.selection_panel.as_ref().unwrap().selected, 1);
    assert_eq!(
        app.update(press(KeyCode::Enter)),
        vec![AppEffect::SetThinking("minimal".to_owned())]
    );
    assert!(app.state.selection_panel.is_none());
    app.update(AppEvent::Command(CommandEvent::ThinkingSetFinished(Ok(
        json!({"level": "minimal"}),
    ))));
    assert_eq!(app.state.session.thinking_level, "minimal");

    app.state.editor.replace("/model".to_owned());
    assert_eq!(
        app.update(press(KeyCode::Enter)),
        vec![AppEffect::ListModels]
    );
    assert!(app.state.selection_panel.as_ref().unwrap().loading);

    app.update(AppEvent::Command(CommandEvent::ModelsFinished(Ok(
        Box::new(ModelListData {
            current: Some(json!({"provider": "provider-b", "id": "model-b"})),
            models: vec![
                ModelSummary {
                    provider: "provider-a".to_owned(),
                    id: "model-a".to_owned(),
                    name: "Model A".to_owned(),
                    reasoning: false,
                    context_window: 32_000,
                },
                ModelSummary {
                    provider: "provider-b".to_owned(),
                    id: "model-b".to_owned(),
                    name: "Model B".to_owned(),
                    reasoning: true,
                    context_window: 64_000,
                },
            ],
        }),
    ))));

    let model = app.state.selection_panel.as_ref().unwrap();
    assert!(!model.loading);
    assert_eq!(model.selected, 1);
    assert_eq!(model.options[1].label, "Model B");
    assert!(model.options[1].description.contains("current"));
    assert_eq!(
        app.update(press(KeyCode::Enter)),
        vec![AppEffect::SetModel {
            provider: "provider-b".to_owned(),
            model_id: "model-b".to_owned(),
        }]
    );
    assert!(app.state.selection_panel.is_none());
}

#[test]
fn subagent_lifecycle_updates_active_state_and_structured_transcript() {
    let mut app = App::new(state());
    let agent = json!({
        "id": "agent-1",
        "profile": "reviewer",
        "task": "Review",
        "taskId": null,
        "goalId": null,
        "lifecycle": "running",
        "startedAt": "2026-01-01T00:00:00Z",
        "turns": 2,
        "maxTurns": 12,
        "model": "test/model",
        "originSessionId": "session-1"
    });
    app.update(AppEvent::Host(RpcEvent {
        kind: "subagent_state".to_owned(),
        payload: json!({"event": "started", "agent": agent.clone()}),
    }));
    assert_eq!(app.state.agents.active.len(), 1);

    app.update(AppEvent::Host(RpcEvent {
        kind: "subagent_state".to_owned(),
        payload: json!({
            "event": "completed",
            "agent": agent,
            "result": {"status": "completed", "summary": "Looks good"}
        }),
    }));
    assert!(app.state.agents.active.is_empty());
    assert!(app.state.transcript.iter().any(|item| matches!(
        item,
        TranscriptItem::Subagent(SubagentTranscript { event, .. })
            if event == "completed"
    )));
}

#[test]
fn pending_worktree_result_opens_integration_choices() {
    let mut app = App::new(state());
    app.update(AppEvent::Host(RpcEvent {
        kind: "subagent_integration".to_owned(),
        payload: json!({
            "event": "pending",
            "agent": {
                "id": "agent-9",
                "profile": "worker",
                "task": "Implement",
                "lifecycle": "awaiting_integration",
                "startedAt": "2026-01-01T00:00:00Z",
                "turns": 3,
                "maxTurns": 32,
                "model": "test/model",
                "originSessionId": "session-1",
                "isolationBackend": "worktree",
                "integrationStatus": "pending"
            },
            "integration": {
                "backend": "worktree",
                "status": "pending",
                "artifactId": "artifact-1",
                "changedPaths": ["src/lib.rs"],
                "patchBytes": 123
            }
        }),
    }));
    assert!(app.state.integration_prompt.is_some());
    assert_eq!(
        app.update(press(KeyCode::Enter)),
        vec![AppEffect::IntegrateSubagent {
            agent_id: "agent-9".to_owned(),
            action: "apply".to_owned(),
        }]
    );
}

#[test]
fn recoverable_host_warnings_are_visible_in_the_transcript() {
    let mut app = App::new(state());
    app.update(AppEvent::Host(RpcEvent {
        kind: "host_warning".to_owned(),
        payload: json!({"message": "worktree recovery could not be persisted"}),
    }));

    assert!(app.state.transcript.iter().any(|item| matches!(
        item,
        TranscriptItem::Error(message)
            if message == "worktree recovery could not be persisted"
    )));
}

#[test]
fn integration_prompts_queue_and_advance_without_overwrite() {
    let mut app = App::new(state());
    let event = |id: &str, lifecycle: &str| {
        AppEvent::Host(RpcEvent {
            kind: "subagent_integration".to_owned(),
            payload: json!({
                "event": lifecycle,
                "agent": {
                    "id": id,
                    "profile": "worker",
                    "task": "Implement",
                    "lifecycle": "awaiting_integration",
                    "startedAt": "2026-01-01T00:00:00Z",
                    "turns": 3,
                    "maxTurns": 32,
                    "model": "test/model",
                    "originSessionId": "session-1",
                    "isolationBackend": "worktree",
                    "integrationStatus": lifecycle
                },
                "integration": {
                    "backend": "worktree",
                    "status": lifecycle,
                    "artifactId": format!("artifact-{id}"),
                    "changedPaths": ["src/lib.rs"],
                    "patchBytes": 123
                }
            }),
        })
    };
    app.update(event("agent-1", "pending"));
    app.update(event("agent-2", "pending"));
    assert_eq!(
        app.state
            .integration_prompt
            .as_ref()
            .map(|prompt| prompt.agent.id.as_str()),
        Some("agent-1")
    );
    assert_eq!(app.state.integration_prompt_queue.len(), 1);

    app.update(event("agent-1", "applied"));
    assert_eq!(
        app.state
            .integration_prompt
            .as_ref()
            .map(|prompt| prompt.agent.id.as_str()),
        Some("agent-2")
    );
    assert!(app.state.integration_prompt_queue.is_empty());
}

#[test]
fn modal_priority_routes_keys_to_the_visible_question() {
    let mut app = App::new(state());
    app.update(press_with(KeyCode::Char('o'), KeyModifiers::CONTROL));
    assert_eq!(app.state.active_modal_kind(), Some(UiModalKind::Transcript));
    app.update(AppEvent::Host(RpcEvent {
        kind: "subagent_integration".to_owned(),
        payload: json!({
            "event": "pending",
            "agent": {
                "id": "agent-9", "profile": "worker", "task": "Implement",
                "lifecycle": "awaiting_integration", "startedAt": "now",
                "turns": 1, "maxTurns": 3, "model": "test/model",
                "originSessionId": "session-1"
            },
            "integration": {
                "backend": "worktree", "status": "pending",
                "changedPaths": [], "patchBytes": 1
            }
        }),
    }));
    app.update(AppEvent::Host(RpcEvent {
        kind: "question_request".to_owned(),
        payload: json!({
            "requestId": "question-visible",
            "questions": [{
                "id": "choice",
                "prompt": "Choose",
                "options": [
                    {"id": "first", "label": "First"},
                    {"id": "second", "label": "Second"}
                ]
            }]
        }),
    }));
    assert_eq!(app.state.active_modal_kind(), Some(UiModalKind::Question));
    app.update(press(KeyCode::Down));
    assert!(matches!(
        app.update(press(KeyCode::Enter)).as_slice(),
        [AppEffect::ReplyQuestions { request_id, answers }]
            if request_id == "question-visible"
                && answers[0].option_id.as_deref() == Some("second")
    ));
    assert!(app.state.integration_prompt.is_some());
    assert!(app.state.transcript_viewer.is_some());
}

#[test]
fn newer_goal_lifecycle_event_wins_over_an_earlier_rpc_response() {
    let mut app = App::new(state());
    let snapshot = |revision: u64, stage: &str| {
        serde_json::from_value::<GoalSnapshot>(json!({
            "goal": {
                "id": "goal-1",
                "sessionId": "session-1",
                "objective": "Implement",
                "stage": stage,
                "revision": revision,
                "constraints": [],
                "acceptanceCriteria": [],
                "tasks": [],
                "reviews": [],
                "repairCycles": 0
            },
            "statePath": "/state/goal.json"
        }))
        .unwrap()
    };
    app.update(AppEvent::Host(RpcEvent {
        kind: "goal_state".to_owned(),
        payload: json!({
            "type": "goal_state",
            "snapshot": serde_json::to_value(snapshot(2, "blocked")).unwrap()
        }),
    }));
    app.update(AppEvent::Command(CommandEvent::GoalStarted(Ok(Box::new(
        snapshot(1, "preparing"),
    )))));

    let goal = app
        .state()
        .goal
        .as_ref()
        .and_then(|snapshot| snapshot.goal.as_ref())
        .unwrap();
    assert_eq!(goal.revision, 2);
    assert_eq!(goal.stage, "blocked");
    assert_ne!(app.state().run_state, RunState::Submitting);
    assert!(!app.state().plan_mode_active);
}

#[test]
fn stale_plan_status_and_foreign_scope_snapshots_cannot_replace_current_state() {
    let mut app = App::new(state());
    let mut current = plan(PlanStatus::Executing);
    current.updated_at = "2026-01-01T00:00:03.000Z".to_owned();
    app.state.plan = Some(current.clone());

    let mut stale = plan(PlanStatus::Submitted);
    stale.updated_at = "2026-01-01T00:00:02.000Z".to_owned();
    app.update(AppEvent::Command(CommandEvent::PlanStateFinished(Ok(
        Box::new(crate::host::PlanStateData {
            scope_id: Some("session-1".to_owned()),
            artifact: Some(stale),
        }),
    ))));
    assert_eq!(
        app.state.plan.as_ref().unwrap().status,
        PlanStatus::Executing
    );

    let mut foreign = plan(PlanStatus::Completed);
    foreign.updated_at = "2026-01-01T00:00:04.000Z".to_owned();
    app.update(AppEvent::Command(CommandEvent::PlanStateFinished(Ok(
        Box::new(crate::host::PlanStateData {
            scope_id: Some("session-other".to_owned()),
            artifact: Some(foreign),
        }),
    ))));
    assert_eq!(app.state.plan.as_ref().unwrap(), &current);
}

#[test]
fn stale_cross_goal_and_lower_revision_snapshots_are_ignored() {
    let mut app = App::new(state());
    let snapshot = |id: &str, revision: u64, updated_at: &str| {
        serde_json::from_value::<GoalSnapshot>(json!({
            "scopeId": "session-1",
            "goal": {
                "id": id,
                "sessionId": "session-1",
                "objective": "Implement",
                "stage": "executing",
                "revision": revision,
                "constraints": [],
                "acceptanceCriteria": [],
                "tasks": [],
                "reviews": [],
                "repairCycles": 0,
                "updatedAt": updated_at
            },
            "statePath": "/state/goal.json"
        }))
        .unwrap()
    };
    assert!(app.receive_goal(snapshot("goal-new", 2, "2026-01-01T00:00:03.000Z"), false,));
    assert!(!app.receive_goal(snapshot("goal-old", 99, "2026-01-01T00:00:02.000Z"), false,));
    assert!(!app.receive_goal(snapshot("goal-new", 1, "2026-01-01T00:00:04.000Z"), false,));
    assert_eq!(
        app.state
            .goal
            .as_ref()
            .and_then(|snapshot| snapshot.goal.as_ref())
            .unwrap()
            .id,
        "goal-new"
    );
}

#[test]
fn host_events_from_a_previous_session_scope_are_ignored() {
    let mut app = App::new(state());
    let initial_revision = app.state.resources.revision;
    app.update(AppEvent::Host(RpcEvent {
        kind: "resource_state".to_owned(),
        payload: json!({
            "scopeId": "session-old",
            "snapshot": {
                "scopeId": "session-old",
                "trusted": true,
                "contextFiles": [],
                "skills": [],
                "prompts": [],
                "extensions": [],
                "commands": [],
                "diagnostics": [],
                "revision": initial_revision + 10
            }
        }),
    }));

    assert_eq!(app.state.resources.revision, initial_revision);
    assert!(!app.state.resources.trusted);
}

#[test]
fn workspace_state_updates_resources_and_agents_atomically() {
    let mut app = App::new(state());
    let event = |agents: Value| {
        AppEvent::Host(RpcEvent {
            kind: "workspace_state".to_owned(),
            payload: json!({
                "scopeId": "session-1",
                "resources": {
                    "scopeId": "session-1",
                    "trusted": true,
                    "contextFiles": ["AGENTS.md"],
                    "skills": [],
                    "prompts": [],
                    "extensions": [],
                    "commands": [],
                    "diagnostics": [],
                    "revision": 2
                },
                "agents": agents
            }),
        })
    };

    app.update(event(json!({"invalid": true})));
    assert!(!app.state.resources.trusted);
    assert_eq!(app.state.agents.revision, 0);

    app.update(event(json!({
        "scopeId": "session-1",
        "revision": 2,
        "maxParallel": 3,
        "profiles": [],
        "active": [],
        "pending": [],
        "diagnostics": []
    })));
    assert!(app.state.resources.trusted);
    assert_eq!(app.state.agents.revision, 2);
}

#[test]
fn goal_spec_approval_is_independent_from_plan_mode_and_plan_review() {
    let mut app = App::new(state());
    app.state.plan_mode_active = true;
    app.update(AppEvent::Host(RpcEvent {
        kind: "goal_spec_ready".to_owned(),
        payload: json!({
            "snapshot": {
                "goal": {
                    "id": "goal-1",
                    "sessionId": "session-1",
                    "objective": "Background work",
                    "stage": "awaiting_approval",
                    "revision": 2,
                    "constraints": [],
                    "acceptanceCriteria": ["cargo test"],
                    "spec": {
                        "revision": 1,
                        "summary": "Execute independently",
                        "acceptanceCriteria": ["cargo test"],
                        "allowedTools": ["read"],
                        "allowedPaths": ["."],
                        "allowedCommands": []
                    },
                    "tasks": [],
                    "reviews": [],
                    "repairCycles": 0
                },
                "statePath": "/state/goal.json"
            }
        }),
    }));

    assert!(app.state.plan_mode_active);
    assert!(app.state.plan_review.is_none());
    assert!(matches!(
        app.state.goal_approval,
        Some(GoalApprovalState {
            submitting: false,
            ..
        })
    ));
    assert_eq!(
        app.update(press(KeyCode::Enter)),
        vec![AppEffect::ApproveGoal]
    );
}

#[test]
fn prompt_acceptance_does_not_finish_the_agent_run() {
    let mut app = App::new(state());
    app.state.run_state = RunState::Submitting;
    app.update(AppEvent::Command(CommandEvent::PromptFinished(Ok(()))));
    assert_eq!(app.state().run_state, RunState::Submitting);

    app.update(AppEvent::Pi(RpcEvent {
        kind: "agent_start".to_owned(),
        payload: json!({"type": "agent_start"}),
    }));
    assert_eq!(app.state().run_state, RunState::Running);
    assert!(app.state().session.is_streaming);

    app.update(AppEvent::Pi(RpcEvent {
        kind: "agent_end".to_owned(),
        payload: json!({"type": "agent_end", "messages": []}),
    }));
    assert_eq!(app.state().run_state, RunState::Idle);
    assert!(!app.state().session.is_streaming);
}

#[test]
fn completed_turn_timing_is_idempotent_and_correlated_by_turn_id() {
    let mut app = App::new(state());
    app.update(AppEvent::Host(RpcEvent {
        kind: "turn_timing".to_owned(),
        payload: json!({
            "type": "turn_timing",
            "phase": "started",
            "scopeId": "session-1",
            "turnId": "turn-1",
            "startedAt": "2026-08-04T01:00:00.000Z"
        }),
    }));
    assert!(
        app.state()
            .transcript
            .iter()
            .all(|item| !matches!(item, TranscriptItem::TurnSeparator(_)))
    );

    for duration_ms in [1_000, 2_000] {
        app.update(AppEvent::Host(RpcEvent {
            kind: "turn_timing".to_owned(),
            payload: json!({
                "type": "turn_timing",
                "phase": "completed",
                "scopeId": "session-1",
                "turnId": "turn-1",
                "startedAt": "2026-08-04T01:00:00.000Z",
                "endedAt": "2026-08-04T01:00:02.000Z",
                "durationMs": duration_ms,
                "unknownFutureField": true
            }),
        }));
    }

    let separators = app
        .state()
        .transcript
        .iter()
        .filter_map(|item| match item {
            TranscriptItem::TurnSeparator(separator) => Some(separator),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(separators.len(), 1);
    assert_eq!(separators[0].turn_id, "turn-1");
    assert_eq!(separators[0].duration_ms, 2_000);
    assert!(!separators[0].estimated);

    app.update(AppEvent::Host(RpcEvent {
        kind: "turn_timing".to_owned(),
        payload: json!({
            "phase": "completed",
            "scopeId": "session-1",
            "turnId": "turn-2",
            "startedAt": "2026-08-04T01:01:00.000Z",
            "endedAt": "2026-08-04T01:01:00.500Z",
            "durationMs": 500
        }),
    }));
    assert_eq!(
        app.state()
            .transcript
            .iter()
            .filter(|item| matches!(item, TranscriptItem::TurnSeparator(_)))
            .count(),
        2
    );
}

#[test]
fn restored_turn_boundaries_keep_exact_and_estimated_metadata() {
    let mut app = App::new(state());
    app.append_history_item(SessionHistoryItem::TurnBoundary {
        turn_id: "legacy-turn".to_owned(),
        started_at: "2026-08-04T02:00:00.000Z".to_owned(),
        ended_at: "2026-08-04T02:00:12.000Z".to_owned(),
        duration_ms: 12_000,
        estimated: true,
    });
    assert!(matches!(
        app.state().transcript.last(),
        Some(TranscriptItem::TurnSeparator(TurnSeparator {
            turn_id,
            duration_ms: 12_000,
            estimated: true,
            ..
        })) if turn_id == "legacy-turn"
    ));
}

#[test]
fn command_failure_enters_the_same_reducer_as_pi_events() {
    let mut app = App::new(state());
    app.state.editor.insert_text("hello");
    app.update(press(KeyCode::Enter));

    app.update(AppEvent::Command(CommandEvent::PromptFinished(Err(
        "request failed".to_owned(),
    ))));

    assert_eq!(app.state().run_state, RunState::Error);
    assert!(matches!(
        &app.state().transcript[0],
        TranscriptItem::User(UserMessage {
            status: UserMessageStatus::Failed,
            ..
        })
    ));
    assert!(matches!(
        &app.state().transcript[1],
        TranscriptItem::Error(error) if error == "request failed"
    ));
}

#[test]
fn login_is_handled_locally_instead_of_becoming_a_prompt() {
    let mut app = App::new(state());
    app.state.editor.insert_text("/login");

    let effects = app.update(press(KeyCode::Enter));

    assert_eq!(effects, vec![AppEffect::AuthList]);
    assert_eq!(app.state().run_state, RunState::Authenticating);
    assert!(matches!(
        app.state().auth_state,
        AuthState::LoadingProviders
    ));
    assert!(matches!(
        &app.state().transcript[0],
        TranscriptItem::User(UserMessage {
            status: UserMessageStatus::Accepted,
            ..
        })
    ));
}

#[test]
fn login_provider_list_filters_and_selects_from_the_search_input() {
    let mut app = App::new(state());
    app.state.auth_state = AuthState::LoadingProviders;
    app.state.run_state = RunState::Authenticating;
    app.update(AppEvent::Command(CommandEvent::AuthProvidersFinished(Ok(
        vec![
            AuthProvider {
                id: "openai-codex".to_owned(),
                name: "OpenAI Codex".to_owned(),
                configured: false,
                configured_type: None,
                configured_source: None,
                methods: vec![AuthMethod {
                    kind: "oauth".to_owned(),
                    label: "ChatGPT Plus/Pro".to_owned(),
                    available: true,
                }],
            },
            AuthProvider {
                id: "github-copilot".to_owned(),
                name: "GitHub Copilot".to_owned(),
                configured: false,
                configured_type: None,
                configured_source: None,
                methods: vec![AuthMethod {
                    kind: "oauth".to_owned(),
                    label: "GitHub device login".to_owned(),
                    available: true,
                }],
            },
        ],
    ))));

    app.update(press(KeyCode::Char('/')));
    app.update(AppEvent::Terminal(TerminalEvent::Paste(
        "github device".to_owned(),
    )));
    let AuthState::Selecting {
        choices,
        selected,
        filter,
        ..
    } = &app.state().auth_state
    else {
        panic!("expected searchable provider selection");
    };
    assert_eq!(filter.text(), "github device");
    assert_eq!(*selected, 0);
    assert_eq!(
        matching_auth_choice_indices(choices, filter.text()),
        vec![1]
    );

    assert!(app.update(press(KeyCode::Enter)).is_empty());
    let effects = app.update(press(KeyCode::Enter));
    assert!(matches!(
        effects.as_slice(),
        [AppEffect::AuthLogin {
            provider_id,
            auth_type,
            ..
        }] if provider_id == "github-copilot" && auth_type == "oauth"
    ));
}

#[test]
fn oauth_url_is_stored_opened_once_and_rejects_unsafe_schemes() {
    let mut app = App::new(state());
    app.state.run_state = RunState::Authenticating;
    app.state.auth_state = AuthState::Running(Box::new(AuthFlowState {
        id: "flow-1".to_owned(),
        provider_name: "OpenAI Codex".to_owned(),
        status: "Starting login…".to_owned(),
        url: None,
        device_code: None,
        prompt: None,
    }));
    let event = || {
        AppEvent::Host(RpcEvent {
            kind: "auth_notify".to_owned(),
            payload: json!({
                "type": "auth_notify",
                "flowId": "flow-1",
                "event": {
                    "type": "auth_url",
                    "url": "https://auth.openai.com/oauth/authorize?state=test",
                    "instructions": "Continue in your browser"
                }
            }),
        })
    };

    assert_eq!(
        app.update(event()),
        vec![AppEffect::OpenUrl(
            "https://auth.openai.com/oauth/authorize?state=test".to_owned()
        )]
    );
    assert!(app.update(event()).is_empty());
    let AuthState::Running(flow) = &app.state().auth_state else {
        panic!("expected active auth flow");
    };
    assert_eq!(
        flow.url.as_deref(),
        Some("https://auth.openai.com/oauth/authorize?state=test")
    );

    let effects = app.update(AppEvent::Host(RpcEvent {
        kind: "auth_notify".to_owned(),
        payload: json!({
            "type": "auth_notify",
            "flowId": "flow-1",
            "event": {
                "type": "auth_url",
                "url": "javascript:alert(1)"
            }
        }),
    }));
    assert!(effects.is_empty());
}

#[test]
fn login_provider_secret_prompt_and_completion_stay_inside_auth_state() {
    let mut app = App::new(state());
    app.state.editor.insert_text("/login");
    app.update(press(KeyCode::Enter));
    app.update(AppEvent::Command(CommandEvent::AuthProvidersFinished(Ok(
        vec![AuthProvider {
            id: "test".to_owned(),
            name: "Test Provider".to_owned(),
            configured: false,
            configured_type: None,
            configured_source: None,
            methods: vec![AuthMethod {
                kind: "api_key".to_owned(),
                label: "API key".to_owned(),
                available: true,
            }],
        }],
    ))));

    let effects = app.update(press(KeyCode::Enter));
    assert!(matches!(
        effects.as_slice(),
        [AppEffect::AuthLogin {
            provider_id,
            auth_type,
            ..
        }] if provider_id == "test" && auth_type == "api_key"
    ));
    let AuthState::Running(flow) = &app.state().auth_state else {
        panic!("expected active auth flow");
    };
    let flow_id = flow.id.clone();

    app.update(AppEvent::Host(RpcEvent {
        kind: "auth_prompt".to_owned(),
        payload: json!({
            "type": "auth_prompt",
            "flowId": flow_id,
            "promptId": "prompt-1",
            "promptType": "secret",
            "message": "Enter API key"
        }),
    }));
    app.update(press(KeyCode::Char('s')));
    app.update(press(KeyCode::Char('k')));

    let effects = app.update(press(KeyCode::Enter));
    assert!(matches!(
        effects.as_slice(),
        [AppEffect::AuthReply {
            prompt_id,
            value,
            ..
        }] if prompt_id == "prompt-1" && value.expose() == "sk"
    ));
    assert_eq!(app.state().transcript.len(), 1);

    app.update(AppEvent::Command(CommandEvent::AuthLoginFinished(Ok(
        AuthLoginData {
            provider_id: "test".to_owned(),
            credential_type: "api_key".to_owned(),
            selected_model: None,
        },
    ))));
    assert!(matches!(app.state().auth_state, AuthState::Inactive));
    assert_eq!(app.state().run_state, RunState::Idle);
}

#[test]
fn compact_uses_the_dedicated_rpc_effect() {
    let mut app = App::new(state());
    app.state.editor.insert_text("/compact keep decisions");

    let effects = app.update(press(KeyCode::Enter));

    assert_eq!(
        effects,
        vec![AppEffect::Compact(Some("keep decisions".to_owned()))]
    );
    assert_eq!(app.state().run_state, RunState::Compacting);
    assert!(
        !app.state()
            .transcript
            .iter()
            .any(|item| matches!(item, TranscriptItem::User(_)))
    );
}

#[test]
fn context_is_a_read_only_local_query_and_never_becomes_a_user_message() {
    let mut app = App::new(state());
    app.state.editor.insert_text("/context");

    assert_eq!(
        app.update(press(KeyCode::Enter)),
        vec![AppEffect::GetContextState]
    );
    assert!(app.state().transcript.is_empty());

    let snapshot = ContextSnapshot {
        context_window: Some(200_000),
        estimated_next_request_tokens: 94_000,
        ..ContextSnapshot::default()
    };
    app.update(AppEvent::Command(CommandEvent::ContextStateFinished(Ok(
        Box::new(snapshot.clone()),
    ))));

    assert_eq!(app.state().context, snapshot);
    assert!(matches!(
        app.state().transcript.as_slice(),
        [
            TranscriptItem::Context(_),
            TranscriptItem::TurnSeparator(separator)
        ] if separator.turn_id.starts_with("local-")
    ));
}

#[test]
fn resource_commands_finish_with_a_local_worked_for_boundary() {
    let mut app = App::new(state());
    app.state.editor.insert_text("/resources");

    assert_eq!(
        app.update(press(KeyCode::Enter)),
        vec![AppEffect::GetResources]
    );
    assert!(app.state().transcript.is_empty());

    app.update(AppEvent::Command(CommandEvent::ResourcesFinished(Ok(
        Box::new(ResourceSnapshot::default()),
    ))));

    assert!(matches!(
        app.state().transcript.as_slice(),
        [
            TranscriptItem::Resources(_),
            TranscriptItem::TurnSeparator(separator)
        ] if separator.turn_id.starts_with("local-") && !separator.estimated
    ));
}

#[test]
fn shift_tab_and_plan_command_switch_only_after_host_confirmation() {
    let mut app = App::new(state());
    assert!(!app.state().plan_mode_active);
    let transcript_len = app.state().transcript.len();

    let effects = app.update(press(KeyCode::BackTab));
    assert_eq!(effects, vec![AppEffect::SetPlanMode(true)]);
    assert!(!app.state().plan_mode_active);
    assert_eq!(app.state().pending_plan_mode, Some(true));

    app.update(AppEvent::Command(CommandEvent::SetPlanModeFinished {
        requested: true,
        result: Ok(HostPlanModeData {
            active: true,
            active_tools: vec![
                "read".to_owned(),
                "grep".to_owned(),
                "find".to_owned(),
                "ls".to_owned(),
            ],
        }),
    }));
    assert!(app.state().plan_mode_active);
    assert_eq!(app.state().pending_plan_mode, None);
    assert_eq!(app.state().transcript.len(), transcript_len);

    app.state.editor.insert_text("/plan exit");
    assert_eq!(
        app.update(press(KeyCode::Enter)),
        vec![AppEffect::SetPlanMode(false)]
    );
    assert_eq!(app.state().transcript.len(), transcript_len);
}

#[test]
fn plan_mode_switch_is_rejected_while_agent_is_running() {
    let mut app = App::new(state());
    app.state.run_state = RunState::Running;
    let transcript_len = app.state().transcript.len();

    assert!(app.update(press(KeyCode::BackTab)).is_empty());
    assert!(!app.state().plan_mode_active);
    assert_eq!(app.state().transcript.len(), transcript_len);
}

#[test]
fn mutation_tool_waits_for_approval_and_resumes_after_allow() {
    let mut app = App::new(state());
    app.state.run_state = RunState::Running;
    app.update(AppEvent::Pi(RpcEvent {
        kind: "tool_execution_start".to_owned(),
        payload: json!({
            "type": "tool_execution_start",
            "toolCallId": "call-1",
            "toolName": "bash",
            "args": {"command": "cargo test"}
        }),
    }));

    app.update(AppEvent::Host(RpcEvent {
        kind: "approval_request".to_owned(),
        payload: json!({
            "type": "approval_request",
            "approvalId": "approval-1",
            "toolCallId": "call-1",
            "toolName": "bash",
            "input": {"command": "cargo test"}
        }),
    }));
    assert!(matches!(
        app.state().transcript.last(),
        Some(TranscriptItem::Tool(ToolExecution {
            status: ToolStatus::WaitingApproval,
            ..
        }))
    ));

    let effects = app.update(press(KeyCode::Char('y')));
    assert_eq!(
        effects,
        vec![AppEffect::ReplyApproval {
            approval_id: "approval-1".to_owned(),
            decision: ApprovalDecision::Allow,
        }]
    );

    app.update(AppEvent::Command(CommandEvent::ApprovalReplyFinished {
        approval_id: "approval-1".to_owned(),
        decision: ApprovalDecision::Allow,
        result: Ok(()),
    }));
    assert!(app.state().approval.is_none());
    assert!(matches!(
        app.state().transcript.last(),
        Some(TranscriptItem::Tool(ToolExecution {
            status: ToolStatus::Running,
            ..
        }))
    ));
}

#[test]
fn approval_interrupt_denies_and_aborts_the_agent() {
    let mut app = App::new(state());
    app.state.run_state = RunState::Running;
    app.state.approval = Some(ApprovalState {
        approval_id: "approval-1".to_owned(),
        tool_call_id: "call-1".to_owned(),
        tool_name: "write".to_owned(),
        input: json!({"path": "src/lib.rs", "content": "changed"}),
        agent_id: None,
        agent_profile: None,
        model: None,
        goal_id: None,
        reason: None,
        risk: None,
        selected: 0,
        replying: false,
    });

    assert_eq!(
        app.update(press(KeyCode::Esc)),
        vec![AppEffect::AbortAndClearQueue]
    );
    assert_eq!(app.state().run_state, RunState::Aborting);
}

#[test]
fn approval_and_goal_approval_use_shared_navigation_before_confirming() {
    let mut app = App::new(state());
    app.state.run_state = RunState::Running;
    app.state.approval = Some(ApprovalState {
        approval_id: "approval-1".to_owned(),
        tool_call_id: "call-1".to_owned(),
        tool_name: "write".to_owned(),
        input: json!({"path": "src/lib.rs"}),
        agent_id: None,
        agent_profile: None,
        model: None,
        goal_id: Some("goal-1".to_owned()),
        reason: None,
        risk: None,
        selected: 0,
        replying: false,
    });

    assert!(
        app.update(press_with(KeyCode::Char('n'), KeyModifiers::CONTROL,))
            .is_empty()
    );
    assert_eq!(app.state.approval.as_ref().unwrap().selected, 1);
    assert!(app.update(press(KeyCode::Tab)).is_empty());
    assert_eq!(app.state.approval.as_ref().unwrap().selected, 2);
    assert!(
        app.update(press_with(KeyCode::Char('p'), KeyModifiers::CONTROL,))
            .is_empty()
    );
    assert_eq!(app.state.approval.as_ref().unwrap().selected, 1);
    assert!(app.update(press(KeyCode::Tab)).is_empty());
    assert_eq!(app.state.approval.as_ref().unwrap().selected, 2);
    assert!(app.update(press(KeyCode::Tab)).is_empty());
    assert_eq!(app.state.approval.as_ref().unwrap().selected, 3);
    assert!(app.update(press(KeyCode::Tab)).is_empty());
    assert_eq!(
        app.update(press(KeyCode::Enter)),
        vec![AppEffect::ReplyApproval {
            approval_id: "approval-1".to_owned(),
            decision: ApprovalDecision::Deny,
        }]
    );

    app.state.approval = None;
    app.state.goal_approval = Some(GoalApprovalState {
        selected: 0,
        submitting: false,
    });
    assert!(
        app.update(press_with(KeyCode::Char('n'), KeyModifiers::CONTROL,))
            .is_empty()
    );
    assert_eq!(app.state.goal_approval.as_ref().unwrap().selected, 1);
    assert!(
        app.update(press_with(KeyCode::Char('p'), KeyModifiers::CONTROL,))
            .is_empty()
    );
    assert_eq!(app.state.goal_approval.as_ref().unwrap().selected, 0);
    assert!(app.update(press(KeyCode::Tab)).is_empty());
    assert!(app.update(press(KeyCode::Enter)).is_empty());
    assert!(app.state.goal_approval.is_none());
}

#[test]
fn persistent_approval_is_available_only_for_normal_risk_requests() {
    let mut app = App::new(state());
    app.state.run_state = RunState::Running;
    app.state.approval = Some(ApprovalState {
        approval_id: "approval-forever".to_owned(),
        tool_call_id: "call-forever".to_owned(),
        tool_name: "write".to_owned(),
        input: json!({"path": "src/lib.rs"}),
        agent_id: None,
        agent_profile: None,
        model: None,
        goal_id: None,
        reason: None,
        risk: Some("normal".to_owned()),
        selected: 0,
        replying: false,
    });
    assert_eq!(
        app.update(press(KeyCode::Char('a'))),
        vec![AppEffect::ReplyApproval {
            approval_id: "approval-forever".to_owned(),
            decision: ApprovalDecision::AllowForever,
        }]
    );
    app.state.approval.as_mut().unwrap().replying = false;
    assert_eq!(
        app.update(press(KeyCode::Char('s'))),
        vec![AppEffect::ReplyApproval {
            approval_id: "approval-forever".to_owned(),
            decision: ApprovalDecision::AllowSession,
        }]
    );

    app.state.approval = Some(ApprovalState {
        approval_id: "approval-high".to_owned(),
        tool_call_id: "call-high".to_owned(),
        tool_name: "bash".to_owned(),
        input: json!({"command": "sudo true"}),
        agent_id: None,
        agent_profile: None,
        model: None,
        goal_id: None,
        reason: None,
        risk: Some("high".to_owned()),
        selected: 0,
        replying: false,
    });
    assert!(app.update(press(KeyCode::Char('a'))).is_empty());
}

#[test]
fn permissions_command_opens_and_manages_project_rules() {
    let mut app = App::new(state());
    app.state.editor.insert_text("/permissions");
    assert_eq!(
        app.update(press(KeyCode::Enter)),
        vec![AppEffect::GetApprovalRules]
    );

    app.update(AppEvent::Command(CommandEvent::ApprovalRulesFinished(Ok(
        Box::new(ApprovalRulesSnapshot {
            workspace: "/workspace".to_owned(),
            rules: vec![crate::state::PersistentApprovalRule {
                id: "rule-1".to_owned(),
                workspace: "/workspace".to_owned(),
                tool_name: "bash".to_owned(),
                kind: "command".to_owned(),
                value: "cargo test".to_owned(),
                recursive: false,
                summary: "cargo test".to_owned(),
                created_at: "2026-08-04T00:00:00.000Z".to_owned(),
            }],
        }),
    ))));
    assert_eq!(
        app.state.active_modal_kind(),
        Some(UiModalKind::Permissions)
    );
    assert_eq!(
        app.update(press(KeyCode::Char('d'))),
        vec![AppEffect::RevokeApprovalRule("rule-1".to_owned())]
    );
    assert_eq!(
        app.update(press(KeyCode::Char('c'))),
        vec![AppEffect::ClearApprovalRules]
    );
    assert!(app.update(press(KeyCode::Esc)).is_empty());
    assert!(app.state.permission_manager.is_none());
}

#[test]
fn shared_choice_navigation_skips_disabled_rows_and_numbers_only_select() {
    let mut selected = 0;
    let enabled = [true, false, true];
    assert_eq!(
        update_choice_navigation(
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            &mut selected,
            &enabled,
        ),
        ChoiceNavAction::Handled
    );
    assert_eq!(selected, 2);
    assert_eq!(
        update_choice_navigation(
            KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE),
            &mut selected,
            &enabled,
        ),
        ChoiceNavAction::Handled
    );
    assert_eq!(selected, 2, "a disabled numeric choice must be ignored");
    assert_eq!(
        update_choice_navigation(
            KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE),
            &mut selected,
            &enabled,
        ),
        ChoiceNavAction::Handled
    );
    assert_eq!(selected, 0);
    assert_eq!(
        update_choice_navigation(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut selected,
            &enabled,
        ),
        ChoiceNavAction::Confirm(0)
    );
}

#[test]
fn tool_updates_replace_accumulated_output_and_keep_denied_status() {
    let mut app = App::new(state());
    app.update(AppEvent::Pi(RpcEvent {
        kind: "tool_execution_start".to_owned(),
        payload: json!({
            "type": "tool_execution_start",
            "toolCallId": "call-1",
            "toolName": "bash",
            "args": {"command": "printf test"}
        }),
    }));
    app.update(AppEvent::Pi(RpcEvent {
        kind: "tool_execution_update".to_owned(),
        payload: json!({
            "type": "tool_execution_update",
            "toolCallId": "call-1",
            "partialResult": {
                "content": [{"type": "text", "text": "test"}]
            }
        }),
    }));
    let Some(TranscriptItem::Tool(tool)) = app.state.transcript.last_mut() else {
        panic!("expected tool");
    };
    assert_eq!(tool.output, "test");
    tool.status = ToolStatus::Denied;

    app.update(AppEvent::Pi(RpcEvent {
        kind: "tool_execution_end".to_owned(),
        payload: json!({
            "type": "tool_execution_end",
            "toolCallId": "call-1",
            "result": {"content": [{"type": "text", "text": "Denied by user"}]},
            "isError": true
        }),
    }));
    assert!(matches!(
        app.state().transcript.last(),
        Some(TranscriptItem::Tool(ToolExecution {
            status: ToolStatus::Denied,
            output,
            ..
        })) if output == "Denied by user"
    ));
}

#[test]
fn successful_edit_tool_results_preserve_structured_diff() {
    let mut app = App::new(state());
    app.update(AppEvent::Pi(RpcEvent {
        kind: "tool_execution_start".to_owned(),
        payload: json!({
            "type": "tool_execution_start",
            "toolCallId": "edit-1",
            "toolName": "edit",
            "args": {"path": "src/lib.rs"}
        }),
    }));
    app.update(AppEvent::Pi(RpcEvent {
        kind: "tool_execution_end".to_owned(),
        payload: json!({
            "type": "tool_execution_end",
            "toolCallId": "edit-1",
            "result": {
                "content": [{"type": "text", "text": "Edited src/lib.rs"}],
                "details": {
                    "diff": " 9 before\n-10 old\n+10 new\n 11 after",
                    "patch": "--- src/lib.rs\n+++ src/lib.rs\n@@ -9,3 +9,3 @@\n before\n-old\n+new\n after\n"
                }
            },
            "isError": false
        }),
    }));

    let Some(TranscriptItem::Tool(tool)) = app.state().transcript.last() else {
        panic!("expected edit tool");
    };
    let diff = tool.diff.as_ref().expect("structured diff");
    assert_eq!(diff.files[0].path, "src/lib.rs");
    assert_eq!((diff.additions, diff.deletions), (1, 1));
}

#[test]
fn discovered_commands_are_passed_through_and_unknown_commands_are_rejected() {
    let mut app = App::with_commands(
        state(),
        vec![DiscoveredCommand {
            name: "fix-tests".to_owned(),
            description: "Fix failing tests".to_owned(),
            source: "prompt".to_owned(),
        }],
    );
    app.state.editor.insert_text("/fix-tests src/parser");

    let effects = app.update(press(KeyCode::Enter));
    assert_eq!(
        effects,
        vec![AppEffect::Prompt("/fix-tests src/parser".to_owned())]
    );

    app.state.run_state = RunState::Idle;
    app.state.editor.insert_text("/missing");
    let effects = app.update(press(KeyCode::Enter));
    assert!(effects.is_empty());
    assert!(matches!(
        app.state().transcript.last(),
        Some(TranscriptItem::Notice(message)) if message.contains("Unknown command /missing")
    ));
}

#[test]
fn tab_shift_tab_and_ctrl_np_navigate_command_candidates() {
    let mut app = App::with_commands(
        state(),
        vec![DiscoveredCommand {
            name: "fix-tests".to_owned(),
            description: "Fix failing tests".to_owned(),
            source: "prompt".to_owned(),
        }],
    );
    app.state.editor.insert_text("/");

    app.update(press(KeyCode::Tab));
    assert_eq!(
        app.state()
            .selected_command()
            .map(|command| command.name.as_str()),
        Some("compact")
    );
    assert_eq!(app.state().editor.text(), "/");

    app.update(press_with(KeyCode::BackTab, KeyModifiers::SHIFT));
    assert_eq!(
        app.state()
            .selected_command()
            .map(|command| command.name.as_str()),
        Some("login")
    );

    app.update(press_with(KeyCode::Char('n'), KeyModifiers::CONTROL));
    assert_eq!(
        app.state()
            .selected_command()
            .map(|command| command.name.as_str()),
        Some("compact")
    );

    app.update(press_with(KeyCode::Char('p'), KeyModifiers::CONTROL));
    assert_eq!(
        app.state()
            .selected_command()
            .map(|command| command.name.as_str()),
        Some("login")
    );

    let effects = app.update(press(KeyCode::Enter));
    assert!(effects.is_empty());
    assert_eq!(app.state().editor.text(), "/login ");
}

#[test]
fn command_navigation_reaches_candidates_beyond_the_visible_window() {
    let mut app = App::new(state());
    app.state.editor.insert_text("/");

    assert_eq!(app.state().command_candidates().len(), 18);
    for _ in 0..17 {
        app.update(press(KeyCode::Tab));
    }
    assert_eq!(
        app.state()
            .selected_command()
            .map(|command| command.name.as_str()),
        Some("tree")
    );

    assert!(app.update(press(KeyCode::Enter)).is_empty());
    assert_eq!(app.state().editor.text(), "/tree ");
}

#[test]
fn command_menu_supports_navigation_enter_and_escape() {
    let mut app = App::with_commands(
        state(),
        vec![DiscoveredCommand {
            name: "fix-tests".to_owned(),
            description: "Fix failing tests".to_owned(),
            source: "prompt".to_owned(),
        }],
    );
    app.state.editor.insert_text("/");
    app.state.reset_command_menu();

    app.update(press(KeyCode::Down));
    assert_eq!(
        app.state()
            .selected_command()
            .map(|command| command.name.as_str()),
        Some("compact")
    );

    let effects = app.update(press(KeyCode::Enter));
    assert!(effects.is_empty());
    assert_eq!(app.state().editor.text(), "/compact ");

    app.state.editor.clear();
    app.state.editor.insert_text("/");
    app.state.reset_command_menu();
    app.update(press(KeyCode::Esc));
    assert!(app.state().command_candidates().is_empty());
    assert_eq!(app.state().editor.text(), "/");

    app.update(press(KeyCode::Char('f')));
    assert_eq!(
        app.state()
            .selected_command()
            .map(|command| command.name.as_str()),
        Some("fix-tests")
    );
}

#[test]
fn missing_credentials_enters_auth_required_and_blocks_more_prompts() {
    let mut app = App::new(state());
    app.state.editor.insert_text("hello");
    app.update(press(KeyCode::Enter));

    app.update(AppEvent::Command(CommandEvent::PromptFinished(Err(
        "Pi command prompt failed: No API key found. Use /login.".to_owned(),
    ))));

    assert_eq!(app.state().run_state, RunState::AuthRequired);
    assert!(matches!(
        &app.state().transcript[0],
        TranscriptItem::User(UserMessage {
            status: UserMessageStatus::Failed,
            ..
        })
    ));
    assert!(matches!(
        &app.state().transcript[2],
        TranscriptItem::Notice(message) if message.contains("Use /login")
    ));

    app.state.editor.insert_text("try again");
    let effects = app.update(press(KeyCode::Enter));
    assert!(effects.is_empty());
    assert_eq!(app.state().run_state, RunState::AuthRequired);
}

#[test]
fn fatal_runtime_event_is_returned_as_an_effect() {
    let mut app = App::new(state());

    let effects = app.update(AppEvent::Runtime(RuntimeEvent::TerminalError(
        "input stream closed unexpectedly".to_owned(),
    )));

    assert_eq!(
        effects,
        vec![AppEffect::ExitWithError(
            "terminal input failed: input stream closed unexpectedly".to_owned()
        )]
    );
    assert_eq!(app.state().run_state, RunState::Error);
}

#[test]
fn runtime_disconnect_maps_to_connection_and_error_state() {
    let mut app = App::new(state());

    app.update(AppEvent::Runtime(RuntimeEvent::PiDisconnected));

    assert_eq!(app.state().connection_state, ConnectionState::Disconnected);
    assert_eq!(app.state().run_state, RunState::Error);
    assert_eq!(
        app.state().last_error.as_deref(),
        Some("Pi process disconnected")
    );
}

#[test]
fn compaction_events_keep_session_and_ui_phase_in_sync() {
    let mut app = App::new(state());

    app.update(AppEvent::Pi(RpcEvent {
        kind: "compaction_start".to_owned(),
        payload: json!({"type": "compaction_start"}),
    }));
    assert_eq!(app.state().run_state, RunState::Compacting);
    assert!(app.state().session.is_compacting);

    app.update(AppEvent::Pi(RpcEvent {
        kind: "compaction_end".to_owned(),
        payload: json!({
            "type": "compaction_end",
            "reason": "manual",
            "aborted": false,
            "willRetry": false,
            "result": {
                "summary": "must not be rendered",
                "firstKeptEntryId": "entry-4",
                "tokensBefore": 82_000,
                "estimatedTokensAfter": 31_000,
                "details": {
                    "readFiles": ["src/a.rs", "src/b.rs"],
                    "modifiedFiles": ["src/b.rs", "src/c.rs"]
                }
            }
        }),
    }));
    assert_eq!(app.state().run_state, RunState::Idle);
    assert!(!app.state().session.is_compacting);
    assert_eq!(
        app.state()
            .transcript
            .iter()
            .filter(|item| matches!(item, TranscriptItem::Compaction(_)))
            .count(),
        1
    );
    let Some(TranscriptItem::Compaction(record)) = app.state().transcript.last() else {
        panic!("expected compaction separator");
    };
    assert_eq!(record.tokens_saved, Some(51_000));
    assert_eq!(record.file_count, 3);
    assert!(!app.state().transcript.iter().any(
        |item| matches!(item, TranscriptItem::Notice(text) if text.contains("must not be rendered"))
    ));

    app.update(AppEvent::Pi(RpcEvent {
        kind: "compaction_end".to_owned(),
        payload: json!({
            "type": "compaction_end",
            "reason": "manual",
            "aborted": false,
            "willRetry": false,
            "result": {
                "firstKeptEntryId": "entry-4",
                "tokensBefore": 82_000,
                "estimatedTokensAfter": 31_000,
                "details": {}
            }
        }),
    }));
    assert_eq!(
        app.state()
            .transcript
            .iter()
            .filter(|item| matches!(item, TranscriptItem::Compaction(_)))
            .count(),
        1,
        "duplicate lifecycle events must not add another separator"
    );
}

#[test]
fn failed_aborted_and_overflow_compactions_are_explicit_without_success_separators() {
    for (reason, aborted, will_retry, error) in [
        ("manual", true, false, None),
        ("threshold", false, false, Some("summary provider failed")),
        ("overflow", false, true, Some("overflow recovery failed")),
    ] {
        let mut app = App::new(state());
        app.update(AppEvent::Pi(RpcEvent {
            kind: "compaction_start".to_owned(),
            payload: json!({"type": "compaction_start", "reason": reason}),
        }));
        app.update(AppEvent::Pi(RpcEvent {
            kind: "compaction_end".to_owned(),
            payload: json!({
                "type": "compaction_end",
                "reason": reason,
                "aborted": aborted,
                "willRetry": will_retry,
                "result": null,
                "errorMessage": error
            }),
        }));

        assert!(
            app.state()
                .transcript
                .iter()
                .any(|item| matches!(item, TranscriptItem::Error(_)))
        );
        assert!(
            !app.state()
                .transcript
                .iter()
                .any(|item| matches!(item, TranscriptItem::Compaction(_)))
        );
    }
}

#[test]
fn lifecycle_completion_wins_over_the_later_compact_rpc_response() {
    let mut app = App::new(state());
    app.state.editor.insert_text("/compact");
    app.update(press(KeyCode::Enter));
    app.update(AppEvent::Pi(RpcEvent {
        kind: "compaction_start".to_owned(),
        payload: json!({"type": "compaction_start", "reason": "manual"}),
    }));
    app.update(AppEvent::Pi(RpcEvent {
        kind: "compaction_end".to_owned(),
        payload: json!({
            "type": "compaction_end",
            "reason": "manual",
            "aborted": false,
            "willRetry": false,
            "result": {
                "firstKeptEntryId": "kept",
                "tokensBefore": 50_000,
                "details": {}
            }
        }),
    }));
    let transcript_len = app.state().transcript.len();

    app.update(AppEvent::Command(CommandEvent::CompactFinished(Err(
        "late RPC error".to_owned(),
    ))));

    assert_eq!(app.state().run_state, RunState::Idle);
    assert_eq!(app.state().transcript.len(), transcript_len);
    assert!(
        !app.state()
            .transcript
            .iter()
            .any(|item| matches!(item, TranscriptItem::Error(error) if error == "late RPC error"))
    );
}

#[test]
fn context_budget_events_update_status_without_querying_or_disturbing_active_ui_state() {
    let mut app = App::new(state());
    app.state.plan_mode_active = false;
    app.state.approval = Some(ApprovalState {
        approval_id: "approval-1".to_owned(),
        tool_call_id: "call-1".to_owned(),
        tool_name: "write".to_owned(),
        input: json!({"path": "src/lib.rs"}),
        agent_id: None,
        agent_profile: None,
        model: None,
        goal_id: None,
        reason: None,
        risk: None,
        selected: 0,
        replying: false,
    });
    let snapshot = ContextSnapshot {
        usage_state: ContextUsageState::Actual,
        actual_tokens: Some(47_000),
        actual_percent: Some(47.0),
        context_window: Some(100_000),
        ..ContextSnapshot::default()
    };

    app.update(AppEvent::Host(RpcEvent {
        kind: "context_budget".to_owned(),
        payload: json!({
            "type": "context_budget",
            "snapshot": snapshot,
            "policyWarning": "Invalid context policy; defaults are active."
        }),
    }));

    assert_eq!(app.state().context.actual_percent, Some(47.0));
    assert!(!app.state().plan_mode_active);
    assert!(app.state().approval.is_some());
    assert!(matches!(
        app.state().transcript.last(),
        Some(TranscriptItem::Notice(message)) if message.contains("defaults")
    ));
}

#[test]
fn new_resume_and_tree_are_local_commands_without_user_transcript_items() {
    for (command, expected) in [
        ("/new", AppEffect::NewSession),
        ("/resume", AppEffect::OpenSessionBrowser),
        (
            "/tree",
            AppEffect::GetTreeState {
                filter_mode: TreeFilterMode::Default,
                query: String::new(),
                folded_entry_ids: Vec::new(),
                generation: 0,
            },
        ),
    ] {
        let mut app = App::new(state());
        app.state.editor.insert_text(command);

        assert_eq!(app.update(press(KeyCode::Enter)), vec![expected]);
        assert!(
            !app.state
                .transcript
                .iter()
                .any(|item| matches!(item, TranscriptItem::User(_))),
            "{command} must not enter the transcript"
        );
    }
}

#[test]
fn session_browser_switches_scope_and_confirms_a_missing_working_directory() {
    let mut app = App::new(state());
    app.state.editor.insert_text("/resume");
    app.update(press(KeyCode::Enter));
    app.update(AppEvent::Command(CommandEvent::SessionBrowserOpened(Ok(
        Box::new(SessionBrowserSnapshot {
            browser_id: "browser-1".to_owned(),
            current_cwd: "/workspace/current".to_owned(),
            scope: SessionScope::Current,
            query: String::new(),
            sort_mode: SessionSortMode::Threaded,
            named_only: false,
            sessions: vec![
                session_summary("/sessions/current.jsonl", "current", true, true),
                session_summary("/sessions/old.jsonl", "old", false, false),
            ],
            total: 2,
            offset: 0,
            next_offset: None,
            truncated: false,
        }),
    ))));

    app.update(press(KeyCode::Tab));
    assert_eq!(app.state.session_browser.as_ref().unwrap().selected, 1);
    app.update(press_with(KeyCode::Tab, KeyModifiers::SHIFT));
    assert_eq!(app.state.session_browser.as_ref().unwrap().selected, 0);
    assert_eq!(
        app.update(press(KeyCode::Char('w'))),
        vec![AppEffect::QuerySessionBrowser {
            browser_id: "browser-1".to_owned(),
            scope: SessionScope::All,
            query: String::new(),
            sort_mode: SessionSortMode::Threaded,
            named_only: false,
            offset: 0,
            generation: 1,
        }]
    );
    app.update(press(KeyCode::Down));
    assert!(app.update(press(KeyCode::Enter)).is_empty());
    assert!(
        app.state
            .session_browser
            .as_ref()
            .is_some_and(|browser| browser.confirm_missing_cwd.is_some())
    );
    assert_eq!(
        app.update(press(KeyCode::Char('y'))),
        vec![AppEffect::ResumeSession {
            session_path: "/sessions/old.jsonl".to_owned(),
            cwd_override: Some("/workspace/current".to_owned()),
        }]
    );
    assert_eq!(app.state.run_state, RunState::SwitchingSession);
}

#[test]
fn session_browser_fetches_and_appends_the_next_page_at_the_end() {
    let mut app = App::new(state());
    app.state.session_browser = Some(SessionBrowserState::loading());
    app.update(AppEvent::Command(CommandEvent::SessionBrowserOpened(Ok(
        Box::new(SessionBrowserSnapshot {
            browser_id: "browser-1".to_owned(),
            current_cwd: "/workspace/current".to_owned(),
            scope: SessionScope::Current,
            query: String::new(),
            sort_mode: SessionSortMode::Recent,
            named_only: false,
            sessions: vec![
                session_summary("/sessions/0.jsonl", "session-0", false, true),
                session_summary("/sessions/1.jsonl", "session-1", false, true),
            ],
            total: 3,
            offset: 0,
            next_offset: Some(2),
            truncated: true,
        }),
    ))));
    app.update(press(KeyCode::End));

    assert_eq!(
        app.update(press(KeyCode::Down)),
        vec![AppEffect::QuerySessionBrowser {
            browser_id: "browser-1".to_owned(),
            scope: SessionScope::Current,
            query: String::new(),
            sort_mode: SessionSortMode::Recent,
            named_only: false,
            offset: 2,
            generation: 1,
        }]
    );

    app.update(AppEvent::Command(
        CommandEvent::SessionBrowserQueryFinished {
            generation: 1,
            result: Ok(Box::new(SessionBrowserSnapshot {
                browser_id: "browser-1".to_owned(),
                current_cwd: "/workspace/current".to_owned(),
                scope: SessionScope::Current,
                query: String::new(),
                sort_mode: SessionSortMode::Recent,
                named_only: false,
                sessions: vec![session_summary(
                    "/sessions/2.jsonl",
                    "session-2",
                    false,
                    true,
                )],
                total: 3,
                offset: 2,
                next_offset: None,
                truncated: false,
            })),
        },
    ));

    let browser = app.state.session_browser.as_ref().expect("browser");
    assert_eq!(browser.sessions.len(), 3);
    assert_eq!(browser.selected, 2);
    assert_eq!(browser.next_offset, None);
}

#[test]
fn resume_and_tree_paging_use_the_configured_page_size() {
    let mut resume = App::new(state());
    resume.set_selection_page_size(24);
    resume.state.session_browser = Some(SessionBrowserState::loading());
    resume.update(AppEvent::Command(CommandEvent::SessionBrowserOpened(Ok(
        Box::new(SessionBrowserSnapshot {
            browser_id: "browser-1".to_owned(),
            current_cwd: "/workspace/current".to_owned(),
            scope: SessionScope::Current,
            query: String::new(),
            sort_mode: SessionSortMode::Recent,
            named_only: false,
            sessions: (0..30)
                .map(|index| {
                    session_summary(
                        &format!("/sessions/{index}.jsonl"),
                        &format!("session-{index}"),
                        false,
                        true,
                    )
                })
                .collect(),
            total: 30,
            offset: 0,
            next_offset: None,
            truncated: false,
        }),
    ))));
    resume.update(press(KeyCode::PageDown));
    assert_eq!(
        resume
            .state
            .session_browser
            .as_ref()
            .map(|browser| browser.selected),
        Some(24)
    );
    resume.set_selection_page_size(12);
    resume.update(press(KeyCode::Home));
    resume.update(press(KeyCode::PageDown));
    assert_eq!(
        resume
            .state
            .session_browser
            .as_ref()
            .map(|browser| browser.selected),
        Some(12)
    );

    let mut tree = App::new(state());
    tree.set_selection_page_size(24);
    tree.state.tree_browser = Some(TreeBrowserState::loading());
    tree.update(AppEvent::Command(CommandEvent::TreeStateFinished {
        generation: 0,
        result: Ok(Box::new(TreeSnapshot {
            items: (0..30)
                .map(|index| tree_item(&format!("entry-{index}"), None, index == 29, false))
                .collect(),
            leaf_id: Some("entry-29".to_owned()),
            filter_mode: TreeFilterMode::Default,
            query: String::new(),
        })),
    }));
    tree.update(press(KeyCode::Home));
    tree.update(press(KeyCode::PageDown));
    tree.update(press(KeyCode::Tab));
    assert_eq!(tree.state.tree_browser.as_ref().unwrap().selected, 25);
    tree.update(press_with(KeyCode::Char('p'), KeyModifiers::CONTROL));
    let browser = tree.state.tree_browser.as_ref().expect("tree browser");
    assert_eq!(browser.selected, 24);
    assert_eq!(browser.selected_entry_id.as_deref(), Some("entry-24"));
}

#[test]
fn session_activation_preserves_scrollback_and_replays_the_active_branch() {
    let mut app = App::new(state());
    app.state
        .transcript
        .push(TranscriptItem::Notice("existing scrollback".to_owned()));

    app.update(AppEvent::Command(CommandEvent::NewSessionFinished(Ok(
        Box::new(SessionCommandData {
            cancelled: false,
            activation: Some(activation("session-restored")),
        }),
    ))));

    assert_eq!(app.state.session.session_id, "session-restored");
    assert_eq!(app.state.run_state, RunState::Idle);
    assert_eq!(
        app.state.context.usage_state,
        ContextUsageState::Recalculating
    );
    assert_eq!(app.state.plan.as_ref().map(|plan| plan.revision), Some(2));
    assert!(matches!(
        app.state.transcript.first(),
        Some(TranscriptItem::Notice(text)) if text == "existing scrollback"
    ));
    assert!(app.state.transcript.iter().any(|item| matches!(
        item,
        TranscriptItem::SessionBoundary { action, label, cwd }
            if action == "new session"
                && label == "Restored work"
                && cwd == "/workspace/restored"
    )));
    assert!(app.state.transcript.iter().any(|item| matches!(
        item,
        TranscriptItem::User(UserMessage { text, status: UserMessageStatus::Accepted })
            if text == "restored question"
    )));
    assert!(app.state.transcript.iter().any(|item| matches!(
        item,
        TranscriptItem::Tool(ToolExecution {
            id,
            output,
            diff: Some(diff),
            status: ToolStatus::Succeeded,
            ..
        }) if id == "tool-restored"
            && output == "restored source"
            && diff.files[0].path == "src/lib.rs"
    )));
    assert!(app.state.transcript.iter().any(|item| matches!(
        item,
        TranscriptItem::Compaction(record)
            if record.reason == "restored" && record.tokens_before == 82_000
    )));
    assert!(app.state.transcript.iter().any(|item| matches!(
        item,
        TranscriptItem::BranchSummary(summary) if summary == "restored branch summary"
    )));
}

#[test]
fn tree_browser_supports_pi_filters_copy_summary_and_abort_flow() {
    let mut app = App::new(state());
    app.state.editor.insert_text("/tree");
    app.update(press(KeyCode::Enter));
    app.update(AppEvent::Command(CommandEvent::TreeStateFinished {
        generation: 0,
        result: Ok(Box::new(TreeSnapshot {
            items: vec![
                tree_item("branch", None, false, true),
                tree_item("leaf", Some("branch"), true, false),
            ],
            leaf_id: Some("leaf".to_owned()),
            filter_mode: TreeFilterMode::Default,
            query: String::new(),
        })),
    }));

    assert_eq!(
        app.update(press_with(KeyCode::Char('t'), KeyModifiers::CONTROL)),
        vec![AppEffect::GetTreeState {
            filter_mode: TreeFilterMode::NoTools,
            query: String::new(),
            folded_entry_ids: Vec::new(),
            generation: 1,
        }]
    );
    assert_eq!(
        app.update(press_with(KeyCode::Char('t'), KeyModifiers::CONTROL)),
        vec![AppEffect::GetTreeState {
            filter_mode: TreeFilterMode::Default,
            query: String::new(),
            folded_entry_ids: Vec::new(),
            generation: 2,
        }]
    );

    app.update(press(KeyCode::Up));
    assert_eq!(
        app.update(press_with(KeyCode::Char('x'), KeyModifiers::CONTROL)),
        vec![AppEffect::CopyTreeEntry {
            entry_id: "branch".to_owned(),
        }]
    );
    assert!(app.update(press(KeyCode::Enter)).is_empty());
    app.update(press(KeyCode::Char('2')));
    assert_eq!(
        app.update(press(KeyCode::Enter)),
        vec![AppEffect::NavigateTree {
            entry_id: "branch".to_owned(),
            summarize: true,
            custom_instructions: None,
        }]
    );
    assert_eq!(app.state.run_state, RunState::SummarizingBranch);
    assert_eq!(
        app.update(press(KeyCode::Esc)),
        vec![AppEffect::AbortTreeNavigation]
    );
    assert!(matches!(
        app.state
            .tree_browser
            .as_ref()
            .map(|browser| &browser.phase),
        Some(TreePhase::Navigating {
            summarizing: true,
            aborting: true,
            ..
        })
    ));
}

#[test]
fn successful_tree_navigation_restores_editor_and_appends_a_boundary() {
    let mut app = App::new(state());
    app.state
        .transcript
        .push(TranscriptItem::Notice("old scrollback".to_owned()));
    app.state.tree_browser = Some(TreeBrowserState::loading());
    app.state.run_state = RunState::NavigatingTree;

    app.update(AppEvent::Command(CommandEvent::TreeNavigateFinished(Ok(
        Box::new(TreeNavigateData {
            cancelled: false,
            aborted: false,
            editor_text: Some("recovered draft".to_owned()),
            activation: Some(activation("session-tree")),
        }),
    ))));

    assert!(app.state.tree_browser.is_none());
    assert_eq!(app.state.editor.text(), "recovered draft");
    assert!(matches!(
        app.state.transcript.first(),
        Some(TranscriptItem::Notice(text)) if text == "old scrollback"
    ));
    assert!(app.state.transcript.iter().any(|item| matches!(
        item,
        TranscriptItem::SessionBoundary { action, .. } if action == "tree navigation"
    )));
}

#[test]
fn streaming_text_and_tool_lifecycle_update_transcript() {
    let mut app = App::new(state());
    app.update(AppEvent::Pi(RpcEvent {
        kind: "message_update".to_owned(),
        payload: json!({
            "type": "message_update",
            "assistantMessageEvent": {"type": "text_delta", "delta": "hello"}
        }),
    }));
    app.update(AppEvent::Pi(RpcEvent {
        kind: "tool_execution_start".to_owned(),
        payload: json!({
            "type": "tool_execution_start",
            "toolCallId": "tool-1",
            "toolName": "read"
        }),
    }));
    app.update(AppEvent::Pi(RpcEvent {
        kind: "tool_execution_end".to_owned(),
        payload: json!({
            "type": "tool_execution_end",
            "toolCallId": "tool-1",
            "isError": false
        }),
    }));

    assert!(matches!(
        &app.state().transcript[0],
        TranscriptItem::Assistant(message) if message.text == "hello"
    ));
    assert!(matches!(
        &app.state().transcript[1],
        TranscriptItem::Tool(tool) if tool.status == ToolStatus::Succeeded
    ));
}

#[test]
fn transcript_viewer_is_local_preserves_editor_and_supports_modes_and_folding() {
    let mut app = App::new(state());
    app.state.editor.insert_text("unfinished draft");
    app.state
        .transcript
        .push(TranscriptItem::Tool(ToolExecution {
            id: "tool-1".to_owned(),
            name: "read".to_owned(),
            args: json!({"path": "src/lib.rs"}),
            output: "line one\nline two".to_owned(),
            diff: None,
            status: ToolStatus::Succeeded,
        }));
    let transcript_len = app.state.transcript.len();

    assert!(
        app.update(press_with(KeyCode::Char('o'), KeyModifiers::CONTROL))
            .is_empty()
    );
    assert_eq!(app.state.active_modal_kind(), Some(UiModalKind::Transcript));
    assert_eq!(app.state.transcript.len(), transcript_len);

    app.update(press(KeyCode::Char('2')));
    assert_eq!(
        app.state
            .transcript_viewer
            .as_ref()
            .map(|viewer| viewer.mode),
        Some(TranscriptViewMode::Verbose)
    );
    app.update(press(KeyCode::Enter));
    assert_eq!(
        app.state
            .transcript_viewer
            .as_ref()
            .and_then(|viewer| viewer.tool_expansion_overrides.get("tool-1"))
            .copied(),
        Some(false)
    );

    app.update(press(KeyCode::Esc));
    assert!(app.state.transcript_viewer.is_none());
    assert_eq!(app.state.transcript_view_mode, TranscriptViewMode::Verbose);
    assert_eq!(app.state.editor.text(), "unfinished draft");
    assert_eq!(app.state.transcript.len(), transcript_len);
}

#[test]
fn transcript_viewer_tool_navigation_supports_tab_shift_tab_and_ctrl_np() {
    let mut app = App::new(state());
    for id in ["tool-1", "tool-2"] {
        app.state
            .transcript
            .push(TranscriptItem::Tool(ToolExecution {
                id: id.to_owned(),
                name: "read".to_owned(),
                args: json!({"path": id}),
                output: String::new(),
                diff: None,
                status: ToolStatus::Succeeded,
            }));
    }
    app.update(press_with(KeyCode::Char('o'), KeyModifiers::CONTROL));
    assert_eq!(
        app.state
            .transcript_viewer
            .as_ref()
            .and_then(|viewer| viewer.selected_item),
        Some(1)
    );

    app.update(press_with(KeyCode::Char('p'), KeyModifiers::CONTROL));
    assert_eq!(
        app.state
            .transcript_viewer
            .as_ref()
            .and_then(|viewer| viewer.selected_item),
        Some(0)
    );
    app.update(press_with(KeyCode::Char('n'), KeyModifiers::CONTROL));
    assert_eq!(
        app.state
            .transcript_viewer
            .as_ref()
            .and_then(|viewer| viewer.selected_item),
        Some(1)
    );
    app.update(press(KeyCode::BackTab));
    assert_eq!(
        app.state
            .transcript_viewer
            .as_ref()
            .and_then(|viewer| viewer.selected_item),
        Some(0)
    );
    app.update(press(KeyCode::Tab));
    assert_eq!(
        app.state
            .transcript_viewer
            .as_ref()
            .and_then(|viewer| viewer.selected_item),
        Some(1)
    );
}

#[test]
fn approval_can_open_full_tool_details_and_escape_back_to_the_inline_panel() {
    let mut app = App::new(state());
    app.state
        .transcript
        .push(TranscriptItem::Tool(ToolExecution {
            id: "tool-1".to_owned(),
            name: "bash".to_owned(),
            args: json!({"command": "cargo test"}),
            output: String::new(),
            diff: None,
            status: ToolStatus::WaitingApproval,
        }));
    app.state.approval = Some(ApprovalState {
        approval_id: "approval-1".to_owned(),
        tool_call_id: "tool-1".to_owned(),
        tool_name: "bash".to_owned(),
        input: json!({"command": "cargo test"}),
        agent_id: None,
        agent_profile: None,
        model: None,
        goal_id: None,
        reason: Some("run tests".to_owned()),
        risk: Some("normal".to_owned()),
        selected: 0,
        replying: false,
    });

    app.update(press_with(KeyCode::Char('o'), KeyModifiers::CONTROL));
    assert_eq!(
        app.state().active_modal_kind(),
        Some(UiModalKind::Transcript)
    );
    assert!(app.state().approval.is_some());
    app.update(press(KeyCode::Esc));
    assert_eq!(app.state().active_modal_kind(), Some(UiModalKind::Approval));
}

#[test]
fn transcript_search_navigates_matches_without_mutating_history() {
    let mut app = App::new(state());
    app.state.transcript.extend([
        TranscriptItem::Notice("alpha first".to_owned()),
        TranscriptItem::Notice("unrelated".to_owned()),
        TranscriptItem::Notice("alpha second".to_owned()),
    ]);
    app.update(press_with(KeyCode::Char('o'), KeyModifiers::CONTROL));
    app.update(press(KeyCode::Char('/')));
    for character in "alpha".chars() {
        app.update(press(KeyCode::Char(character)));
    }

    let viewer = app.state.transcript_viewer.as_ref().unwrap();
    assert_eq!(viewer.search_matches, vec![0, 2]);
    assert_eq!(viewer.selected_item, Some(0));

    app.update(press(KeyCode::Enter));
    app.update(press(KeyCode::Char('n')));
    assert_eq!(
        app.state
            .transcript_viewer
            .as_ref()
            .and_then(|viewer| viewer.selected_item),
        Some(2)
    );
    assert_eq!(app.state.transcript.len(), 3);
}

#[test]
fn alternate_search_focus_uses_unicode_editor_and_two_stage_escape() {
    let mut app = App::new(state());
    app.state
        .transcript
        .push(TranscriptItem::Notice("你好 result".to_owned()));
    app.update(press_with(KeyCode::Char('o'), KeyModifiers::CONTROL));
    app.update(press(KeyCode::Char('/')));
    app.update(press(KeyCode::Char('你')));
    app.update(press(KeyCode::Char('a')));
    app.update(press(KeyCode::Left));
    app.update(press(KeyCode::Char('好')));

    let viewer = app.state.transcript_viewer.as_ref().unwrap();
    assert!(viewer.search_active);
    assert_eq!(viewer.search_query.text(), "你好a");
    assert_eq!(viewer.search_query.cursor(), 2);

    app.update(press(KeyCode::Esc));
    let viewer = app.state.transcript_viewer.as_ref().unwrap();
    assert!(!viewer.search_active);
    assert!(viewer.search_query.text().is_empty());

    app.update(press(KeyCode::Esc));
    assert!(app.state.transcript_viewer.is_none());
}
