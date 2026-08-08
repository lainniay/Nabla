use crate::{
    command::COMMAND_MENU_VISIBLE_ROWS,
    host::ApprovalDecision,
    rpc::PiState,
    state::{
        AppState, ApprovalState, AssistantMessage, AuthFlowState, AuthPromptKind, AuthPromptState,
        AuthState, EditorState, GrantProposal, RunState, SessionBrowserState, ToolExecution,
        ToolStatus, TranscriptItem, TranscriptViewMode, TranscriptViewerState, TreeBrowserState,
        TreeItem, TreePhase, UserMessage, UserMessageStatus,
    },
    ui::{
        SurfaceManager, palette,
        scene::{
            composer::alternate_input_model,
            panels::{
                modals::tree::{tree_choice_rows, tree_identity_color, tree_prefix},
                panel_choice_row, primary_panel_request,
            },
        },
        store::UiStore,
        text::display_width,
        types::{Color, HitTarget, Rect, SurfaceKind, VisualRow},
    },
};
use serde_json::json;

use super::*;

fn view(domain: &AppState) -> SceneViewModel<'_> {
    SceneViewModel::from_domain(domain)
}

fn state() -> AppState {
    AppState::new(PiState {
        model: Some(json!({"provider": "test", "id": "model"})),
        thinking_level: "off".to_owned(),
        is_streaming: false,
        is_compacting: false,
        steering_mode: "one-at-a-time".to_owned(),
        follow_up_mode: "one-at-a-time".to_owned(),
        session_file: None,
        session_id: "session".to_owned(),
        session_name: None,
        auto_compaction_enabled: true,
        message_count: 0,
        pending_message_count: 0,
    })
}

#[test]
fn frame_rows_layout_bounds_and_cursor_share_one_revision() {
    let mut domain = state();
    domain.transcript = vec![
        TranscriptItem::User(UserMessage {
            text: "hello".to_owned(),
            status: UserMessageStatus::Accepted,
        }),
        TranscriptItem::Assistant(AssistantMessage {
            text: "streaming".to_owned(),
            complete: false,
            ..AssistantMessage::default()
        }),
    ];
    domain.editor.insert_text("你👩🏽‍💻");
    let mut store = UiStore::new(super::super::types::TerminalSize::new(20, 8));
    store.synchronize(&domain);
    let frame = SceneBuilder.build(&view(&domain), store.state(), SurfaceKind::Primary);

    assert_eq!(frame.revision, store.state().revision);
    assert_eq!(frame.rows.len(), 8);
    assert_eq!(frame.main_layout.status.y, 7);
    assert!(
        frame
            .component_bounds
            .keys()
            .any(|id| id.starts_with("assistant:0:2:text:segment:"))
    );
    assert!(
        frame
            .cursor
            .is_some_and(|cursor| cursor.row < 8 && cursor.column < 20)
    );
}

#[test]
fn stable_history_remains_resident_until_it_leaves_the_fixed_window() {
    let mut domain = state();
    domain.transcript = vec![
        TranscriptItem::Notice("sealed".to_owned()),
        TranscriptItem::Assistant(AssistantMessage {
            text: "live".to_owned(),
            complete: false,
            ..AssistantMessage::default()
        }),
    ];
    let mut store = UiStore::new(super::super::types::TerminalSize::new(30, 8));
    store.synchronize(&domain);
    let frame = SceneBuilder.build(&view(&domain), store.state(), SurfaceKind::Primary);
    let visible = frame
        .rows
        .iter()
        .map(VisualRow::plain_text)
        .collect::<String>();
    assert!(visible.contains("sealed"));
    assert!(visible.contains("live"));
}

#[test]
fn empty_primary_surface_owns_the_full_screen_with_fixed_history_geometry() {
    let domain = state();
    let size = super::super::types::TerminalSize::new(40, 12);
    let mut store = UiStore::new(size);
    store.synchronize(&domain);

    let frame = SceneBuilder.build(&view(&domain), store.state(), SurfaceKind::Primary);

    assert_eq!(frame.viewport, Rect::new(0, 0, size.width, size.height));
    assert_eq!(frame.main_layout.owned_surface, frame.viewport);
    assert_eq!(
        frame.main_layout.history_window,
        frame.main_layout.transcript
    );
    assert_eq!(frame.main_layout.history_window.y, 0);
    assert_eq!(
        frame.main_layout.history_window.bottom().saturating_add(1),
        frame.main_layout.composer.y
    );
    assert!(
        frame.rows[..usize::from(frame.main_layout.history_window.bottom())]
            .iter()
            .all(|row| row.plain_text().is_empty())
    );
}

#[test]
fn bootstrap_blank_rows_move_up_as_transcript_grows() {
    let mut domain = state();
    let size = super::super::types::TerminalSize::new(40, 12);
    let mut store = UiStore::new(size);
    store.synchronize(&domain);
    let empty = SceneBuilder.build(&view(&domain), store.state(), SurfaceKind::Primary);

    domain
        .transcript
        .push(TranscriptItem::Assistant(AssistantMessage {
            text: "first\n\nsecond\n\nthird".to_owned(),
            complete: false,
            ..AssistantMessage::default()
        }));
    store.synchronize(&domain);
    let grown = SceneBuilder.build(&view(&domain), store.state(), SurfaceKind::Primary);

    assert_eq!(empty.viewport, Rect::new(0, 0, size.width, size.height));
    assert_eq!(grown.viewport, empty.viewport);
    let first_content = grown
        .rows
        .iter()
        .position(|row| !row.plain_text().is_empty())
        .expect("resident transcript content");
    assert!(first_content > 0);
    assert!(
        grown.rows[..first_content]
            .iter()
            .all(|row| row.plain_text().is_empty())
    );
}

