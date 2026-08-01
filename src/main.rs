use std::{error::Error, future::Future, time::Duration};

use crossterm::event::{Event as TerminalEvent, EventStream, MouseButton, MouseEventKind};
use futures_util::StreamExt;
use nabla::{
    app::{App, AppEffect},
    browser,
    event::{AppEvent, CommandEvent, RuntimeEvent},
    host::{HostClient, HostEventReceiver},
    pi_process::{PiChildGuard, PiClient, PiEventReceiver, PiProcessConfig, PiRuntime},
    terminal_driver::{
        InlineTerminal, InlineTerminalMode, InlineViewportAnchor, TerminalSurfaceMode,
    },
    ui,
    ui_types::{RenderOutcome, UiHitMap, UiInputEvent},
};
use tokio::{
    sync::mpsc,
    time::{MissedTickBehavior, interval},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let cwd = std::env::current_dir()?;
    let PiRuntime {
        client,
        mut events,
        host,
        mut host_events,
        mut process,
    } = PiRuntime::spawn(PiProcessConfig::local_ui(cwd)).await?;

    let (state, bootstrap) = match tokio::try_join!(client.get_state(), host.bootstrap_state()) {
        Ok(runtime_state) => runtime_state,
        Err(error) => {
            let _ = process.shutdown().await;
            return Err(error.into());
        }
    };
    let (_, terminal_rows) = crossterm::terminal::size()?;
    let inline_viewport_height = 4.min(terminal_rows).max(1);
    let mut app = App::new(state);
    app.set_inline_viewport_height(inline_viewport_height);
    app.set_initial_bootstrap_state(bootstrap);

    let mut terminal = match InlineTerminal::new(inline_viewport_height) {
        Ok(terminal) => terminal,
        Err(error) => {
            ratatui::restore();
            let _ = process.shutdown().await;
            return Err(format!("failed to initialize terminal renderer: {error}").into());
        }
    };
    let mut transcript_presenter = ui::TranscriptPresenter::default();
    let runtime = TuiRuntime {
        client,
        host,
        events: &mut events,
        host_events: &mut host_events,
        process: &mut process,
    };
    let ui_result = run_tui(&mut terminal, &mut app, runtime, &mut transcript_presenter).await;
    let flush_result = terminal
        .set_surface_mode(TerminalSurfaceMode::Inline)
        .and_then(|()| transcript_presenter.flush(terminal.terminal_mut()))
        .and_then(|()| terminal.finish_inline());
    ratatui::restore();

    let shutdown_result = process.shutdown().await;
    flush_result?;
    let status = shutdown_result?;
    ui_result?;
    if !status.success() {
        return Err(format!("Pi exited with status {status}").into());
    }

    Ok(())
}

struct TuiRuntime<'a> {
    client: PiClient,
    host: HostClient,
    events: &'a mut PiEventReceiver,
    host_events: &'a mut HostEventReceiver,
    process: &'a mut PiChildGuard,
}

