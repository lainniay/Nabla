use crate::{
    rpc::PiState,
    state::{
        AppState, AssistantMessage, PlanArtifact, ToolExecution, ToolStatus, TranscriptItem,
        TranscriptViewMode, TurnSeparator, UserMessage, UserMessageStatus,
    },
    ui::{
        palette,
        transcript::render::common::format_turn_duration,
        transcript::render::{
            ToolRenderMode, render_item, render_tool, render_turn_separator, render_user,
            render_viewer_item,
        },
        transcript::store::cache::RENDER_CACHE_MAX_ENTRIES,
        types::{Color, CommittedHistoryBlock, TranscriptSyncOutcome, VisualRow},
    },
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::*;

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
fn only_the_contiguous_sealed_prefix_can_enter_history() {
    let mut state = state();
    state.transcript = vec![
        TranscriptItem::User(UserMessage {
            text: "done".to_owned(),
            status: UserMessageStatus::Accepted,
        }),
        TranscriptItem::Assistant(AssistantMessage {
            text: "stream".to_owned(),
            complete: false,
            ..AssistantMessage::default()
        }),
        TranscriptItem::Notice("later".to_owned()),
    ];
    let mut store = TranscriptStore::default();
    assert_eq!(store.sync(&state), TranscriptSyncOutcome::AppendOnly);
    let pending = store
        .project_primary(80, 0, 1, 24, usize::MAX, 0)
        .overflow_blocks;
    assert_eq!(pending.len(), 1);
    assert_eq!(store.scrollback_cursor(), 0);
    store.acknowledge_overflow(&pending);
    assert_eq!(store.scrollback_cursor(), 1);
    assert_eq!(
        store.phase(&pending[0].component_id),
        Some(ComponentPhase::Committed)
    );
}

#[test]
fn resident_window_stable_rows_overflow_only_after_being_pushed_out() {
    let mut state = state();
    state.transcript.push(TranscriptItem::User(UserMessage {
        text: "oldest".to_owned(),
        status: UserMessageStatus::Accepted,
    }));
    let mut store = TranscriptStore::default();
    store.sync(&state);
    let resident = store.project_primary(40, 8, 1, 100, usize::MAX, 0);
    assert!(resident.overflow_blocks.is_empty());
    assert!(
        resident
            .resident_rows
            .iter()
            .any(|row| row.plain_text().contains("oldest"))
    );

    state
        .transcript
        .extend((0..12).map(|index| TranscriptItem::Notice(format!("new-{index}"))));
    store.sync(&state);
    let overflowed = store.project_primary(40, 8, 2, 100, usize::MAX, 0);
    assert!(!overflowed.overflow_blocks.is_empty());
    assert_eq!(overflowed.resident_rows.len(), 8);
}

#[test]
fn streaming_rows_never_enter_overflow_even_when_taller_than_resident_window() {
    let mut state = state();
    state
        .transcript
        .push(TranscriptItem::Assistant(AssistantMessage {
            text: "```text\none\ntwo\nthree\nfour\nfive\nsix".to_owned(),
            complete: false,
            ..AssistantMessage::default()
        }));
    let mut store = TranscriptStore::default();
    store.sync(&state);
    let projection = store.project_primary(40, 3, 1, 100, usize::MAX, 0);

    assert!(projection.overflow_blocks.is_empty());
    assert_eq!(projection.resident_rows.len(), 3);
}

#[test]
fn one_component_can_overflow_at_row_granularity_without_duplication() {
    let mut state = state();
    state
        .transcript
        .push(TranscriptItem::Assistant(AssistantMessage {
            text: "```text\none\ntwo\nthree\nfour\nfive\nsix\n```".to_owned(),
            complete: true,
            ..AssistantMessage::default()
        }));
    let mut store = TranscriptStore::default();
    store.sync(&state);
    let projection = store.project_primary(40, 3, 1, 100, usize::MAX, 0);
    let overflow = projection
        .overflow_blocks
        .first()
        .expect("partial component overflow");

    assert_eq!(overflow.row_offset, 0);
    assert!(overflow.rows.len() < overflow.total_rows);
    assert_eq!(
        overflow.rows.len() + projection.resident_rows.len(),
        overflow.total_rows
    );
}

#[test]
fn bootstrap_padding_is_monotonic_and_never_enters_canonical_projection() {
    let mut state = state();
    let mut store = TranscriptStore::default();
    store.sync(&state);
    let empty = store.project_primary(40, 8, 1, 100, usize::MAX, 0);
    state
        .transcript
        .push(TranscriptItem::Notice("resident".to_owned()));
    store.sync(&state);
    let grown = store.project_primary(40, 8, 2, 100, usize::MAX, 0);

    assert!(grown.bootstrap_padding_rows < empty.bootstrap_padding_rows);
    assert_eq!(store.render_canonical_history(40), grown.resident_rows);
}

#[test]
fn phase_changes_do_not_move_resident_rows() {
    let mut state = state();
    state
        .transcript
        .push(TranscriptItem::Assistant(AssistantMessage {
            id: 1,
            text: "same visible row".to_owned(),
            complete: false,
            ..AssistantMessage::default()
        }));
    let mut store = TranscriptStore::default();
    store.sync(&state);
    let streaming = store.project_primary(40, 8, 1, 100, usize::MAX, 0);
    let TranscriptItem::Assistant(message) = &mut state.transcript[0] else {
        unreachable!()
    };
    message.complete = true;
    store.sync(&state);
    let sealed = store.project_primary(40, 8, 2, 100, usize::MAX, 0);

    assert_eq!(streaming.resident_rows, sealed.resident_rows);
    assert!(sealed.overflow_blocks.is_empty());
}

#[test]
fn stable_prefix_segments_commit_before_the_streaming_tail() {
    let mut state = state();
    state
        .transcript
        .push(TranscriptItem::Assistant(AssistantMessage {
            id: 7,
            text: "first paragraph\n\nsecond paragraph\n\nmutable".to_owned(),
            text_revision: 1,
            complete: false,
            ..AssistantMessage::default()
        }));
    let mut store = TranscriptStore::default();
    store.sync(&state);

    assert_eq!(store.order.len(), 3);
    assert_eq!(store.phase(&store.order[0]), Some(ComponentPhase::Stable));
    assert_eq!(store.phase(&store.order[1]), Some(ComponentPhase::Stable));
    assert_eq!(
        store.phase(&store.order[2]),
        Some(ComponentPhase::Streaming)
    );
    let stable_ids = store.order[..2].to_vec();
    let pending = store
        .project_primary(80, 0, 1, 100, usize::MAX, 0)
        .overflow_blocks;
    assert_eq!(
        pending
            .iter()
            .map(|block| block.component_id.clone())
            .collect::<Vec<_>>(),
        stable_ids
    );
    store.acknowledge_overflow(&pending);
    assert_eq!(store.scrollback_cursor(), 2);
    assert_eq!(store.uncommitted_components().count(), 1);

    let TranscriptItem::Assistant(message) = &mut state.transcript[0] else {
        unreachable!()
    };
    message.text.push_str(" tail");
    message.text_revision += 1;
    store.sync(&state);
    assert_eq!(&store.order[..2], stable_ids.as_slice());
    assert_eq!(store.scrollback_cursor(), 2);
}

#[test]
fn fenced_code_and_tables_remain_streaming_until_structurally_complete() {
    for source in [
        "```rust\nfn main() {}",
        "| key | value |\n|---|---|\n| one | two |",
    ] {
        let mut state = state();
        state
            .transcript
            .push(TranscriptItem::Assistant(AssistantMessage {
                id: 9,
                text: source.to_owned(),
                text_revision: 1,
                complete: false,
                ..AssistantMessage::default()
            }));
        let mut store = TranscriptStore::default();
        store.sync(&state);
        assert!(
            store
                .project_primary(80, 0, 1, 100, usize::MAX, 0)
                .overflow_blocks
                .is_empty()
        );
        assert_eq!(
            store.phase(&store.order[0]),
            Some(ComponentPhase::Streaming)
        );
    }
}

#[test]
fn history_batches_acknowledge_rows_before_the_whole_segment() {
    let mut state = state();
    let body = (0..64)
        .map(|index| format!("line {index}\n"))
        .collect::<String>();
    state
        .transcript
        .push(TranscriptItem::Assistant(AssistantMessage {
            id: 11,
            text: format!("```text\n{body}```"),
            text_revision: 1,
            complete: true,
            ..AssistantMessage::default()
        }));
    let mut store = TranscriptStore::default();
    store.sync(&state);

    let first = store
        .project_primary(24, 0, 1, 7, usize::MAX, 0)
        .overflow_blocks;
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].row_offset, 0);
    assert_eq!(first[0].rows.len(), 7);
    assert!(first[0].total_rows > first[0].rows.len());
    store.acknowledge_overflow(&first);
    assert_eq!(store.scrollback_cursor(), 0);
    assert_eq!(store.scrollback_row_offset(), 7);

    let second = store
        .project_primary(24, 0, 2, 7, usize::MAX, 0)
        .overflow_blocks;
    assert_eq!(second[0].row_offset, 7);
}