#[test]
fn claimed_primary_surface_never_exposes_shell_rows() {
    let domain = state();
    let size = super::super::types::TerminalSize::new(40, 12);
    let mut store = UiStore::new(size);
    store.synchronize(&domain);
    let frame = SceneBuilder.build(&view(&domain), store.state(), SurfaceKind::Primary);

    assert_eq!(frame.viewport, Rect::new(0, 0, size.width, size.height));
    assert_eq!(frame.main_layout.transcript.y, 0);
    assert_eq!(frame.main_layout.status.bottom(), size.height);
}

#[test]
fn message_completion_preserves_visible_row_positions() {
    let mut domain = state();
    domain
        .transcript
        .push(TranscriptItem::Assistant(AssistantMessage {
            id: 1,
            text: "```text\none\ntwo\nthree\nfour\nfive".to_owned(),
            complete: false,
            ..AssistantMessage::default()
        }));
    let size = super::super::types::TerminalSize::new(48, 14);
    let mut store = UiStore::new(size);
    store.synchronize(&domain);
    let streaming = SceneBuilder.build(&view(&domain), store.state(), SurfaceKind::Primary);
    let row_before = streaming
        .rows
        .iter()
        .position(|row| row.plain_text().contains("three"))
        .expect("streaming row");

    let TranscriptItem::Assistant(message) = &mut domain.transcript[0] else {
        unreachable!()
    };
    message.complete = true;
    store.synchronize(&domain);
    let projection = store.state().transcript.project_primary(
        size.width,
        usize::from(streaming.main_layout.history_window.height),
        store.state().revision,
        100,
        usize::MAX,
        store.state().animation_frame,
    );
    let completed = SceneBuilder.build_with_projection(
        &view(&domain),
        store.state(),
        SurfaceKind::Primary,
        &projection,
    );
    let row_after = completed
        .rows
        .iter()
        .position(|row| row.plain_text().contains("three"))
        .expect("completed row remains resident");

    assert_eq!(row_after, row_before);
}

#[test]
fn completed_assistant_does_not_leave_screen_height_gap() {
    let mut domain = state();
    domain.transcript = vec![
        TranscriptItem::Assistant(AssistantMessage {
            id: 7,
            text: "```text\none\ntwo\nthree\nfour\nfive".to_owned(),
            complete: true,
            ..AssistantMessage::default()
        }),
        TranscriptItem::TurnSeparator(crate::state::TurnSeparator {
            turn_id: "turn-gap".to_owned(),
            started_at: "start".to_owned(),
            ended_at: "end".to_owned(),
            duration_ms: 1_000,
            estimated: false,
        }),
    ];
    let size = super::super::types::TerminalSize::new(48, 14);
    let mut store = UiStore::new(size);
    store.synchronize(&domain);
    let frame = SceneBuilder.build(&view(&domain), store.state(), SurfaceKind::Primary);
    let last_history_row = frame.rows[..usize::from(frame.main_layout.composer.y)]
        .iter()
        .rposition(|row| !row.plain_text().is_empty())
        .expect("completed transcript remains visible");
    assert_eq!(
        last_history_row.saturating_add(2),
        usize::from(frame.main_layout.composer.y)
    );
}

#[test]
fn turn_separator_remains_adjacent_to_visible_history() {
    let mut domain = state();
    domain.transcript = vec![
        TranscriptItem::Assistant(AssistantMessage {
            id: 1,
            text: "```text\nvisible tail".to_owned(),
            complete: true,
            ..AssistantMessage::default()
        }),
        TranscriptItem::TurnSeparator(crate::state::TurnSeparator {
            turn_id: "turn-adjacent".to_owned(),
            started_at: "start".to_owned(),
            ended_at: "end".to_owned(),
            duration_ms: 1_000,
            estimated: false,
        }),
    ];
    let size = super::super::types::TerminalSize::new(48, 14);
    let mut store = UiStore::new(size);
    store.synchronize(&domain);
    let frame = SceneBuilder.build(&view(&domain), store.state(), SurfaceKind::Primary);
    let visible = frame
        .rows
        .iter()
        .map(VisualRow::plain_text)
        .collect::<Vec<_>>();
    let tail = visible
        .iter()
        .position(|row| row.contains("visible tail"))
        .expect("visible assistant tail");
    let separator = visible
        .iter()
        .position(|row| row.contains("Worked for"))
        .expect("visible turn separator");
    assert!(separator > tail && separator.saturating_sub(tail) <= 2);
}

#[test]
fn opening_and_closing_panel_restores_owned_primary_rows() {
    let mut domain = state();
    domain
        .transcript
        .push(TranscriptItem::Assistant(AssistantMessage {
            text: "owned transcript row".to_owned(),
            complete: false,
            ..AssistantMessage::default()
        }));
    let size = super::super::types::TerminalSize::new(48, 14);
    let mut store = UiStore::new(size);
    store.synchronize(&domain);
    let baseline = SceneBuilder.build(&view(&domain), store.state(), SurfaceKind::Primary);

    domain.editor.insert_text("/");
    store.synchronize(&domain);
    let opened = SceneBuilder.build(&view(&domain), store.state(), SurfaceKind::Primary);
    assert!(opened.panel.is_some());
    domain.editor.clear();
    store.synchronize(&domain);
    let restored = SceneBuilder.build(&view(&domain), store.state(), SurfaceKind::Primary);

    assert_eq!(baseline.viewport, Rect::new(0, 0, size.width, size.height));
    assert_eq!(restored.viewport, baseline.viewport);
    assert_eq!(
        restored
            .rows
            .iter()
            .map(VisualRow::plain_text)
            .collect::<Vec<_>>(),
        baseline
            .rows
            .iter()
            .map(VisualRow::plain_text)
            .collect::<Vec<_>>()
    );
}

