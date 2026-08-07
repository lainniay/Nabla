use std::{
    error::Error,
    io,
    time::{Duration, Instant},
};

use crossterm::event::Event as TerminalEvent;
use nabla::{
    app::{App, AppEffect, AppEvent},
    config::UiConfig,
    event::RuntimeEvent,
    pi_process::{PiProcessConfig, PiRuntime},
    runtime::{DispatchOutcome, EffectDispatcher},
    ui::{
        CanonicalReflowProjection, CommittedHistoryBlock, FrameCoordinator, SceneBuilder,
        SurfaceKind, SurfaceManager, TerminalDriver, TerminalSize, TranscriptStore, UiEvent,
        UiStore, animation_active,
    },
};
use tokio::{
    sync::mpsc,
    time::{MissedTickBehavior, interval},
};

#[derive(Debug)]
struct ResizeDebouncer {
    pending: Option<(TerminalSize, Instant)>,
    delay: Duration,
}

const CANONICAL_REPLAY_BATCH_ROWS: usize = 256;
const CANONICAL_REPLAY_BATCH_BYTES: usize = 64 * 1024;

#[derive(Debug)]
struct CanonicalReplayState {
    projection: CanonicalReflowProjection,
    batches: Vec<Vec<CommittedHistoryBlock>>,
    next_batch: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecoveryProgress {
    InProgress,
    RestartRequired,
    Complete,
}

impl ResizeDebouncer {
    fn new(delay: Duration) -> Self {
        Self {
            pending: None,
            delay,
        }
    }

    fn queue(&mut self, size: TerminalSize, now: Instant) {
        self.pending = Some((size, now));
    }

    fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    fn due(&self, now: Instant) -> Option<TerminalSize> {
        self.pending
            .filter(|(_, queued_at)| now.duration_since(*queued_at) >= self.delay)
            .map(|(size, _)| size)
    }