#[test]
fn history_offsets_are_usize_beyond_u16() {
    let id = "large".to_owned();
    let block = Arc::new(TranscriptBlock {
        id: id.clone(),
        item: TranscriptItem::Notice("large".to_owned()),
        assistant_segment: None,
        leading_blank: false,
        trailing_blank: false,
        render_cache: Arc::new(Mutex::new(HashMap::new())),
    });
    let mut store = TranscriptStore::default();
    store.order.push(id.clone());
    store.components.insert(id.clone(), block);
    store.phases.insert(id.clone(), ComponentPhase::Sealed);
    store.acknowledge_overflow(&[CommittedHistoryBlock {
        component_id: id,
        source_revision: 1,
        row_offset: 0,
        total_rows: 70_000,
        rows: vec![VisualRow::blank("large"); 65_536],
    }]);
    assert_eq!(store.scrollback_cursor(), 0);
    assert_eq!(store.scrollback_row_offset(), 65_536);
}

#[test]
fn stable_render_rows_are_cached_per_width() {
    let mut state = state();
    state
        .transcript
        .push(TranscriptItem::Assistant(AssistantMessage {
            id: 12,
            text: "cached paragraph".to_owned(),
            text_revision: 1,
            complete: true,
            ..AssistantMessage::default()
        }));
    let mut store = TranscriptStore::default();
    store.sync(&state);
    let block = store.uncommitted_components().next().unwrap();
    block.render(40);
    block.render(40);
    assert_eq!(block.render_cache.lock().unwrap().len(), 1);
    block.render(20);
    assert_eq!(block.render_cache.lock().unwrap().len(), 2);
}