#[test]
fn active_transcript_keeps_one_blank_row_above_the_composer() {
    let mut domain = state();
    domain.transcript.push(TranscriptItem::User(UserMessage {
        text: "hello from the bottom".to_owned(),
        status: UserMessageStatus::Pending,
    }));
    let size = super::super::types::TerminalSize::new(40, 12);
    let mut store = UiStore::new(size);
    store.synchronize(&domain);

    let frame = SceneBuilder.build(&view(&domain), store.state(), SurfaceKind::Primary);
    let transcript = frame
        .component_bounds
        .get("transcript:0")
        .expect("active transcript bounds");

    assert_eq!(
        transcript.end.saturating_add(1),
        usize::from(frame.main_layout.composer.y)
    );
    assert!(frame.rows[transcript.end].plain_text().is_empty());
    assert_eq!(frame.viewport.y, 0);
    assert_eq!(frame.main_layout.history_window.y, 0);
    assert!(
        frame.rows[transcript.start..transcript.end]
            .iter()
            .map(VisualRow::plain_text)
            .collect::<Vec<_>>()
            .join("\n")
            .contains("hello from the bottom")
    );
    assert!(
        frame.rows[transcript.start..transcript.end]
            .iter()
            .any(|row| row.plain_text().starts_with('╭'))
    );
}

#[test]
fn resident_turn_separator_keeps_an_owned_blank_row_above_the_composer() {
    let mut domain = state();
    domain
        .transcript
        .push(TranscriptItem::TurnSeparator(crate::state::TurnSeparator {
            turn_id: "turn-1".to_owned(),
            started_at: "start".to_owned(),
            ended_at: "end".to_owned(),
            duration_ms: 1_000,
            estimated: false,
        }));
    let size = super::super::types::TerminalSize::new(48, 10);
    let mut store = UiStore::new(size);
    store.synchronize(&domain);

    let frame = SceneBuilder.build(&view(&domain), store.state(), SurfaceKind::Primary);
    let gap = frame.main_layout.composer.y.saturating_sub(1);
    assert_eq!(frame.viewport.y, 0);
    assert_eq!(frame.main_layout.history_window.y, 0);
    assert!(frame.rows[usize::from(gap)].plain_text().is_empty());
    assert!(
        frame.rows[..usize::from(gap)]
            .iter()
            .any(|row| row.plain_text().contains("Worked for"))
    );
    assert!(
        frame.rows[usize::from(frame.main_layout.composer.y)]
            .plain_text()
            .starts_with('╭')
    );
}

#[test]
fn animation_changes_only_the_live_frame_and_never_history_or_domain_state() {
    let mut domain = state();
    domain.transcript.push(TranscriptItem::Tool(ToolExecution {
        id: "running-tool".to_owned(),
        name: "bash".to_owned(),
        args: json!({"command": "cargo test"}),
        output: String::new(),
        diff: None,
        status: ToolStatus::Running,
    }));
    let original_transcript = domain.transcript.clone();
    let size = super::super::types::TerminalSize::new(48, 10);
    let mut store = UiStore::new(size);
    store.synchronize(&domain);

    assert!(animation_active(&view(&domain)));
    let first = SceneBuilder.build(&view(&domain), store.state(), SurfaceKind::Primary);
    store.state_mut().animation_frame = 1;
    let second = SceneBuilder.build(&view(&domain), store.state(), SurfaceKind::Primary);
    let first_text = first
        .rows
        .iter()
        .map(VisualRow::plain_text)
        .collect::<Vec<_>>()
        .join("\n");
    let second_text = second
        .rows
        .iter()
        .map(VisualRow::plain_text)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(first_text.contains("⠋ Ran"));
    assert!(second_text.contains("⠙ Ran"));
    assert_eq!(domain.transcript, original_transcript);
    assert!(
        store
            .state()
            .transcript
            .project_primary(size.width, 100, 1, 100, usize::MAX, 0)
            .overflow_blocks
            .is_empty()
    );
}