fn uses_native_scrollback(
    surface_mode: TerminalSurfaceMode,
    inline_mode: InlineTerminalMode,
) -> bool {
    surface_mode == TerminalSurfaceMode::Inline && inline_mode == InlineTerminalMode::Dynamic
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InlineResizePolicy {
    anchor: InlineViewportAnchor,
    preserve_released_top: bool,
}

fn inline_resize_policy(
    surface_mode: TerminalSurfaceMode,
    physical_size_unchanged: bool,
    bottom_pinned: bool,
    command_menu_was_visible: bool,
    command_menu_visible: bool,
    current_height: u16,
    desired_height: u16,
) -> InlineResizePolicy {
    let command_menu_controls_resize = surface_mode == TerminalSurfaceMode::Inline
        && physical_size_unchanged
        && (command_menu_was_visible || command_menu_visible);
    InlineResizePolicy {
        anchor: if bottom_pinned || command_menu_controls_resize {
            InlineViewportAnchor::Bottom
        } else {
            InlineViewportAnchor::Top
        },
        preserve_released_top: (bottom_pinned || command_menu_controls_resize)
            && desired_height < current_height,
    }
}

#[derive(Debug, Default)]
struct ViewportStabilizer {
    busy_height_floor: Option<u16>,
}

impl ViewportStabilizer {
    fn apply(
        &mut self,
        desired_height: u16,
        maximum_height: u16,
        main_surface: bool,
        busy: bool,
        idle_popup_visible: bool,
        reset: bool,
    ) -> u16 {
        if reset {
            self.busy_height_floor = None;
        }
        let desired_height = desired_height.min(maximum_height.max(1));
        if !main_surface {
            return desired_height;
        }
        if busy {
            let floor = self.busy_height_floor.get_or_insert(desired_height);
            *floor = (*floor).max(desired_height);
            return *floor;
        }
        if !idle_popup_visible {
            self.busy_height_floor = None;
        }
        desired_height
    }
}

async fn run_tui(
    terminal: &mut InlineTerminal,
    app: &mut App,
    runtime: TuiRuntime<'_>,
    transcript_presenter: &mut ui::TranscriptPresenter,
) -> Result<(), Box<dyn Error>> {
    let mut terminal_events = Some(EventStream::new());
    let (command_tx, mut command_rx) = mpsc::channel::<AppEvent>(256);
    let mut redraw = interval(Duration::from_millis(33));
    redraw.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut pi_events_open = true;
    let mut host_events_open = true;
    let mut pi_stderr_open = true;
    let mut hit_map = UiHitMap::default();
    let mut inline_command_menu_was_visible = false;
    let mut viewport_stabilizer = ViewportStabilizer::default();

    loop {
        let event = tokio::select! {
            terminal_event = terminal_events
                .as_mut()
                .expect("terminal event stream is restored before polling")
                .next() => {
                match terminal_event {
                    Some(Ok(TerminalEvent::Mouse(mouse))) => {
                        let event = match mouse.kind {
                            MouseEventKind::ScrollUp => Some(UiInputEvent::ScrollUp { lines: 3 }),
                            MouseEventKind::ScrollDown => {
                                Some(UiInputEvent::ScrollDown { lines: 3 })
                            }
                            MouseEventKind::Down(MouseButton::Left) => hit_map
                                .target_at(mouse.column, mouse.row)
                                .map(UiInputEvent::Click),
                            _ => None,
                        };
                        event.map(AppEvent::UiInput)
                    }
                    Some(Ok(event)) => Some(AppEvent::Terminal(event)),
                    Some(Err(error)) => {
                        Some(AppEvent::Runtime(RuntimeEvent::TerminalError(error.to_string())))
                    }
                    None => Some(AppEvent::Runtime(RuntimeEvent::TerminalClosed)),
                }
            }
            pi_event = runtime.events.recv(), if pi_events_open => {
                match pi_event {
                    Some(Ok(event)) => Some(AppEvent::Pi(event)),
                    Some(Err(error)) => Some(AppEvent::Runtime(RuntimeEvent::PiRpcError(
                        error.to_string(),
                    ))),
                    None => {
                        pi_events_open = false;
                        Some(AppEvent::Runtime(RuntimeEvent::PiDisconnected))
                    }
                }
            }
            host_event = runtime.host_events.recv(), if host_events_open => {
                match host_event {
                    Some(Ok(event)) => Some(AppEvent::Host(event)),
                    Some(Err(error)) => Some(AppEvent::Runtime(RuntimeEvent::PiRpcError(
                        format!("host control transport failed: {error}"),
                    ))),
                    None => {
                        host_events_open = false;
                        Some(AppEvent::Runtime(RuntimeEvent::HostDisconnected))
                    }
                }
            }
            stderr_line = runtime.process.recv_stderr(), if pi_stderr_open => {
                match stderr_line {
                    Some(line) => Some(AppEvent::Runtime(RuntimeEvent::PiStderr(line))),
                    None => {
                        pi_stderr_open = false;
                        None
                    }
                }
            }
            command_event = command_rx.recv() => command_event,
            _ = redraw.tick() => Some(AppEvent::Tick),
        };

        let Some(event) = event else {
            continue;
        };
        let terminal_resized = matches!(&event, AppEvent::Terminal(TerminalEvent::Resize(_, _)));
        let should_render = event.is_tick() || terminal_resized;

        // Crossterm's EventStream waits on the same global input reader used by
        // synchronous cursor-position reports. Inline viewport resizing needs
        // one such report, so release the stream before Ratatui autoresizes.
        if terminal_resized {
            drop(terminal_events.take());
        }

        let effects = app.update(event);

        for effect in effects {
            match effect {
                AppEffect::Prompt(message) => {
                    let client = runtime.client.clone();
                    spawn_command(&command_tx, async move {
                        let result = client
                            .prompt(message)
                            .await
                            .map_err(|error| error.to_string());
                        CommandEvent::PromptFinished(result)
                    });
                }
                AppEffect::Steer(message) => {
                    let client = runtime.client.clone();
                    spawn_command(&command_tx, async move {
                        let result = client
                            .steer(message)
                            .await
                            .map_err(|error| error.to_string());
                        CommandEvent::PromptFinished(result)
                    });
                }
                AppEffect::FollowUp(message) => {
                    let client = runtime.client.clone();
                    spawn_command(&command_tx, async move {
                        let result = client
                            .follow_up(message)
                            .await
                            .map_err(|error| error.to_string());
                        CommandEvent::PromptFinished(result)
                    });
                }
                AppEffect::Abort => {
                    let client = runtime.client.clone();
                    spawn_command(&command_tx, async move {
                        let result = client.abort().await.map_err(|error| error.to_string());
                        CommandEvent::AbortFinished(result)
                    });
                }
                AppEffect::ClearQueue => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .clear_queue()
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string());
                        CommandEvent::QueueCleared(result)
                    });
                }
                AppEffect::AbortAndClearQueue => {
                    let client = runtime.client.clone();
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let queue = host.clear_queue().await;
                        let abort = client.abort().await;
                        let result = match (queue, abort) {
                            (Ok(queue), Ok(())) => Ok(Box::new(queue)),
                            (_, Err(error)) => Err(error.to_string()),
                            (Err(error), _) => Err(error.to_string()),
                        };
                        CommandEvent::AbortAndQueueCleared(result)
                    });
                }
                AppEffect::Compact(custom_instructions) => {
                    let client = runtime.client.clone();
                    spawn_command(&command_tx, async move {
                        let result = client
                            .compact(custom_instructions)
                            .await
                            .map_err(|error| error.to_string());
                        CommandEvent::CompactFinished(result)
                    });
                }
                AppEffect::GetContextState => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .get_context_state()
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string());
                        CommandEvent::ContextStateFinished(result)
                    });
                }
                AppEffect::GetResources => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .get_resources()
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string());
                        CommandEvent::ResourcesFinished(result)
                    });
                }
                AppEffect::ReloadResources => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .reload_resources()
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string());
                        CommandEvent::ResourceReloadFinished(result)
                    });
                }
                AppEffect::SetWorkspaceTrust(trusted) => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .set_workspace_trust(trusted)
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string());
                        CommandEvent::WorkspaceTrustFinished(result)
                    });
                }
                AppEffect::GetGoal => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .get_goal()
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string());
                        CommandEvent::GoalStateFinished(result)
                    });
                }
                AppEffect::GetGoals => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .get_goals()
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string());
                        CommandEvent::GoalsFinished(result)
                    });
                }
                AppEffect::StartGoal {
                    objective,
                    from_plan,
                } => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .start_goal(objective, from_plan)
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string());
                        CommandEvent::GoalStarted(result)
                    });
                }
                AppEffect::GoalAction(action) => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .goal_action(action)
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string());
                        CommandEvent::GoalActionFinished(result)
                    });
                }
                AppEffect::ApproveGoal => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .approve_goal()
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string());
                        CommandEvent::GoalApproved(result)
                    });
                }
                AppEffect::ListModels => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .list_models()
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string());
                        CommandEvent::ModelsFinished(result)
                    });
                }
                AppEffect::SetModel { provider, model_id } => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .set_model(provider, model_id)
                            .await
                            .map_err(|error| error.to_string());
                        CommandEvent::ModelSetFinished(result)
                    });
                }
                AppEffect::SetThinking(level) => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .set_thinking(level)
                            .await
                            .map_err(|error| error.to_string());
                        CommandEvent::ThinkingSetFinished(result)
                    });
                }
                AppEffect::GetAgents => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .get_agents()
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string());
                        CommandEvent::AgentsFinished(result)
                    });
                }
                AppEffect::ReloadAgents => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .reload_agents()
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string());
                        CommandEvent::AgentsReloaded(result)
                    });
                }
                AppEffect::StartSubagent { profile, task } => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .start_subagent(profile, task)
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string());
                        CommandEvent::SubagentStarted(result)
                    });
                }
                AppEffect::CancelSubagent(agent_id) => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .cancel_subagent(agent_id)
                            .await
                            .map_err(|error| error.to_string());
                        CommandEvent::SubagentCancelled(result)
                    });
                }
                AppEffect::IntegrateSubagent { agent_id, action } => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .integrate_subagent(agent_id, action)
                            .await
                            .map_err(|error| error.to_string());
                        CommandEvent::SubagentIntegrated(result)
                    });
                }
                AppEffect::OpenSessionBrowser => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .open_session_browser()
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string());
                        CommandEvent::SessionBrowserOpened(result)
                    });
                }
                AppEffect::QuerySessionBrowser {
                    browser_id,
                    scope,
                    query,
                    sort_mode,
                    named_only,
                    offset,
                    generation,
                } => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .query_session_browser(
                                browser_id, scope, query, sort_mode, named_only, offset,
                            )
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string());
                        CommandEvent::SessionBrowserQueryFinished { generation, result }
                    });
                }
                AppEffect::CloseSessionBrowser { browser_id } => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .close_session_browser(browser_id)
                            .await
                            .map_err(|error| error.to_string());
                        CommandEvent::SessionBrowserClosed(result)
                    });
                }
                AppEffect::NewSession => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .new_session()
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string());
                        CommandEvent::NewSessionFinished(result)
                    });
                }
                AppEffect::ResumeSession {
                    session_path,
                    cwd_override,
                } => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .resume_session(session_path, cwd_override)
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string());
                        CommandEvent::ResumeSessionFinished(result)
                    });
                }
                AppEffect::GetTreeState {
                    filter_mode,
                    query,
                    folded_entry_ids,
                    generation,
                } => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .get_tree_state(filter_mode, query, folded_entry_ids)
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string());
                        CommandEvent::TreeStateFinished { generation, result }
                    });
                }
                AppEffect::SetTreeLabel { entry_id, label } => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .set_tree_label(entry_id, label)
                            .await
                            .map_err(|error| error.to_string());
                        CommandEvent::TreeLabelFinished(result)
                    });
                }
                AppEffect::CopyTreeEntry { entry_id } => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .copy_tree_entry(entry_id)
                            .await
                            .map_err(|error| error.to_string());
                        CommandEvent::TreeCopyFinished(result)
                    });
                }
                AppEffect::NavigateTree {
                    entry_id,
                    summarize,
                    custom_instructions,
                } => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .navigate_tree(entry_id, summarize, custom_instructions)
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string());
                        CommandEvent::TreeNavigateFinished(result)
                    });
                }
                AppEffect::AbortTreeNavigation => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .abort_tree_navigation()
                            .await
                            .map_err(|error| error.to_string());
                        CommandEvent::TreeAbortFinished(result)
                    });
                }
                AppEffect::AuthList => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .list_providers()
                            .await
                            .map_err(|error| error.to_string());
                        CommandEvent::AuthProvidersFinished(result)
                    });
                }
                AppEffect::AuthLogin {
                    flow_id,
                    provider_id,
                    auth_type,
                } => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .login(flow_id, provider_id, auth_type)
                            .await
                            .map_err(|error| error.to_string());
                        CommandEvent::AuthLoginFinished(result)
                    });
                }
                AppEffect::AuthReply {
                    flow_id,
                    prompt_id,
                    value,
                } => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .reply(flow_id, prompt_id, value.into_inner())
                            .await
                            .map_err(|error| error.to_string());
                        CommandEvent::AuthReplyFinished(result)
                    });
                }
                AppEffect::AuthCancel { flow_id } => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .cancel(flow_id)
                            .await
                            .map_err(|error| error.to_string());
                        CommandEvent::AuthCancelFinished(result)
                    });
                }
                AppEffect::OpenUrl(url) => {
                    spawn_command(&command_tx, async move {
                        let result = browser::open_url(&url).await;
                        CommandEvent::OpenUrlFinished(result)
                    });
                }
                AppEffect::SetPlanMode(active) => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .set_plan_mode(active)
                            .await
                            .map_err(|error| error.to_string());
                        CommandEvent::SetPlanModeFinished {
                            requested: active,
                            result,
                        }
                    });
                }
                AppEffect::ReplyApproval {
                    approval_id,
                    decision,
                } => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .reply_approval(approval_id.clone(), decision)
                            .await
                            .map_err(|error| error.to_string());
                        CommandEvent::ApprovalReplyFinished {
                            approval_id,
                            decision,
                            result,
                        }
                    });
                }
                AppEffect::ReplyQuestions {
                    request_id,
                    answers,
                } => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .reply_questions(request_id, answers)
                            .await
                            .map_err(|error| error.to_string());
                        CommandEvent::QuestionReplyFinished(result)
                    });
                }
                AppEffect::ExecutePlan(target) => {
                    let host = runtime.host.clone();
                    spawn_command(&command_tx, async move {
                        let result = host
                            .execute_plan(target)
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string());
                        CommandEvent::PlanExecutionFinished { target, result }
                    });
                }
                AppEffect::Quit => return Ok(()),
                AppEffect::ExitWithError(error) => {
                    return Err(std::io::Error::other(error).into());
                }
            }
        }

        if should_render && app.take_redraw_request() {
            let synchronized = terminal.begin_update()?;
            let surface_mode = if ui::uses_fullscreen_surface(app.state()) {
                TerminalSurfaceMode::Alternate
            } else {
                TerminalSurfaceMode::Inline
            };
            if surface_mode != terminal.surface_mode() && terminal_events.is_some() {
                drop(terminal_events.take());
            }
            terminal.set_surface_mode(surface_mode)?;
            if surface_mode == TerminalSurfaceMode::Inline {
                terminal.refresh_viewport();
            }
            let physical_size = crossterm::terminal::size()?;
            let (terminal_columns, terminal_rows) = physical_size;
            let command_menu_visible = surface_mode == TerminalSurfaceMode::Inline
                && app.state().active_modal_kind().is_none()
                && !app.state().command_candidates().is_empty();
            let mut prepared = uses_native_scrollback(surface_mode, terminal.mode()).then(|| {
                transcript_presenter.prepare(app.state(), terminal_columns.max(1) as usize)
            });
            let waiting_for_choice = matches!(
                app.state().active_modal_kind(),
                Some(
                    nabla::state::UiModalKind::Approval
                        | nabla::state::UiModalKind::GoalApproval
                        | nabla::state::UiModalKind::Question
                        | nabla::state::UiModalKind::Integration
                        | nabla::state::UiModalKind::PlanReview
                )
            );
            let retain_live_output = app.state().run_state.is_busy()
                || waiting_for_choice
                || transcript_presenter.has_mutable_table()
                || prepared
                    .as_ref()
                    .is_some_and(|projection| projection.projected().has_mutable_table());
            if let Some(prepared) = prepared.as_mut() {
                if retain_live_output {
                    prepared.retain_live_tail(app.state(), ui::LIVE_TRANSCRIPT_TAIL_HEIGHT);
                } else {
                    prepared.release_live_tail();
                }
            }
            let request = if let Some(prepared) = prepared.as_ref() {
                ui::measure_layout_request(
                    app.state(),
                    prepared.projected(),
                    terminal_columns,
                    terminal_rows,
                )
            } else {
                ui::measure_layout_request(
                    app.state(),
                    transcript_presenter,
                    terminal_columns,
                    terminal_rows,
                )
            };
            let busy = retain_live_output || transcript_presenter.has_live_tail();
            let previous_metrics = app.state().layout_metrics;
            let physical_size_unchanged = previous_metrics.terminal_columns == terminal_columns
                && previous_metrics.terminal_rows == terminal_rows;
            let applied_height = viewport_stabilizer.apply(
                request.desired_height(),
                terminal_rows,
                surface_mode == TerminalSurfaceMode::Inline,
                busy,
                command_menu_visible && !busy,
                !physical_size_unchanged,
            );
            let mut metrics = request.resolve_layout(applied_height);
            let resize_policy = inline_resize_policy(
                surface_mode,
                physical_size_unchanged,
                terminal.bottom_pinned(),
                inline_command_menu_was_visible,
                command_menu_visible,
                terminal.height(),
                metrics.desired_height,
            );
            if metrics.desired_height != terminal.height() && terminal_events.is_some() {
                drop(terminal_events.take());
            }

            if resize_policy.preserve_released_top {
                let viewport = terminal.viewport();
                let released_height = terminal.height().saturating_sub(metrics.desired_height);
                let restore_area = ratatui::layout::Rect::new(
                    viewport.x,
                    viewport.y,
                    viewport.width,
                    released_height,
                );
                terminal.terminal_mut().draw(|frame| {
                    ui::render_recent_history_background(frame, transcript_presenter, restore_area);
                })?;
            }

            // Inline terminals anchor themselves from the physical cursor when
            // their height changes. Resize first so insert_before writes
            // relative to the final viewport and cannot be cleared by a
            // terminal reconstruction later in this frame.
            terminal.resize_height(
                metrics.desired_height,
                physical_size,
                resize_policy.anchor,
                resize_policy.preserve_released_top,
            )?;
            if let Some(prepared_projection) = prepared.take() {
                if terminal.mode() == InlineTerminalMode::Dynamic {
                    transcript_presenter.commit(terminal.terminal_mut(), prepared_projection)?;
                    terminal.refresh_viewport();
                } else {
                    let fallback_request = ui::measure_layout_request(
                        app.state(),
                        transcript_presenter,
                        terminal_columns,
                        terminal_rows,
                    );
                    let fallback_height = viewport_stabilizer.apply(
                        fallback_request.desired_height(),
                        terminal_rows,
                        true,
                        busy,
                        command_menu_visible && !busy,
                        false,
                    );
                    metrics = fallback_request.resolve_layout(fallback_height);
                    if metrics.desired_height != terminal.height() && terminal_events.is_some() {
                        drop(terminal_events.take());
                    }
                    terminal.resize_height(
                        metrics.desired_height,
                        physical_size,
                        resize_policy.anchor,
                        resize_policy.preserve_released_top,
                    )?;
                }
            }
            app.set_layout_metrics(metrics);
            let mut outcome = RenderOutcome::default();
            let completed = terminal.terminal_mut().draw(|frame| {
                outcome = ui::render(frame, app.state(), transcript_presenter, metrics);
            })?;
            let viewport = completed.buffer.area;
            terminal.update_anchor(viewport);
            ui::render_terminal_overlays(app.state(), viewport)?;
            terminal.set_mouse_capture(outcome.mouse_capture)?;
            hit_map = outcome.hit_map;
            if surface_mode == TerminalSurfaceMode::Inline {
                inline_command_menu_was_visible = command_menu_visible;
            }
            synchronized.finish()?;
        }

        if terminal_events.is_none() {
            terminal_events = Some(EventStream::new());
        }
    }
}