#[test]
fn render_cache_is_bounded_across_terminal_widths() {
    let mut state = state();
    state
        .transcript
        .push(TranscriptItem::Assistant(AssistantMessage {
            id: 13,
            text: "bounded cache".to_owned(),
            text_revision: 1,
            complete: true,
            ..AssistantMessage::default()
        }));
    let mut store = TranscriptStore::default();
    store.sync(&state);
    let block = store.uncommitted_components().next().unwrap();
    for width in 1..=70 {
        block.render(width);
    }
    assert!(block.render_cache.lock().unwrap().len() <= RENDER_CACHE_MAX_ENTRIES);
}

#[test]
fn projection_reset_reflows_canonical_segments_at_the_new_width() {
    let mut state = state();
    state
        .transcript
        .push(TranscriptItem::Assistant(AssistantMessage {
            id: 13,
            text: "This canonical sentence is intentionally long enough to wrap differently."
                .to_owned(),
            text_revision: 1,
            complete: true,
            ..AssistantMessage::default()
        }));
    let mut store = TranscriptStore::default();
    store.sync(&state);
    let ids = store.order.clone();
    let wide = store
        .project_primary(80, 0, 1, 100, usize::MAX, 0)
        .overflow_blocks;
    let wide_rows = wide.iter().map(|block| block.rows.len()).sum::<usize>();
    store.acknowledge_overflow(&wide);
    assert_eq!(store.scrollback_cursor(), store.order.len());

    store.reset_projection();
    let narrow = store
        .project_primary(20, 0, 2, 100, usize::MAX, 0)
        .overflow_blocks;
    let narrow_rows = narrow.iter().map(|block| block.rows.len()).sum::<usize>();
    assert_eq!(store.order, ids);
    assert!(narrow_rows > wide_rows);
    assert_eq!(store.scrollback_cursor(), 0);
}

#[test]
fn canonical_resize_reflow_preserves_ids_and_rebuilds_the_resident_tail() {
    let mut state = state();
    state.transcript = vec![
        TranscriptItem::User(UserMessage {
            text: "A user message that wraps at narrow widths".to_owned(),
            status: UserMessageStatus::Accepted,
        }),
        TranscriptItem::Notice("canonical notice".to_owned()),
        TranscriptItem::Assistant(AssistantMessage {
            id: 14,
            text: "mutable streaming tail".to_owned(),
            text_revision: 1,
            complete: false,
            ..AssistantMessage::default()
        }),
    ];
    let mut store = TranscriptStore::default();
    store.sync(&state);
    let ids = store.order.clone();

    let wide = store.canonical_reflow_projection(80, 3, 1, 0);
    let narrow = store.canonical_reflow_projection(20, 3, 2, 0);
    let wide_rows = wide
        .history_blocks
        .iter()
        .map(|block| block.rows.len())
        .sum::<usize>();
    let narrow_rows = narrow
        .history_blocks
        .iter()
        .map(|block| block.rows.len())
        .sum::<usize>();

    assert_eq!(store.order, ids);
    assert_eq!(
        wide.history_blocks
            .iter()
            .map(|block| &block.component_id)
            .collect::<Vec<_>>(),
        narrow
            .history_blocks
            .iter()
            .map(|block| &block.component_id)
            .collect::<Vec<_>>()
    );
    assert!(narrow_rows > wide_rows);
    assert!(
        narrow
            .resident_rows
            .iter()
            .any(|row| row.plain_text().contains("mutable"))
    );
}