    fn clear(&mut self) {
        self.pending = None;
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    match std::env::args().nth(1).as_deref() {
        Some("__sandbox-probe") => {
            nabla::sandbox::run_probe().map_err(std::io::Error::other)?;
            return Ok(());
        }
        Some("__sandbox-exec") => {
            nabla::sandbox::run_exec().map_err(std::io::Error::other)?;
            return Ok(());
        }
        _ => {}
    }

    let cwd = std::env::current_dir()?.canonicalize()?;
    let mut runtime = PiRuntime::spawn(PiProcessConfig::local(cwd.clone())).await?;
    let (session, bootstrap) =
        match tokio::try_join!(runtime.client.get_state(), runtime.host.bootstrap_state()) {
            Ok(state) => state,
            Err(error) => {
                let _ = runtime.process.shutdown().await;
                return Err(error.into());
            }
        };

    let size = TerminalSize::from(crossterm::terminal::size()?);
    let mut app = App::new(session);
    app.set_selection_page_size(usize::from(size.height.saturating_sub(4).max(1)));
    app.set_initial_bootstrap_state(bootstrap);
    let mut ui = UiStore::new(size);
    ui.synchronize(app.state());
    let ui_config = UiConfig::from_env();

    let mut terminal = match TerminalDriver::open(size) {
        Ok(terminal) => terminal,
        Err(error) => {
            let _ = runtime.process.shutdown().await;
            return Err(format!("failed to initialize terminal: {error}").into());
        }
    };
    let ui_result = run(
        &mut app,
        &mut ui,
        &mut terminal,
        &mut runtime,
        cwd,
        ui_config,
    )
    .await;
    let terminal_result = terminal.finish();
    let shutdown_result = runtime.process.shutdown().await;

    terminal_result?;
    let status = shutdown_result?;
    ui_result?;
    if !status.success() {
        return Err(format!("Pi exited with status {status}").into());
    }
    Ok(())
}

async fn run(
    app: &mut App,
    ui: &mut UiStore,
    terminal: &mut TerminalDriver<std::io::Stdout>,
    runtime: &mut PiRuntime,
    workspace: std::path::PathBuf,
    ui_config: UiConfig,
) -> Result<(), Box<dyn Error>> {
    let (event_tx, mut event_rx) = mpsc::channel::<AppEvent>(512);
    spawn_terminal_reader(event_tx.clone());
    let dispatcher = EffectDispatcher::new(
        runtime.client.clone(),
        runtime.host.clone(),
        event_tx,
        workspace,
    )?;
    let mut tick = interval(Duration::from_millis(100));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut coordinator = FrameCoordinator::default();
    let mut pi_open = true;
    let mut host_open = true;
    let mut stderr_open = true;
    let mut terminal_failures = 0usize;
    const RESIZE_DEBOUNCE: Duration = Duration::from_millis(75);
    let mut resize_debouncer = ResizeDebouncer::new(RESIZE_DEBOUNCE);
    let mut canonical_replay = None;

    render(app, ui, terminal, &mut coordinator)?;

    loop {
        let event = tokio::select! {
            event = event_rx.recv() => event,
            event = runtime.events.recv(), if pi_open => {
                match event {
                    Some(Ok(event)) => Some(AppEvent::Pi(event)),
                    Some(Err(error)) => Some(AppEvent::Runtime(RuntimeEvent::PiRpcError(error.to_string()))),
                    None => {
                        pi_open = false;
                        Some(AppEvent::Runtime(RuntimeEvent::PiDisconnected))
                    }
                }
            }
            event = runtime.host_events.recv(), if host_open => {
                match event {
                    Some(Ok(event)) => Some(AppEvent::Host(event)),
                    Some(Err(error)) => Some(AppEvent::Runtime(RuntimeEvent::PiRpcError(
                        format!("host control transport failed: {error}")
                    ))),
                    None => {
                        host_open = false;
                        Some(AppEvent::Runtime(RuntimeEvent::HostDisconnected))
                    }
                }
            }
            line = runtime.process.recv_stderr(), if stderr_open => {
                match line {
                    Some(line) => Some(AppEvent::Runtime(RuntimeEvent::PiStderr(line))),
                    None => {
                        stderr_open = false;
                        None
                    }
                }
            }
            _ = tick.tick() => {
                let now = Instant::now();
                if let Ok(actual_size) = crossterm::terminal::size() {
                    let actual_size = TerminalSize::from(actual_size);
                    if actual_size != ui.state().terminal.size {
                        canonical_replay = None;
                        let width_changed = actual_size.width != ui.state().terminal.size.width;
                        ui.reduce(UiEvent::Resize(actual_size));
                        app.set_selection_page_size(
                            usize::from(actual_size.height.saturating_sub(4).max(1))
                        );
                        coordinator.invalidate();
                        if width_changed || resize_debouncer.is_pending() {
                            resize_debouncer.queue(actual_size, now);
                        }
                    }
                }
                let tick_result = ui.reduce(UiEvent::Tick {
                    now,
                    animate: animation_active(app.state()),
                });
                let resize_due = resize_debouncer.due(now).is_some();
                let recovery_ready = canonical_replay.is_some()
                    || (ui.state().terminal.projection_requires_rebuild
                        && (!resize_debouncer.is_pending() || resize_due));
                if recovery_ready {
                    match drive_canonical_recovery(
                        app,
                        ui,
                        terminal,
                        &mut coordinator,
                        ui_config.resize_reflow_max_rows,
                        &mut canonical_replay,
                    ) {
                        Ok(RecoveryProgress::Complete) => {
                            if resize_due {
                                resize_debouncer.clear();
                            }
                            terminal_failures = 0;
                        }
                        Ok(RecoveryProgress::InProgress) => {}
                        Ok(RecoveryProgress::RestartRequired) => {
                            canonical_replay = None;
                        }
                        Err(error) => {
                            canonical_replay = None;
                            ui.reduce(UiEvent::ProjectionInvalidated);
                            terminal_failures += 1;
                            if terminal_failures >= 3 {
                                return Err(error.into());
                            }
                        }
                    }
                } else if !resize_debouncer.is_pending()
                    && (tick_result.changed || coordinator.terminal_invalid)
                {
                    if let Err(error) = render(app, ui, terminal, &mut coordinator) {
                        terminal_failures += 1;
                        if terminal_failures >= 3 {
                            return Err(error.into());
                        }
                    } else {
                        terminal_failures = 0;
                    }
                }
                None
            }
        };
        let Some(event) = event else {
            continue;
        };

        if let AppEvent::Terminal(TerminalEvent::Resize(columns, rows)) = &event {
            canonical_replay = None;
            let size = TerminalSize::new(*columns, *rows);
            let width_changed = size.width != ui.state().terminal.size.width;
            ui.reduce(UiEvent::Resize(size));
            app.set_selection_page_size(usize::from(rows.saturating_sub(4).max(1)));
            coordinator.invalidate();
            if width_changed || resize_debouncer.is_pending() {
                resize_debouncer.queue(size, Instant::now());
            }
        }

        let effects = app.update(event);
        ui.reduce(UiEvent::DomainChanged);
        ui.synchronize(app.state());
        let mut quit = false;
        for effect in effects {
            match dispatcher.dispatch(effect) {
                DispatchOutcome::Continue => {}
                DispatchOutcome::Quit => quit = true,
                DispatchOutcome::ExitWithError(error) => {
                    return Err(io::Error::other(error).into());
                }
            }
        }

        let recovery_ready = canonical_replay.is_some()
            || (ui.state().terminal.projection_requires_rebuild && !resize_debouncer.is_pending());
        if recovery_ready {
            match drive_canonical_recovery(
                app,
                ui,
                terminal,
                &mut coordinator,
                ui_config.resize_reflow_max_rows,
                &mut canonical_replay,
            ) {
                Ok(RecoveryProgress::Complete) => terminal_failures = 0,
                Ok(RecoveryProgress::InProgress) => {}
                Ok(RecoveryProgress::RestartRequired) => {
                    canonical_replay = None;
                }
                Err(error) => {
                    canonical_replay = None;
                    ui.reduce(UiEvent::ProjectionInvalidated);
                    terminal_failures += 1;
                    if terminal_failures >= 3 {
                        return Err(error.into());
                    }
                }
            }
        } else if !resize_debouncer.is_pending() {
            match render(app, ui, terminal, &mut coordinator) {
                Ok(()) => terminal_failures = 0,
                Err(error) => {
                    terminal_failures += 1;
                    if terminal_failures >= 3 {
                        return Err(error.into());
                    }
                }
            }
        }
        if quit {
            return Ok(());
        }
    }
}

fn render(
    app: &mut App,
    ui: &mut UiStore,
    terminal: &mut TerminalDriver<std::io::Stdout>,
    coordinator: &mut FrameCoordinator,
) -> io::Result<()> {
    if ui.state().terminal.projection_requires_rebuild {
        return Err(io::Error::other(
            "incremental render is disabled until canonical recovery",
        ));
    }
    let surface = SurfaceManager.route(app.state());
    let current_surface = ui.state().terminal.surface;
    if surface != current_surface {
        ui.reduce(match surface {
            SurfaceKind::Primary => UiEvent::LeaveAlternate,
            SurfaceKind::Alternate => UiEvent::EnterAlternate,
        });
        coordinator.invalidate();
    }
    let provisional = SceneBuilder.build(app.state(), ui.state(), surface);
    let (frame, projection) = if surface == SurfaceKind::Primary {
        let history_window = provisional.main_layout.history_window;
        let projection = ui.state().transcript.project_primary(
            history_window.width,
            usize::from(history_window.height),
            ui.state().revision,
            CANONICAL_REPLAY_BATCH_ROWS,
            CANONICAL_REPLAY_BATCH_BYTES,
            ui.state().animation_frame,
        );
        let frame =
            SceneBuilder.build_with_projection(app.state(), ui.state(), surface, &projection);
        (frame, Some(projection))
    } else {
        (provisional, None)
    };
    let plan = coordinator.plan(frame, surface, projection);
    let result = coordinator.commit(terminal, &mut ui.state_mut().transcript, plan);
    if result.is_err() {
        ui.reduce(if surface == SurfaceKind::Primary {
            UiEvent::ProjectionInvalidated
        } else {
            UiEvent::TerminalFailed
        });
    }
    result
}

fn drive_canonical_recovery(
    app: &mut App,
    ui: &mut UiStore,
    terminal: &mut TerminalDriver<std::io::Stdout>,
    coordinator: &mut FrameCoordinator,
    maximum_rows: usize,
    replay: &mut Option<CanonicalReplayState>,
) -> io::Result<RecoveryProgress> {
    if replay.is_none() {
        ui.state().transcript.invalidate_render_caches();
        let provisional = SceneBuilder.build(app.state(), ui.state(), SurfaceKind::Primary);
        let history_window = provisional.main_layout.history_window;
        let projection = ui.state().transcript.canonical_reflow_projection(
            history_window.width,
            usize::from(history_window.height),
            ui.state().revision,
            maximum_rows,
        );
        let batches = TranscriptStore::canonical_reflow_batches(
            &projection,
            if maximum_rows == 0 {
                CANONICAL_REPLAY_BATCH_ROWS
            } else {
                maximum_rows.min(CANONICAL_REPLAY_BATCH_ROWS)
            },
            CANONICAL_REPLAY_BATCH_BYTES,
        );
        terminal.begin_canonical_reflow()?;
        coordinator.invalidate();
        *replay = Some(CanonicalReplayState {
            projection,
            batches,
            next_batch: 0,
        });
    }

    let compatible = ui.state().transcript.reflow_projection_is_compatible(
        &replay
            .as_ref()
            .expect("canonical replay initialized")
            .projection,
    );
    if !compatible {
        *replay = None;
        return Ok(RecoveryProgress::RestartRequired);
    }
    let state = replay.as_mut().expect("canonical replay initialized");
    if let Some(batch) = state.batches.get(state.next_batch) {
        terminal.replay_canonical_history_batch(batch)?;
        state.next_batch += 1;
        return Ok(RecoveryProgress::InProgress);
    }

    let projection = state.projection.clone();
    let mut preview = ui.state().clone();
    if !preview.transcript.apply_reflow_projection(&projection) {
        return Err(io::Error::other(
            "canonical history changed incompatibly during replay",
        ));
    }
    let resident_projection = nabla::ui::PrimaryTranscriptProjection {
        overflow_blocks: Vec::new(),
        resident_rows: projection.resident_rows.clone(),
        bootstrap_padding_rows: projection.bootstrap_padding_rows,
        resident_capacity: projection.resident_capacity,
        scrollback_cursor: projection.scrollback_cursor,
        scrollback_row_offset: projection.scrollback_row_offset,
        canonical_revision: projection.canonical_revision,
    };
    let frame = SceneBuilder.build_with_projection(
        app.state(),
        &preview,
        SurfaceKind::Primary,
        &resident_projection,
    );
    let plan = coordinator.plan_canonical_reflow_frame(frame, &resident_projection);
    coordinator.finish_canonical_reflow(
        terminal,
        &mut ui.state_mut().transcript,
        plan,
        &projection,
    )?;
    ui.reduce(UiEvent::ProjectionRebuilt);
    *replay = None;

    if SurfaceManager.route(app.state()) == SurfaceKind::Alternate {
        render(app, ui, terminal, coordinator)?;
    }
    Ok(RecoveryProgress::Complete)
}

fn spawn_terminal_reader(events: mpsc::Sender<AppEvent>) {
    std::thread::Builder::new()
        .name("nabla-terminal-events".to_owned())
        .spawn(move || {
            loop {
                let event = match crossterm::event::read() {
                    Ok(event) => AppEvent::Terminal(event),
                    Err(error) => {
                        let _ = events.blocking_send(AppEvent::Runtime(
                            RuntimeEvent::TerminalError(error.to_string()),
                        ));
                        break;
                    }
                };
                let closed = matches!(event, AppEvent::Runtime(RuntimeEvent::TerminalClosed));
                if events.blocking_send(event).is_err() || closed {
                    break;
                }
            }
        })
        .expect("failed to start terminal event reader");
}

#[allow(dead_code)]
fn _assert_effect_is_send(effect: AppEffect) {
    fn assert_send<T: Send>(_: T) {}
    assert_send(effect);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_reflow_debounce_uses_only_the_final_size() {
        let start = Instant::now();
        let mut debounce = ResizeDebouncer::new(Duration::from_millis(75));
        debounce.queue(TerminalSize::new(80, 24), start);
        debounce.queue(TerminalSize::new(40, 20), start + Duration::from_millis(30));
        debounce.queue(
            TerminalSize::new(120, 36),
            start + Duration::from_millis(60),
        );

        assert_eq!(debounce.due(start + Duration::from_millis(100)), None);
        assert_eq!(
            debounce.due(start + Duration::from_millis(135)),
            Some(TerminalSize::new(120, 36))
        );
    }
}