#[test]
fn panel_open_and_close_restores_the_occluded_transcript_exactly() {
    let mut domain = state();
    domain
        .transcript
        .push(TranscriptItem::Assistant(AssistantMessage {
            text: "persistent live row".to_owned(),
            complete: false,
            ..AssistantMessage::default()
        }));
    let mut store = UiStore::new(super::super::types::TerminalSize::new(80, 24));
    store.synchronize(&domain);
    let baseline = SceneBuilder.build(&view(&domain), store.state(), SurfaceKind::Primary);

    domain.editor.insert_text("/");
    store.reduce(super::super::store::UiEvent::DomainChanged);
    let opened = SceneBuilder.build(&view(&domain), store.state(), SurfaceKind::Primary);
    assert!(opened.main_layout.panel.is_some());
    assert_eq!(
        opened.main_layout.transcript,
        baseline.main_layout.transcript
    );
    assert_eq!(opened.main_layout.composer, baseline.main_layout.composer);
    assert_eq!(opened.main_layout.status, baseline.main_layout.status);
    assert_eq!(opened.viewport, baseline.viewport);
    let panel = opened.panel.as_ref().expect("floating panel");
    assert_eq!(panel.area.x, 0);
    assert_eq!(panel.area.width, opened.terminal_size.width);
    assert_eq!(
        panel.area.height,
        u16::try_from(COMMAND_MENU_VISIBLE_ROWS + 2).unwrap()
    );
    assert_eq!(panel.rows.len(), usize::from(panel.area.height));
    assert!(
        opened
            .hit_regions
            .iter()
            .all(|region| !matches!(region.target, HitTarget::Panel | HitTarget::Command(_)))
    );

    domain.editor.clear();
    store.reduce(super::super::store::UiEvent::DomainChanged);
    let restored = SceneBuilder.build(&view(&domain), store.state(), SurfaceKind::Primary);
    assert!(restored.main_layout.panel.is_none());
    assert!(restored.panel.is_none());
    assert_eq!(
        baseline
            .rows
            .iter()
            .map(VisualRow::plain_text)
            .collect::<Vec<_>>(),
        restored
            .rows
            .iter()
            .map(VisualRow::plain_text)
            .collect::<Vec<_>>()
    );
}

#[test]
fn approval_panel_only_renders_actions_valid_for_the_request() {
    let mut domain = state();
    domain.approval = Some(ApprovalState {
        approval_id: "approval".to_owned(),
        tool_call_id: "tool".to_owned(),
        tool_name: "bash".to_owned(),
        input: json!({"command": "cargo test"}),
        agent_id: None,
        agent_profile: None,
        model: None,
        reason: Some("run tests".to_owned()),
        risk: Some("normal".to_owned()),
        summary: "run tests".to_owned(),
        selected: 0,
        replying: false,
        available_decisions: vec![
            ApprovalDecision::AllowOnce,
            ApprovalDecision::AllowSession,
            ApprovalDecision::AllowWorkspace,
            ApprovalDecision::Deny,
        ],
        session_grant: Some(GrantProposal {
            scope: "session".to_owned(),
            workspace_id: "workspace-1".to_owned(),
            session_id: Some("session-1".to_owned()),
            matchers: vec![json!({
                "kind": "exec",
                "executable": "cargo",
                "argv": ["test"],
                "cwd": "/workspace",
                "environment": {}
            })],
            invalidation_keys: Vec::new(),
        }),
        workspace_grant: Some(GrantProposal {
            scope: "workspace".to_owned(),
            workspace_id: "workspace-1".to_owned(),
            session_id: None,
            matchers: vec![json!({
                "kind": "exec",
                "executable": "cargo",
                "argv": ["test"],
                "cwd": "/workspace",
                "environment": {}
            })],
            invalidation_keys: vec![json!({
                "kind": "file_digest",
                "path": "/workspace/Cargo.toml",
                "value": "manifest-digest"
            })],
        }),
        ..ApprovalState::default()
    });
    let size = super::super::types::TerminalSize::new(120, 16);
    let mut store = UiStore::new(size);
    store.synchronize(&domain);

    let frame = SceneBuilder.build(&view(&domain), store.state(), SurfaceKind::Primary);
    let panel = frame.panel.expect("approval panel");
    let text = panel
        .rows
        .iter()
        .map(VisualRow::plain_text)
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(panel.area.width, size.width);
    assert!(panel.area.height >= 4);
    assert!(text.contains("Ask for Approval"));
    assert!(text.contains("run tests"));
    assert!(text.contains("• run tests"));
    assert!(text.contains("cargo test"));
    assert!(text.contains("  └ cargo test"));
    let command_row = panel
        .rows
        .iter()
        .find(|row| row.plain_text().contains("  └ cargo test"))
        .expect("command row");
    assert!(
        command_row
            .cells
            .iter()
            .any(|cell| cell.style.foreground == Color::Cyan && cell.style.bold)
    );
    assert!(command_row.cells.len() > 1);
    assert!(text.contains("Allow once"));
    assert!(text.contains("Allow for Session"));
    assert!(text.contains("Allow for Workspace"));
    assert!(!text.contains("Session saves"));
    assert!(!text.contains("Workspace saves"));
    assert!(!text.contains("file_digest"));
    assert!(!text.contains("[Y]"));
    assert!(!text.contains("[S]"));
    assert!(!text.contains("[A]"));
    assert!(!text.contains("[N]"));
    assert!(text.contains("Deny"));
    assert!(
        panel
            .rows
            .iter()
            .filter(|row| {
                let text = row.plain_text();
                text.contains("Allow once") || text.contains("Deny")
            })
            .all(|row| row.display_width() == size.width)
    );
    assert!(
        panel
            .rows
            .iter()
            .any(|row| row.plain_text().contains("Approve only this request"))
    );
    assert!(
        panel
            .rows
            .iter()
            .any(|row| row.plain_text().contains("Reject this tool request"))
    );
}