#[test]
fn canonical_recovery_restarts_when_the_resident_tail_changes() {
    let mut state = state();
    state.transcript = vec![
        TranscriptItem::Notice("committed".to_owned()),
        TranscriptItem::Assistant(AssistantMessage {
            id: 91,
            text: "live".to_owned(),
            text_revision: 1,
            complete: false,
            ..AssistantMessage::default()
        }),
    ];
    let mut store = TranscriptStore::default();
    assert_eq!(store.sync(&state), TranscriptSyncOutcome::AppendOnly);
    let pending = store
        .project_primary(40, 0, 1, 100, usize::MAX, 0)
        .overflow_blocks;
    store.acknowledge_overflow(&pending);
    assert_eq!(store.scrollback_cursor(), 1);
    let replay = store.canonical_reflow_projection(40, 0, 2, 0);

    let TranscriptItem::Assistant(message) = state.transcript.last_mut().unwrap() else {
        unreachable!()
    };
    message.text.push_str(" tail");
    message.text_revision += 1;
    assert_eq!(store.sync(&state), TranscriptSyncOutcome::AppendOnly);
    assert_eq!(store.scrollback_cursor(), 1);
    assert!(!store.reflow_projection_is_compatible(&replay));

    state
        .transcript
        .push(TranscriptItem::Notice("appended".to_owned()));
    assert_eq!(store.sync(&state), TranscriptSyncOutcome::AppendOnly);
    assert_eq!(store.scrollback_cursor(), 1);

    state.transcript[0] = TranscriptItem::Notice("replaced".to_owned());
    assert_eq!(
        store.sync(&state),
        TranscriptSyncOutcome::ProjectionInvalidated
    );
    assert_eq!(store.scrollback_cursor(), 0);
    assert!(!store.reflow_projection_is_compatible(&replay));
}

#[test]
fn session_epoch_change_always_invalidates_projection() {
    let mut state = state();
    state
        .transcript
        .push(TranscriptItem::Notice("session A".to_owned()));
    let mut store = TranscriptStore::default();
    assert_eq!(store.sync(&state), TranscriptSyncOutcome::AppendOnly);
    state.session_epoch += 1;
    state.transcript[0] = TranscriptItem::Notice("session B".to_owned());
    assert_eq!(
        store.sync(&state),
        TranscriptSyncOutcome::ProjectionInvalidated
    );
}

#[test]
fn recovery_row_limit_never_truncates_the_physical_scrollback_cursor() {
    let mut state = state();
    state.transcript = vec![
        TranscriptItem::Notice("one".to_owned()),
        TranscriptItem::Notice("two".to_owned()),
        TranscriptItem::Notice("three".to_owned()),
    ];
    let mut store = TranscriptStore::default();
    store.sync(&state);

    let limited = store.canonical_reflow_projection(80, 0, 1, 2);
    assert_eq!(limited.history_blocks.len(), 3);
    assert_eq!(
        limited
            .history_blocks
            .iter()
            .map(|block| block.rows[0].plain_text())
            .collect::<Vec<_>>(),
        vec!["! Notice · one", "! Notice · two", "! Notice · three"]
    );

    let unlimited = store.canonical_reflow_projection(80, 0, 2, 0);
    assert_eq!(
        limited
            .history_blocks
            .iter()
            .flat_map(|block| block.rows.iter())
            .cloned()
            .collect::<Vec<_>>(),
        unlimited
            .history_blocks
            .iter()
            .flat_map(|block| block.rows.iter())
            .cloned()
            .collect::<Vec<_>>()
    );
}

#[test]
fn resize_reflow_preserves_every_row_of_an_oversized_component() {
    let mut state = state();
    state.transcript.push(TranscriptItem::User(UserMessage {
        text: "word ".repeat(200),
        status: UserMessageStatus::Accepted,
    }));
    let mut store = TranscriptStore::default();
    store.sync(&state);

    let projection = store.canonical_reflow_projection(12, 0, 1, 3);
    assert_eq!(projection.history_blocks.len(), 1);
    assert_eq!(projection.history_blocks[0].row_offset, 0);
    assert_eq!(
        projection.history_blocks[0].rows.len(),
        projection.history_blocks[0].total_rows
    );
    assert!(store.apply_reflow_projection(&projection));
    assert_eq!(store.scrollback_cursor(), store.order.len());
    assert_eq!(
        store.render_canonical_history(12).len(),
        projection.history_blocks[0].total_rows
    );
}

#[test]
fn canonical_replay_batches_split_physical_rows_without_loss() {
    let mut state = state();
    state.transcript.push(TranscriptItem::User(UserMessage {
        text: (0..10_100)
            .map(|line| format!("line-{line}\n"))
            .collect::<String>(),
        status: UserMessageStatus::Accepted,
    }));
    let mut store = TranscriptStore::default();
    store.sync(&state);
    let projection = store.canonical_reflow_projection(80, 0, 1, 0);
    let batches = TranscriptStore::canonical_reflow_batches(&projection, 128, 64 * 1024);

    assert!(batches.len() > 70);
    assert!(
        batches
            .iter()
            .all(|batch| { batch.iter().map(|block| block.rows.len()).sum::<usize>() <= 128 })
    );
    let replayed = batches
        .iter()
        .flat_map(|batch| batch.iter())
        .flat_map(|block| block.rows.iter())
        .map(VisualRow::plain_text)
        .collect::<Vec<_>>();
    let canonical = projection
        .history_blocks
        .iter()
        .flat_map(|block| block.rows.iter())
        .map(VisualRow::plain_text)
        .collect::<Vec<_>>();
    assert_eq!(replayed, canonical);
}

