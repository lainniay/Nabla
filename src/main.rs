use std::{
    error::Error,
    io,
    time::{Duration, Instant},
};

use crossterm::event::Event as TerminalEvent;
use nabla::{
    app::{App, AppEffect, AppEvent},
    event::RuntimeEvent,
    pi_process::{PiProcessConfig, PiRuntime},
    runtime::{DispatchOutcome, EffectDispatcher},
    ui::{
        FrameCoordinator, SceneBuilder, SurfaceKind, SurfaceManager, TerminalDriver, TerminalSize,
        UiEvent, UiStore, animation_active,
    },
};
use tokio::{
    sync::mpsc,
    time::{MissedTickBehavior, interval},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
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

    let mut terminal = match TerminalDriver::open(size) {
        Ok(terminal) => terminal,
        Err(error) => {
            let _ = runtime.process.shutdown().await;
            return Err(format!("failed to initialize terminal: {error}").into());
        }
    };
    let ui_result = run(&mut app, &mut ui, &mut terminal, &mut runtime, cwd).await;
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
                let tick_result = ui.reduce(UiEvent::Tick {
                    now: Instant::now(),
                    animate: animation_active(app.state()),
                });
                if tick_result.changed || coordinator.terminal_invalid {
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
            let size = TerminalSize::new(*columns, *rows);
            ui.reduce(UiEvent::Resize(size));
            app.set_selection_page_size(usize::from(rows.saturating_sub(4).max(1)));
            coordinator.invalidate();
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

        match render(app, ui, terminal, &mut coordinator) {
            Ok(()) => terminal_failures = 0,
            Err(error) => {
                terminal_failures += 1;
                if terminal_failures >= 3 {
                    return Err(error.into());
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
    // Resize events can be coalesced or delayed by terminal multiplexers.
    // Sampling immediately before layout keeps the owned surface bounded by
    // the actual viewport even when no Resize event reaches the event loop.
    if let Ok(actual_size) = crossterm::terminal::size() {
        let actual_size = TerminalSize::from(actual_size);
        if actual_size != ui.state().terminal.size {
            ui.reduce(UiEvent::Resize(actual_size));
            app.set_selection_page_size(usize::from(actual_size.height.saturating_sub(4).max(1)));
            coordinator.invalidate();
        }
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
    let size = ui.state().terminal.size;
    let history = if surface == SurfaceKind::Primary {
        ui.state()
            .transcript
            .pending_history(size.width, ui.state().revision, 4096)
    } else {
        Vec::new()
    };
    let frame = SceneBuilder.build(app.state(), ui.state(), surface, history.len());
    let plan = coordinator.plan(frame, surface, history);
    coordinator.commit(terminal, &mut ui.state_mut().transcript, plan)
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