#[test]
fn high_risk_approval_uses_the_same_full_width_floating_panel() {
    let mut domain = state();
    domain.selection_page_size = 12;
    domain.approval = Some(ApprovalState {
        approval_id: "approval".to_owned(),
        tool_call_id: "tool".to_owned(),
        tool_name: "bash".to_owned(),
        input: json!({"command": "printenv SECRET_TOKEN"}),
        agent_id: None,
        agent_profile: None,
        model: None,
        reason: Some("inspect credentials".to_owned()),
        risk: Some("credential".to_owned()),
        selected: 1,
        replying: false,
        available_decisions: vec![ApprovalDecision::AllowOnce, ApprovalDecision::Deny],
        ..ApprovalState::default()
    });
    let size = super::super::types::TerminalSize::new(72, 16);
    let mut store = UiStore::new(size);
    store.synchronize(&domain);

    assert_eq!(SurfaceManager.route(&domain), SurfaceKind::Primary);
    let frame = SceneBuilder.build(&view(&domain), store.state(), SurfaceKind::Primary);
    let panel = frame.panel.expect("high-risk approval panel");
    let text = panel
        .rows
        .iter()
        .map(VisualRow::plain_text)
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(panel.area.width, size.width);
    assert_eq!(panel.rows.len(), 9);
    assert!(text.contains("Ask for Approval"));
    assert!(text.contains("May access sensitive credentials"));
    assert!(text.contains("printenv SECRET_TOKEN"));
    assert!(!text.contains("Credential risk"));
    assert!(!text.contains("inspect credentials"));
    assert!(!text.contains("Allow for Session"));
    assert!(!text.contains("Always allow matching"));
    assert!(
        panel
            .rows
            .iter()
            .filter(|row| {
                let text = row.plain_text();
                text.contains("Allow once") || text.contains("Deny")
            })
            .all(|row| row.display_width() == size.width)
    );
    assert!(
        panel
            .rows
            .iter()
            .any(|row| row.plain_text().contains("Approve only this request"))
    );
}