#[test]
fn canonical_projection_handles_more_than_ten_thousand_history_rows() {
    let mut state = state();
    state.transcript = (0..10_050)
        .map(|index| TranscriptItem::Notice(format!("row {index}")))
        .collect();
    let mut store = TranscriptStore::default();
    store.sync(&state);

    let projection = store.canonical_reflow_projection(40, 20, 1, 0);
    let overflow_rows = projection
        .history_blocks
        .iter()
        .map(|block| block.rows.len())
        .sum::<usize>();
    assert_eq!(overflow_rows, 10_030);
    assert_eq!(projection.resident_rows.len(), 20);
    assert_eq!(
        projection.resident_rows.last().map(VisualRow::plain_text),
        Some("! Notice · row 10049".to_owned())
    );
}

#[test]
fn streaming_resize_then_completion_matches_uninterrupted_rendering() {
    let initial = "Stable paragraph.\n\n```rust\nfn main() {";
    let completed = concat!(
        "Stable paragraph.\n\n",
        "```rust\nfn main() {}\n```\n\n",
        "| a | b |\n|---|---|\n| 1 | 2 |\n"
    );
    let mut resized_state = state();
    resized_state
        .transcript
        .push(TranscriptItem::Assistant(AssistantMessage {
            id: 44,
            text: initial.to_owned(),
            text_revision: 1,
            complete: false,
            ..AssistantMessage::default()
        }));
    let mut resized = TranscriptStore::default();
    resized.sync(&resized_state);
    let projection = resized.canonical_reflow_projection(24, 0, 1, 0);
    assert!(resized.apply_reflow_projection(&projection));

    let TranscriptItem::Assistant(message) = resized_state.transcript.last_mut().unwrap() else {
        unreachable!()
    };
    message.text = completed.to_owned();
    message.text_revision = 2;
    message.complete = true;
    resized.sync(&resized_state);

    let mut uninterrupted = TranscriptStore::default();
    uninterrupted.sync(&resized_state);
    assert_eq!(
        resized
            .render_canonical_history(24)
            .into_iter()
            .map(|row| row.plain_text())
            .collect::<Vec<_>>(),
        uninterrupted
            .render_canonical_history(24)
            .into_iter()
            .map(|row| row.plain_text())
            .collect::<Vec<_>>()
    );
    let mut unique_ids = resized.order.clone();
    unique_ids.sort();
    unique_ids.dedup();
    assert_eq!(unique_ids.len(), resized.order.len());
}

#[test]
fn assistant_messages_render_markdown_in_the_primary_transcript() {
    let block = TranscriptBlock {
        id: "assistant:markdown".to_owned(),
        item: TranscriptItem::Assistant(AssistantMessage {
            text: "# Result\n\nUse **bold** and `cargo test`.\n\n- first\n- second".to_owned(),
            complete: true,
            ..AssistantMessage::default()
        }),
        assistant_segment: None,
        leading_blank: false,
        trailing_blank: false,
        render_cache: Arc::new(Mutex::new(HashMap::new())),
    };

    let rows = block.render(42);
    let text = rows
        .iter()
        .map(VisualRow::plain_text)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rows[0].plain_text().starts_with("• # Result"));
    assert!(text.contains("Use bold and cargo test."));
    assert!(text.contains("  - first"));
    assert!(rows.iter().all(|row| row.display_width() <= 42));
    assert!(
        rows.iter()
            .flat_map(|row| &row.cells)
            .any(|cell| { cell.symbol == "b" && cell.style.bold })
    );
    assert!(
        rows.iter()
            .flat_map(|row| &row.cells)
            .any(|cell| { cell.symbol == "c" && cell.style.foreground == palette::SAPPHIRE })
    );
}

#[test]
fn tool_updates_replace_the_canonical_snapshot_by_id() {
    let mut state = state();
    state.transcript.push(TranscriptItem::Tool(ToolExecution {
        id: "call-1".to_owned(),
        name: "read".to_owned(),
        args: json!({"path": "a"}),
        output: "partial".to_owned(),
        diff: None,
        status: ToolStatus::Running,
    }));
    let mut store = TranscriptStore::default();
    store.sync(&state);
    let revision = store.revision;
    let TranscriptItem::Tool(tool) = &mut state.transcript[0] else {
        unreachable!()
    };
    tool.output = "complete".to_owned();
    tool.status = ToolStatus::Succeeded;
    store.sync(&state);
    assert!(store.revision > revision);
    assert_eq!(store.order, vec!["tool:call-1"]);
    assert_eq!(store.phase("tool:call-1"), Some(ComponentPhase::Sealed));
    assert_eq!(
        store
            .uncommitted_components()
            .next()
            .unwrap()
            .render(40)
            .len(),
        2
    );
}