fn spawn_command<F>(command_tx: &mpsc::Sender<AppEvent>, future: F)
where
    F: Future<Output = CommandEvent> + Send + 'static,
{
    let command_tx = command_tx.clone();
    tokio::spawn(async move {
        let _ = command_tx.send(AppEvent::Command(future.await)).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_dynamic_inline_surfaces_commit_native_scrollback() {
        assert!(uses_native_scrollback(
            TerminalSurfaceMode::Inline,
            InlineTerminalMode::Dynamic
        ));
        assert!(!uses_native_scrollback(
            TerminalSurfaceMode::Inline,
            InlineTerminalMode::FixedFallback
        ));
        assert!(!uses_native_scrollback(
            TerminalSurfaceMode::Alternate,
            InlineTerminalMode::Dynamic
        ));
    }

    #[test]
    fn command_menu_open_filter_and_close_keep_the_composer_bottom() {
        let opening =
            inline_resize_policy(TerminalSurfaceMode::Inline, true, false, false, true, 4, 12);
        assert_eq!(opening.anchor, InlineViewportAnchor::Bottom);
        assert!(!opening.preserve_released_top);

        let filtering =
            inline_resize_policy(TerminalSurfaceMode::Inline, true, false, true, true, 12, 7);
        assert_eq!(filtering.anchor, InlineViewportAnchor::Bottom);
        assert!(filtering.preserve_released_top);

        let closing =
            inline_resize_policy(TerminalSurfaceMode::Inline, true, false, true, false, 7, 4);
        assert_eq!(closing.anchor, InlineViewportAnchor::Bottom);
        assert!(closing.preserve_released_top);
    }

    #[test]
    fn physical_resize_and_alternate_surfaces_override_command_menu_anchoring() {
        let physical_resize = inline_resize_policy(
            TerminalSurfaceMode::Inline,
            false,
            false,
            true,
            false,
            12,
            4,
        );
        assert_eq!(physical_resize.anchor, InlineViewportAnchor::Top);
        assert!(!physical_resize.preserve_released_top);

        let alternate = inline_resize_policy(
            TerminalSurfaceMode::Alternate,
            true,
            false,
            true,
            false,
            24,
            24,
        );
        assert_eq!(alternate.anchor, InlineViewportAnchor::Top);
        assert!(!alternate.preserve_released_top);
    }

    #[test]
    fn bottom_pinned_resizes_keep_the_composer_coordinate() {
        let policy =
            inline_resize_policy(TerminalSurfaceMode::Inline, true, true, false, false, 12, 5);
        assert_eq!(policy.anchor, InlineViewportAnchor::Bottom);
        assert!(policy.preserve_released_top);
    }

    #[test]
    fn viewport_stabilizer_only_grows_while_busy_and_releases_at_idle() {
        let mut stabilizer = ViewportStabilizer::default();
        assert_eq!(stabilizer.apply(6, 24, true, true, false, false), 6);
        assert_eq!(stabilizer.apply(10, 24, true, true, false, false), 10);
        assert_eq!(stabilizer.apply(5, 24, true, true, false, false), 10);
        assert_eq!(stabilizer.apply(5, 24, true, false, false, false), 5);
    }

    #[test]
    fn fullscreen_and_idle_popups_do_not_pollute_the_busy_floor() {
        let mut stabilizer = ViewportStabilizer::default();
        assert_eq!(stabilizer.apply(10, 24, true, true, false, false), 10);
        assert_eq!(stabilizer.apply(24, 24, false, false, false, false), 24);
        assert_eq!(stabilizer.apply(7, 24, true, false, true, false), 7);
        assert_eq!(stabilizer.apply(4, 24, true, false, false, false), 4);
    }

    #[test]
    fn physical_resize_resets_and_clamps_the_busy_floor() {
        let mut stabilizer = ViewportStabilizer::default();
        assert_eq!(stabilizer.apply(20, 30, true, true, false, false), 20);
        assert_eq!(stabilizer.apply(8, 10, true, true, false, true), 8);
        assert_eq!(stabilizer.apply(12, 16, true, true, false, true), 12);
    }
}