#[test]
fn approval_panel_normalizes_paths_truncates_details_and_keeps_actions_visible() {
    let mut domain = state();
    domain.selection_page_size = 8;
    domain.approval = Some(ApprovalState {
        approval_id: "approval".to_owned(),
        tool_call_id: "tool".to_owned(),
        tool_name: "edit_file".to_owned(),
        input: json!({
            "path": "src/./nested/../lib.rs",
            "replacement": {
                "deep": {"value": ["你好", "世界", {"more": true}]}
            }
        }),
        agent_id: Some("agent-1".to_owned()),
        agent_profile: Some("worker".to_owned()),
        model: Some("provider/model".to_owned()),
        reason: Some("Apply a requested change".to_owned()),
        risk: Some("outside_workspace".to_owned()),
        selected: 2,
        replying: false,
        available_decisions: vec![ApprovalDecision::AllowOnce, ApprovalDecision::Deny],
        ..ApprovalState::default()
    });

    let request = primary_panel_request(&view(&domain), 36).expect("approval panel request");
    assert!(request.height <= 10);
    let all_text = request
        .rows
        .iter()
        .map(VisualRow::plain_text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(all_text.contains("Ask for Approval"));
    assert!(all_text.contains("Outside trusted project scope"));
    assert!(all_text.contains("Edit src/lib.rs"));
    assert!(!all_text.contains("Workspace boundary"));
    assert!(!all_text.contains("Apply a requested change"));
    assert!(!all_text.contains("replacement"));
    assert!(all_text.contains("Deny"));
    assert!(!all_text.contains("[N]"));
    assert!(!all_text.contains("agent-1"));
    assert!(!all_text.contains("provider/model"));

    let selected = request.clone().render(Rect::new(0, 0, 36, 3));
    assert!(
        selected
            .rows
            .iter()
            .any(|row| row.plain_text().contains("Deny"))
    );
}

#[test]
fn narrow_panel_rows_preserve_action_and_shortcut_before_description() {
    let row = panel_choice_row(
        "approval",
        "Allow once",
        "Approve only this request",
        true,
        true,
        12,
    );
    assert_eq!(row.display_width(), 12);
    assert!(row.plain_text().contains("Allow once"));
    assert!(!row.plain_text().contains("Approve"));
}

#[test]
fn alternate_transcript_uses_modes_expansion_selection_and_full_output() {
    let mut domain = state();
    domain
        .transcript
        .push(TranscriptItem::Tool(crate::state::ToolExecution {
            id: "tool-1".to_owned(),
            name: "read_file".to_owned(),
            args: json!({"path": "src/lib.rs", "line": 7}),
            output: "FULL OUTPUT\nsecond line".to_owned(),
            diff: None,
            status: crate::state::ToolStatus::Succeeded,
        }));
    domain.transcript_viewer = Some(TranscriptViewerState::new(
        TranscriptViewMode::Normal,
        &domain.transcript,
    ));
    let size = super::super::types::TerminalSize::new(48, 20);
    let mut store = UiStore::new(size);
    store.synchronize(&domain);

    let normal = SceneBuilder.build(&view(&domain), store.state(), SurfaceKind::Alternate);
    let normal_text = normal
        .rows
        .iter()
        .map(VisualRow::plain_text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!normal_text.contains("FULL OUTPUT"));
    assert_eq!(normal.main_layout.composer.height, 3);
    assert!(
        normal.rows[usize::from(normal.main_layout.composer.y.saturating_sub(1))]
            .plain_text()
            .is_empty()
    );
    assert!(
        normal.rows[usize::from(normal.main_layout.composer.y)]
            .plain_text()
            .starts_with('╭')
    );

    domain.transcript_viewer.as_mut().unwrap().mode = TranscriptViewMode::Verbose;
    let verbose = SceneBuilder.build(&view(&domain), store.state(), SurfaceKind::Alternate);
    let verbose_text = verbose
        .rows
        .iter()
        .map(VisualRow::plain_text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(verbose_text.contains("Arguments"));
    assert!(verbose_text.contains("\"line\": 7"));
    assert!(verbose_text.contains("FULL OUTPUT"));
    assert!(
        verbose
            .rows
            .iter()
            .flat_map(|row| &row.cells)
            .any(|cell| { cell.style.background == palette::SURFACE_0 })
    );

    domain.transcript_viewer.as_mut().unwrap().mode = TranscriptViewMode::Summary;
    let summary = SceneBuilder.build(&view(&domain), store.state(), SurfaceKind::Alternate);
    let summary_text = summary
        .rows
        .iter()
        .map(VisualRow::plain_text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!summary_text.contains("Arguments"));
    assert!(!summary_text.contains("FULL OUTPUT"));
}

#[test]
fn common_panel_rows_right_align_descriptions() {
    let width = 48;
    let normal = panel_choice_row("panel", "Option", "Description", false, true, width);
    let selected = panel_choice_row("panel", "Selected", "Right aligned", true, true, width);

    for (row, description) in [(&normal, "Description"), (&selected, "Right aligned")] {
        assert_eq!(row.display_width(), width);
        assert!(row.plain_text().ends_with(description));
        assert!(!row.plain_text().contains('○'));
        assert!(!row.plain_text().contains('●'));
    }
    assert!(
        selected
            .cells
            .iter()
            .all(|cell| cell.style.background == Color::Default)
    );
    assert!(
        selected
            .cells
            .iter()
            .all(|cell| { cell.style.foreground == palette::LAVENDER && cell.style.bold })
    );
}

#[test]
fn alternate_search_box_tracks_editor_focus_cursor_and_escape_visual_state() {
    let mut domain = state();
    domain
        .transcript
        .push(TranscriptItem::Notice("你好 result".to_owned()));
    domain.transcript_viewer = Some(TranscriptViewerState::new(
        TranscriptViewMode::Normal,
        &domain.transcript,
    ));
    let viewer = domain.transcript_viewer.as_mut().unwrap();
    viewer.search_active = true;
    viewer.search_query.insert_text("你好");
    let size = super::super::types::TerminalSize::new(36, 12);
    let mut store = UiStore::new(size);
    store.synchronize(&domain);

    let frame = SceneBuilder.build(&view(&domain), store.state(), SurfaceKind::Alternate);
    let composer = frame.main_layout.composer;
    let text = frame.rows[usize::from(composer.y)..usize::from(composer.bottom())]
        .iter()
        .map(VisualRow::plain_text)
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(composer.height, 3);
    assert!(text.contains("你好"));
    assert!(frame.cursor.is_some());
    assert!(
        frame.rows[usize::from(composer.y)]
            .cells
            .iter()
            .all(|cell| cell.style.foreground == palette::INPUT_ACCENT)
    );
}

#[test]
fn alternate_input_projection_covers_session_tree_and_secret_auth_editors() {
    let mut domain = state();
    domain.session_browser = Some(SessionBrowserState::loading());
    let session = alternate_input_model(&view(&domain));
    assert_eq!(session.placeholder, "Search sessions");
    assert!(!session.focused);

    domain.session_browser = None;
    let mut tree = TreeBrowserState::loading();
    let mut label = EditorState::default();
    label.insert_text("branch label");
    tree.phase = TreePhase::EditLabel {
        entry_id: "entry".to_owned(),
        editor: label,
    };
    domain.tree_browser = Some(tree);
    let tree = alternate_input_model(&view(&domain));
    assert_eq!(tree.text, "branch label");
    assert!(tree.focused);

    domain.tree_browser = None;
    let mut secret = EditorState::default();
    secret.insert_text("密钥");
    domain.auth_state = AuthState::Running(Box::new(AuthFlowState {
        id: "flow".to_owned(),
        provider_name: "Provider".to_owned(),
        status: "Waiting".to_owned(),
        url: None,
        device_code: None,
        prompt: Some(AuthPromptState {
            id: "prompt".to_owned(),
            kind: AuthPromptKind::Secret,
            message: "Enter token".to_owned(),
            placeholder: None,
            options: Vec::new(),
            selected: 0,
            editor: secret,
        }),
    }));
    let auth = alternate_input_model(&view(&domain));
    assert!(auth.focused);
    assert!(auth.secret);
    assert_eq!(auth.display_text(), "••");
}

#[test]
fn deep_tree_prefix_caps_gutter_and_keeps_recent_connectors() {
    let item = TreeItem {
        entry_id: "entry".to_owned(),
        parent_id: Some("parent".to_owned()),
        kind: "message".to_owned(),
        role: Some("assistant".to_owned()),
        preview: "assistant: deep node".to_owned(),
        label: None,
        label_timestamp: None,
        visual_depth: 9,
        show_connector: true,
        gutter_positions: vec![0, 2, 5, 6, 7],
        is_last: true,
        is_active_path: true,
        is_leaf: false,
        foldable: true,
        folded: true,
    };
    let prefix = tree_prefix(&item);
    assert!(prefix.starts_with("… "));
    assert!(prefix.contains('│'));
    assert!(prefix.contains("└─"));
    assert!(prefix.ends_with("▸ "));
    assert!(display_width(&prefix) <= 10);

    let mut identity_item = item;
    identity_item.is_active_path = false;
    let rows = tree_choice_rows(&identity_item, false, 48);
    assert_eq!(rows.len(), 2);
    assert!(rows[0].plain_text().starts_with("• Assistant"));
    assert!(rows[1].plain_text().starts_with("  └ "));
    assert!(!rows[1].plain_text().contains("assistant:"));
    assert!(
        rows[1]
            .cells
            .iter()
            .any(|cell| { cell.symbol == "d" && cell.style.foreground == palette::TEXT })
    );
    assert!(
        rows[1]
            .cells
            .iter()
            .any(|cell| { cell.symbol == "…" && cell.style.foreground == palette::MAUVE })
    );

    let selected = tree_choice_rows(&identity_item, true, 48);
    assert!(
        selected
            .iter()
            .flat_map(|row| &row.cells)
            .all(|cell| cell.style.background == Color::Default)
    );
    assert!(
        selected
            .iter()
            .flat_map(|row| &row.cells)
            .all(|cell| cell.style.bold)
    );
    for width in 1..=12 {
        assert!(
            tree_choice_rows(&identity_item, false, width)
                .iter()
                .all(|row| row.display_width() <= width)
        );
    }

    identity_item.label = Some("checkpoint".to_owned());
    identity_item.preview = "[checkpoint] assistant: deep node".to_owned();
    let labeled = tree_choice_rows(&identity_item, false, 48);
    assert!(labeled[0].plain_text().contains("checkpoint"));
    assert!(labeled[1].plain_text().contains("deep node"));
    assert!(!labeled[1].plain_text().contains("checkpoint"));
    assert!(!labeled[1].plain_text().contains("assistant:"));

    for (role, kind, color) in [
        (Some("user"), "message", palette::BLUE),
        (Some("assistant"), "message", palette::MAUVE),
        (Some("toolCall"), "message", palette::TEAL),
        (Some("toolResult"), "message", palette::PEACH),
        (None, "compaction", palette::RED),
        (None, "branch_summary", palette::GREEN),
        (None, "custom_message", palette::PINK),
    ] {
        identity_item.role = role.map(ToOwned::to_owned);
        identity_item.kind = kind.to_owned();
        assert_eq!(tree_identity_color(&identity_item), color);
    }
}

#[test]
fn resize_rebuilds_every_row_at_the_new_width_and_height() {
    let mut domain = state();
    domain
        .transcript
        .push(TranscriptItem::Assistant(AssistantMessage {
            text: "你👩🏽‍💻 mixed-width content ".repeat(20),
            complete: false,
            ..AssistantMessage::default()
        }));
    let mut store = UiStore::new(super::super::types::TerminalSize::new(120, 40));
    store.synchronize(&domain);

    for size in [
        super::super::types::TerminalSize::new(120, 40),
        super::super::types::TerminalSize::new(80, 24),
        super::super::types::TerminalSize::new(200, 60),
    ] {
        store.reduce(super::super::store::UiEvent::Resize(size));
        let frame = SceneBuilder.build(&view(&domain), store.state(), SurfaceKind::Primary);
        assert_eq!(frame.terminal_size, size);
        assert_eq!(frame.rows.len(), usize::from(size.height));
        assert_eq!(frame.viewport.bottom(), size.height);
        assert!(frame.viewport.height <= size.height);
        assert!(
            frame
                .rows
                .iter()
                .all(|row| row.display_width() <= size.width)
        );
    }
}

#[test]
fn busy_to_idle_converges_in_the_same_frame_without_layout_holes() {
    let mut domain = state();
    let mut store = UiStore::new(super::super::types::TerminalSize::new(80, 24));
    store.synchronize(&domain);
    domain.run_state = RunState::Running;
    store.reduce(super::super::store::UiEvent::DomainChanged);
    let busy = SceneBuilder.build(&view(&domain), store.state(), SurfaceKind::Primary);

    domain.run_state = RunState::Idle;
    store.reduce(super::super::store::UiEvent::DomainChanged);
    let idle = SceneBuilder.build(&view(&domain), store.state(), SurfaceKind::Primary);
    assert_eq!(busy.main_layout, idle.main_layout);
    assert_eq!(busy.rows.len(), idle.rows.len());
    let busy_status = busy.rows.last().unwrap().plain_text();
    let idle_status = idle.rows.last().unwrap().plain_text();
    assert!(busy_status.starts_with(" model · thinking off"));
    assert!(busy_status.contains("⠋ · ctx"));
    assert!(!busy_status.contains("running"));
    assert!(!busy_status.contains("connected"));
    assert!(idle_status.starts_with(" model · thinking off"));
    assert!(!idle_status.contains('⠋'));
    assert!(!idle_status.contains("idle"));
    assert!(!idle_status.contains("connected"));
}

#[test]
fn composer_has_a_full_width_border_and_cursor_stays_inside_it() {
    let mut domain = state();
    domain.editor.insert_text("hello");
    let size = super::super::types::TerminalSize::new(32, 10);
    let mut store = UiStore::new(size);
    store.synchronize(&domain);

    let frame = SceneBuilder.build(&view(&domain), store.state(), SurfaceKind::Primary);
    let composer = frame.main_layout.composer;
    let top = &frame.rows[usize::from(composer.y)];
    let content = &frame.rows[usize::from(composer.y.saturating_add(1))];
    let bottom = &frame.rows[usize::from(composer.bottom().saturating_sub(1))];

    assert_eq!(top.display_width(), size.width);
    assert_eq!(content.display_width(), size.width);
    assert_eq!(bottom.display_width(), size.width);
    assert!(top.plain_text().starts_with('╭'));
    assert!(top.plain_text().ends_with('╮'));
    assert!(content.plain_text().starts_with("│› "));
    assert!(content.plain_text().ends_with('│'));
    assert!(
        content
            .cells
            .iter()
            .filter(|cell| {
                !cell.symbol.trim().is_empty() && cell.symbol != "│" && cell.symbol != "›"
            })
            .all(|cell| !cell.style.bold)
    );
    assert!(bottom.plain_text().starts_with('╰'));
    assert!(bottom.plain_text().ends_with('╯'));
    assert!(frame.cursor.is_some_and(|cursor| {
        cursor.row > composer.y
            && cursor.row < composer.bottom().saturating_sub(1)
            && cursor.column < size.width
    }));
}

#[test]
fn status_line_fills_the_width_without_a_background() {
    let domain = state();
    let size = super::super::types::TerminalSize::new(48, 10);
    let mut store = UiStore::new(size);
    store.synchronize(&domain);

    let frame = SceneBuilder.build(&view(&domain), store.state(), SurfaceKind::Primary);
    let status = frame.rows.last().expect("status row");
    assert_eq!(status.display_width(), size.width);
    assert!(status.plain_text().starts_with(' '));
    assert!(status.plain_text().ends_with(' '));
    assert!(
        status
            .cells
            .iter()
            .all(|cell| cell.style.background == Color::Default && !cell.style.reversed)
    );
}

#[test]
fn plan_review_panel_shows_only_execute_fresh_execute_close() {
    let mut domain = state();
    domain.plan_review = Some(crate::state::PlanReviewState {
        selected: 0,
        submitting: false,
    });
    domain.context = crate::state::ContextSnapshot {
        usage_state: crate::state::ContextUsageState::Estimated,
        actual_tokens: Some(40_000),
        actual_percent: Some(40.0),
        context_window: Some(100_000),
        ..crate::state::ContextSnapshot::default()
    };

    let panel = primary_panel_request(&view(&domain), 48).expect("plan review panel");
    let text = panel
        .rows
        .iter()
        .map(|row| row.plain_text())
        .collect::<Vec<_>>()
        .join(" ");

    assert!(text.contains("Current context remaining: 60% (estimated)"));
    for label in ["Execute", "Fresh execute", "Close"] {
        assert!(text.contains(label), "missing {label}: {text}");
    }
    assert!(!text.contains("Confirm"));
    assert!(!text.contains("Execute in current context"));
    assert_eq!(panel.selected_row, Some(1));
}

#[test]
fn plan_review_panel_shows_unknown_when_context_is_unavailable() {
    let mut domain = state();
    domain.plan_review = Some(crate::state::PlanReviewState {
        selected: 2,
        submitting: false,
    });

    let panel = primary_panel_request(&view(&domain), 48).expect("plan review panel");
    let text = panel
        .rows
        .iter()
        .map(|row| row.plain_text())
        .collect::<Vec<_>>()
        .join(" ");

    assert!(text.contains("Current context remaining: unknown"));
    assert_eq!(panel.selected_row, Some(3));
}

#[test]
fn open_panel_sits_flush_against_the_composer() {
    let size = super::super::types::TerminalSize::new(48, 14);
    let mut store = UiStore::new(size);
    let baseline_domain = state();
    store.synchronize(&baseline_domain);
    let baseline = SceneBuilder.build(&view(&baseline_domain), store.state(), SurfaceKind::Primary);

    let mut domain = state();
    domain.plan_review = Some(crate::state::PlanReviewState {
        selected: 0,
        submitting: false,
    });
    store.synchronize(&domain);

    let frame = SceneBuilder.build(&view(&domain), store.state(), SurfaceKind::Primary);
    let panel = frame.panel.expect("plan review panel");

    assert_eq!(frame.main_layout.composer, baseline.main_layout.composer);
    assert_eq!(frame.main_layout.composer.y, panel.area.bottom());
    assert!(
        panel
            .rows
            .last()
            .expect("panel border")
            .plain_text()
            .starts_with('╰')
    );
    assert!(
        frame.rows[usize::from(frame.main_layout.composer.y)]
            .plain_text()
            .starts_with('╭')
    );
}

#[test]
fn trust_prompt_question_modal_omits_custom_answer() {
    let mut domain = state();
    domain.question = Some(crate::state::QuestionFlowState {
        request_id: "workspace-trust".to_owned(),
        questions: vec![crate::state::PlanQuestion {
            id: "trust".to_owned(),
            prompt: "Trust this workspace?".to_owned(),
            options: vec![
                crate::state::QuestionOption {
                    id: "trust".to_owned(),
                    label: "Trust workspace".to_owned(),
                    description: None,
                },
                crate::state::QuestionOption {
                    id: "deny".to_owned(),
                    label: "Don't trust".to_owned(),
                    description: None,
                },
            ],
        }],
        current: 0,
        selected: 0,
        custom_answer: false,
        editor: crate::state::EditorState::default(),
        answers: Vec::new(),
        replying: false,
        workspace_trust_prompt: true,
    });

    let request = crate::ui::scene::panels::modals::question::question_modal(&view(&domain), 80)
        .expect("trust prompt panel");
    let text = request
        .rows
        .iter()
        .map(|row| row.plain_text())
        .collect::<Vec<_>>()
        .join(" ");

    assert!(text.contains("Trust workspace"));
    assert!(text.contains("Don't trust"));
    assert!(!text.contains("Custom answer"));
}