#[test]
fn primary_tools_are_two_line_summaries_and_never_render_output_body() {
    let tool = ToolExecution {
        id: "call-1".to_owned(),
        name: "bash".to_owned(),
        args: json!({"command": "cargo test --all"}),
        output: "PRIVATE OUTPUT\nsecond line\nthird line".to_owned(),
        diff: None,
        status: ToolStatus::Succeeded,
    };

    let rows = render_item("tool:call-1", &TranscriptItem::Tool(tool), 60, 0);
    let text = rows
        .iter()
        .map(VisualRow::plain_text)
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(rows.len(), 2);
    assert!(text.contains("• Ran"));
    assert!(text.contains("└ cargo test --all"));
    assert!(text.contains("· 3 lines"));
    assert!(!text.contains("succeeded"));
    assert!(!text.contains("PRIVATE OUTPUT"));
}

#[test]
fn successful_structured_edits_render_inline_file_diffs() {
    let patch = "\
--- a/one.rs
+++ b/one.rs
@@ -1,2 +1,2 @@
-old
+new
 same
--- a/two.rs
+++ b/two.rs
@@ -0,0 +1,2 @@
+first
+second
--- a/three.rs
+++ b/three.rs
@@ -1 +0,0 @@
-gone
";
    let tool = ToolExecution {
        id: "edit-1".to_owned(),
        name: "edit".to_owned(),
        args: serde_json::Value::Null,
        output: "done".to_owned(),
        diff: crate::state::parse_tool_diff(&serde_json::Value::Null, &json!({"patch": patch})),
        status: ToolStatus::Succeeded,
    };

    let rows = render_tool("edit", &tool, 80, ToolRenderMode::Compact, 0);
    let text = rows
        .iter()
        .map(VisualRow::plain_text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("Edited 3 files (+3 -2)"));
    assert!(text.contains("└ one.rs (+1 -1)"));
    assert!(text.contains("└ two.rs (+2 -0)"));
    assert!(text.contains("└ three.rs (+0 -1)"));
    assert!(text.contains("1 -old"));
    assert!(text.contains("1 +new"));
    assert!(rows.iter().all(|row| row.display_width() <= 80));
    assert!(
        rows.iter()
            .flat_map(|row| &row.cells)
            .any(|cell| { cell.symbol == "+" && cell.style.foreground == palette::GREEN })
    );
    assert!(
        rows.iter()
            .flat_map(|row| &row.cells)
            .any(|cell| { cell.symbol == "-" && cell.style.foreground == palette::RED })
    );
    let added = rows
        .iter()
        .find(|row| row.plain_text().contains("1 +new"))
        .expect("added diff row");
    assert!(
        added
            .cells
            .iter()
            .all(|cell| cell.style.background == palette::DIFF_ADDED_BACKGROUND)
    );
    assert_eq!(added.display_width(), 80);
    let removed = rows
        .iter()
        .find(|row| row.plain_text().contains("1 -old"))
        .expect("removed diff row");
    assert!(
        removed
            .cells
            .iter()
            .all(|cell| cell.style.background == palette::DIFF_REMOVED_BACKGROUND)
    );
    assert_eq!(removed.display_width(), 80);
    let context = rows
        .iter()
        .find(|row| row.plain_text().contains("2  same"))
        .expect("context diff row");
    assert!(
        context
            .cells
            .iter()
            .all(|cell| cell.style.background == Color::Default)
    );
    assert!(context.display_width() < 80);
    assert!(
        rows[0]
            .cells
            .iter()
            .all(|cell| cell.style.background == Color::Default)
    );
}

#[test]
fn compact_edit_diffs_are_bounded_and_viewer_expansion_is_complete() {
    let display_diff = (1..=45)
        .map(|line| format!("+{line} line {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let tool = ToolExecution {
        id: "edit-large".to_owned(),
        name: "edit".to_owned(),
        args: json!({"path": "src/\u{1b}[31mlib.rs"}),
        output: "done".to_owned(),
        diff: crate::state::parse_tool_diff(
            &json!({"path": "src/\u{1b}[31mlib.rs"}),
            &json!({"diff": display_diff}),
        ),
        status: ToolStatus::Succeeded,
    };

    let compact = render_tool("edit", &tool, 64, ToolRenderMode::Compact, 0);
    let compact_text = compact
        .iter()
        .map(VisualRow::plain_text)
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(compact.len(), 43);
    assert!(compact_text.contains("5 more diff lines"));
    assert!(!compact_text.contains('\u{1b}'));

    let expanded = render_viewer_item(
        "edit",
        &TranscriptItem::Tool(tool),
        64,
        TranscriptViewMode::Normal,
        true,
        false,
    );
    assert_eq!(expanded.len(), 47);
    assert!(
        expanded
            .last()
            .unwrap()
            .plain_text()
            .contains("45 +line 45")
    );
}

#[test]
fn primary_shell_tools_use_token_colors_and_only_surface_non_success_states() {
    let tool = ToolExecution {
        id: "call-shell".to_owned(),
        name: "bash".to_owned(),
        args: json!({"command": "cargo test --all ./src | rg '你好'"}),
        output: String::new(),
        diff: None,
        status: ToolStatus::Succeeded,
    };
    let rows = render_tool("tool", &tool, 80, ToolRenderMode::Compact, 0);
    assert_eq!(rows.len(), 2);
    assert!(!rows[1].plain_text().contains("0 B"));
    for color in [
        palette::SAPPHIRE,
        palette::BLUE,
        palette::GREEN,
        palette::PEACH,
        palette::YELLOW,
    ] {
        assert!(
            rows[1]
                .cells
                .iter()
                .any(|cell| cell.style.foreground == color),
            "missing {color:?}"
        );
    }

    let running = render_tool(
        "running",
        &ToolExecution {
            status: ToolStatus::Running,
            ..tool.clone()
        },
        80,
        ToolRenderMode::Compact,
        2,
    );
    assert!(running[0].plain_text().starts_with("⠹ Ran"));
    assert!(running[1].plain_text().ends_with("· running"));

    let waiting = render_tool(
        "waiting",
        &ToolExecution {
            status: ToolStatus::WaitingApproval,
            ..tool.clone()
        },
        80,
        ToolRenderMode::Compact,
        2,
    );
    assert!(waiting[0].plain_text().starts_with("● Ran"));
    assert_eq!(waiting[0].cells[0].style.foreground, palette::YELLOW);

    let failed = render_tool(
        "failed",
        &ToolExecution {
            output: "private failure body".to_owned(),
            status: ToolStatus::Failed,
            ..tool
        },
        80,
        ToolRenderMode::Compact,
        0,
    );
    assert_eq!(failed.len(), 3);
    assert!(failed[2].plain_text().contains("failed"));
    assert!(
        !failed
            .iter()
            .map(VisualRow::plain_text)
            .collect::<String>()
            .contains("private failure body")
    );
}

#[test]
fn transcript_viewer_modes_fold_expand_summarize_and_highlight_tools() {
    let item = TranscriptItem::Tool(ToolExecution {
        id: "call-1".to_owned(),
        name: "read_file".to_owned(),
        args: json!({"path": "src/lib.rs", "line": 4}),
        output: "complete output\nwith every line".to_owned(),
        diff: None,
        status: ToolStatus::Succeeded,
    });

    let normal = render_viewer_item(
        "viewer",
        &item,
        48,
        TranscriptViewMode::Normal,
        false,
        false,
    );
    let normal_text = normal
        .iter()
        .map(VisualRow::plain_text)
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(normal.len(), 2);
    assert!(!normal_text.contains("complete output"));
    assert!(!normal_text.contains("\"line\""));

    let verbose = render_viewer_item("viewer", &item, 48, TranscriptViewMode::Verbose, true, true);
    let verbose_text = verbose
        .iter()
        .map(VisualRow::plain_text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(verbose_text.contains("Arguments"));
    assert!(verbose_text.contains("\"line\": 4"));
    assert!(verbose_text.contains("complete output"));
    assert!(
        verbose
            .iter()
            .flat_map(|row| &row.cells)
            .all(|cell| cell.style.background == palette::SURFACE_0)
    );
    assert!(verbose.iter().all(|row| row.display_width() == 48));

    let summary = render_viewer_item(
        "viewer",
        &item,
        48,
        TranscriptViewMode::Summary,
        true,
        false,
    );
    assert_eq!(summary.len(), 1);
    assert!(!summary[0].plain_text().contains("complete output"));
}

#[test]
fn user_messages_use_full_width_unicode_safe_frames_and_narrow_fallbacks() {
    let message = UserMessage {
        text: "Markdown **stays literal** and CJK wraps: 你好世界".to_owned(),
        status: UserMessageStatus::Accepted,
    };
    let rows = render_user("user", &message, 40);
    assert!(rows[0].plain_text().starts_with('╭'));
    assert!(rows.last().unwrap().plain_text().starts_with('╰'));
    assert!(rows.iter().all(|row| row.display_width() == 40));
    assert!(
        rows.iter()
            .map(VisualRow::plain_text)
            .collect::<String>()
            .contains("**stays literal**")
    );
    assert_eq!(rows[0].cells[0].style.foreground, palette::HISTORY_BORDER);
    assert!(rows[0].cells[0].style.dim);

    let narrow = render_user(
        "narrow",
        &UserMessage {
            status: UserMessageStatus::Failed,
            ..message
        },
        5,
    );
    assert!(narrow[0].plain_text().starts_with("› "));
    assert!(narrow.iter().all(|row| row.display_width() <= 5));
    assert!(
        narrow
            .iter()
            .flat_map(|row| &row.cells)
            .any(|cell| cell.style.foreground == Color::Red)
    );
}

#[test]
fn transcript_spacing_never_duplicates_blank_rows_between_groups() {
    let mut state = state();
    state.transcript = vec![
        TranscriptItem::User(UserMessage {
            text: "question".to_owned(),
            status: UserMessageStatus::Accepted,
        }),
        TranscriptItem::Assistant(AssistantMessage {
            text: "answer".to_owned(),
            complete: true,
            ..AssistantMessage::default()
        }),
        TranscriptItem::Tool(ToolExecution {
            id: "tool".to_owned(),
            name: "read".to_owned(),
            args: json!({"path": "src/lib.rs"}),
            output: String::new(),
            diff: None,
            status: ToolStatus::Succeeded,
        }),
    ];
    let mut store = TranscriptStore::default();
    store.sync(&state);
    let rows = store
        .uncommitted_components()
        .flat_map(|component| component.render(40))
        .collect::<Vec<_>>();
    assert!(
        !rows
            .windows(2)
            .any(|pair| pair[0].plain_text().is_empty() && pair[1].plain_text().is_empty())
    );
}

#[test]
fn thinking_and_each_tool_call_have_single_blank_separators() {
    let mut state = state();
    state.transcript = vec![
        TranscriptItem::Assistant(AssistantMessage {
            text: "final answer".to_owned(),
            thinking: "consider the options".to_owned(),
            complete: true,
            ..AssistantMessage::default()
        }),
        TranscriptItem::Tool(ToolExecution {
            id: "tool-1".to_owned(),
            name: "read".to_owned(),
            args: json!({"path": "src/lib.rs"}),
            output: String::new(),
            diff: None,
            status: ToolStatus::Succeeded,
        }),
        TranscriptItem::Tool(ToolExecution {
            id: "tool-2".to_owned(),
            name: "find".to_owned(),
            args: json!({"query": "palette"}),
            output: String::new(),
            diff: None,
            status: ToolStatus::Succeeded,
        }),
    ];
    let mut store = TranscriptStore::default();
    store.sync(&state);
    let blocks = store.uncommitted_components().collect::<Vec<_>>();
    let first_tool = blocks
        .iter()
        .find(|block| block.id == "tool:tool-1")
        .expect("first tool");
    let second_tool = blocks
        .iter()
        .find(|block| block.id == "tool:tool-2")
        .expect("second tool");
    assert!(first_tool.render(48)[0].plain_text().is_empty());
    assert!(second_tool.render(48)[0].plain_text().is_empty());

    let assistant = blocks
        .iter()
        .take_while(|block| block.assistant_segment.is_some())
        .flat_map(|block| block.render(48))
        .collect::<Vec<_>>();
    assert!(
        assistant
            .iter()
            .flat_map(|row| &row.cells)
            .any(|cell| { cell.symbol == "c" && cell.style.foreground == palette::THINKING_TEXT })
    );
    assert!(
        assistant
            .iter()
            .flat_map(|row| &row.cells)
            .any(|cell| { cell.symbol == "f" && cell.style.foreground == palette::TEXT })
    );
}

#[test]
fn turn_duration_formats_and_estimated_separator_are_stable() {
    assert_eq!(format_turn_duration(0), "<1s");
    assert_eq!(format_turn_duration(999), "<1s");
    assert_eq!(format_turn_duration(12_999), "12s");
    assert_eq!(format_turn_duration(65_000), "1m 05s");
    assert_eq!(format_turn_duration(3_720_000), "1h 02m");

    let rows = render_turn_separator(
        "turn",
        &TurnSeparator {
            turn_id: "turn".to_owned(),
            started_at: "start".to_owned(),
            ended_at: "end".to_owned(),
            duration_ms: 65_000,
            estimated: true,
        },
        40,
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].display_width(), 40);
    assert!(rows[0].plain_text().ends_with(" Worked for ~1m 05s ─"));
}

#[test]
fn plan_transcript_hides_status_and_expands_handoff() {
    let artifact = PlanArtifact {
        id: "plan-1".to_owned(),
        revision: 2,
        title: "Structured planning".to_owned(),
        summary: "Treat plans as artifacts.".to_owned(),
        body_markdown: "Implement the artifact flow.".to_owned(),
        assumptions: vec!["Rust owns interaction".to_owned()],
        test_plan: vec!["Run cargo test".to_owned()],
        handoff_markdown: "Carry the Plan into the implementation turn.".to_owned(),
        source_session_id: "session-1".to_owned(),
        created_at: "2026-01-01T00:00:00.000Z".to_owned(),
        updated_at: "2026-01-01T00:00:01.000Z".to_owned(),
    };
    let item = TranscriptItem::Plan(artifact.clone());

    let compact = render_item("plan", &item, 80, 0)
        .iter()
        .map(VisualRow::plain_text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(compact.contains("Plan · Structured planning"));
    assert!(compact.contains("(r2)"));
    assert!(compact.contains("## Assumptions"));
    assert!(compact.contains("## Test plan"));
    assert!(!compact.contains("submitted"));
    assert!(!compact.contains("## Handoff"));

    let expanded = render_viewer_item(
        "viewer",
        &item,
        80,
        TranscriptViewMode::Verbose,
        true,
        false,
    )
    .iter()
    .map(VisualRow::plain_text)
    .collect::<Vec<_>>()
    .join("\n");
    assert!(expanded.contains("## Handoff"));
    assert!(expanded.contains("Carry the Plan into the implementation turn."));
}
