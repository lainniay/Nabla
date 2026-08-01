use std::io::{self, Write};

use crossterm::{
    cursor::MoveTo,
    event::{DisableMouseCapture, EnableMouseCapture},
    execute, queue,
    terminal::{
        BeginSynchronizedUpdate, Clear, ClearType, EndSynchronizedUpdate, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};
use ratatui::{
    DefaultTerminal, Terminal, TerminalOptions, Viewport, backend::CrosstermBackend, layout::Rect,
};

use crate::ui_types::MouseCaptureMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineTerminalMode {
    Dynamic,
    FixedFallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalSurfaceMode {
    Inline,
    Alternate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineViewportAnchor {
    Top,
    Bottom,
}

/// Owns the inline terminal footprint and all terminal-global modes.
///
/// Ratatui intentionally keeps an inline viewport's requested height immutable.
/// To grow and shrink it, this driver clears the previous footprint, restores
/// the anchor, and rebuilds the terminal around the same stdout handle.
pub struct InlineTerminal {
    terminal: DefaultTerminal,
    saved_inline_terminal: Option<DefaultTerminal>,
    mode: InlineTerminalMode,
    surface_mode: TerminalSurfaceMode,
    viewport: Rect,
    mouse_capture: MouseCaptureMode,
    terminal_size: (u16, u16),
    bottom_pinned: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ViewportResize {
    viewport: Rect,
    clear_area: Rect,
    scroll_from_current_anchor: bool,
}

fn viewport_resize(
    current: Rect,
    terminal_size: (u16, u16),
    desired_height: u16,
    anchor: InlineViewportAnchor,
) -> ViewportResize {
    let (columns, rows) = (terminal_size.0.max(1), terminal_size.1.max(1));
    let height = desired_height.clamp(1, rows);
    let scroll_from_current_anchor = anchor == InlineViewportAnchor::Top
        && current.bottom() <= rows
        && current.y.saturating_add(height) > rows;
    let y = match anchor {
        InlineViewportAnchor::Top if scroll_from_current_anchor => rows.saturating_sub(height),
        InlineViewportAnchor::Top => current.y.min(rows.saturating_sub(height)),
        InlineViewportAnchor::Bottom => current.bottom().min(rows).saturating_sub(height),
    };
    let viewport = Rect::new(0, y, columns, height);
    let clear_top = current.y.min(viewport.y);
    let clear_bottom = current.bottom().max(viewport.bottom()).min(rows);
    ViewportResize {
        viewport,
        clear_area: Rect::new(
            0,
            clear_top,
            columns,
            clear_bottom.saturating_sub(clear_top),
        ),
        scroll_from_current_anchor,
    }
}

fn clear_rows(output: &mut impl Write, area: Rect) -> io::Result<()> {
    for row in area.y..area.bottom() {
        queue!(output, MoveTo(area.x, row), Clear(ClearType::CurrentLine))?;
    }
    Ok(())
}

impl InlineTerminal {
    pub fn new(initial_height: u16) -> io::Result<Self> {
        let initial_height = initial_height.max(1);
        match ratatui::try_init_with_options(TerminalOptions {
            viewport: Viewport::Inline(initial_height),
        }) {
            Ok(mut terminal) => {
                let terminal_size = crossterm::terminal::size()?;
                let viewport = terminal.get_frame().area();
                Ok(Self {
                    terminal,
                    saved_inline_terminal: None,
                    mode: InlineTerminalMode::Dynamic,
                    surface_mode: TerminalSurfaceMode::Inline,
                    viewport,
                    mouse_capture: MouseCaptureMode::Off,
                    terminal_size,
                    bottom_pinned: viewport.bottom() >= terminal_size.1,
                })
            }
            Err(inline_error) => {
                ratatui::restore();
                let (columns, rows) = crossterm::terminal::size()?;
                let height = initial_height.min(rows).max(1);
                let area = Rect::new(0, rows.saturating_sub(height), columns, height);
                let terminal = ratatui::try_init_with_options(TerminalOptions {
                    viewport: Viewport::Fixed(area),
                })
                .map_err(|fallback_error| {
                    io::Error::other(format!(
                        "inline terminal unavailable ({inline_error}); fixed fallback failed: {fallback_error}"
                    ))
                })?;
                eprintln!(
                    "warning: dynamic inline viewport unavailable; using a fixed keyboard-only renderer"
                );
                Ok(Self {
                    terminal,
                    saved_inline_terminal: None,
                    mode: InlineTerminalMode::FixedFallback,
                    surface_mode: TerminalSurfaceMode::Inline,
                    viewport: area,
                    mouse_capture: MouseCaptureMode::Off,
                    terminal_size: (columns, rows),
                    bottom_pinned: area.bottom() >= rows,
                })
            }
        }
    }

    pub fn terminal_mut(&mut self) -> &mut DefaultTerminal {
        &mut self.terminal
    }

    pub fn mode(&self) -> InlineTerminalMode {
        self.mode
    }

    pub fn height(&self) -> u16 {
        self.viewport.height
    }

    pub fn viewport(&self) -> Rect {
        self.viewport
    }

    pub fn bottom_pinned(&self) -> bool {
        self.bottom_pinned
    }

    pub fn surface_mode(&self) -> TerminalSurfaceMode {
        self.surface_mode
    }

    pub fn set_surface_mode(&mut self, surface_mode: TerminalSurfaceMode) -> io::Result<()> {
        if surface_mode == self.surface_mode {
            return Ok(());
        }
        match surface_mode {
            TerminalSurfaceMode::Alternate => {
                self.refresh_viewport();
                execute!(io::stdout(), EnterAlternateScreen)?;
                let fullscreen = match Terminal::with_options(
                    CrosstermBackend::new(io::stdout()),
                    TerminalOptions {
                        viewport: Viewport::Fullscreen,
                    },
                ) {
                    Ok(terminal) => terminal,
                    Err(error) => {
                        let _ = execute!(io::stdout(), LeaveAlternateScreen);
                        return Err(error);
                    }
                };
                let inline = std::mem::replace(&mut self.terminal, fullscreen);
                self.saved_inline_terminal = Some(inline);
                self.surface_mode = TerminalSurfaceMode::Alternate;
                self.viewport = self.terminal.get_frame().area();
            }
            TerminalSurfaceMode::Inline => {
                let inline = self.saved_inline_terminal.take().ok_or_else(|| {
                    io::Error::other(
                        "inline terminal was not saved before leaving alternate screen",
                    )
                })?;
                let fullscreen = std::mem::replace(&mut self.terminal, inline);
                execute!(io::stdout(), LeaveAlternateScreen)?;
                drop(fullscreen);
                self.surface_mode = TerminalSurfaceMode::Inline;
                self.terminal.autoresize()?;
                self.terminal_size = crossterm::terminal::size()?;
                self.refresh_viewport();
            }
        }
        Ok(())
    }

    pub fn refresh_viewport(&mut self) {
        self.viewport = self.terminal.get_frame().area();
        if self.surface_mode == TerminalSurfaceMode::Inline {
            self.bottom_pinned = self.viewport.bottom() >= self.terminal_size.1;
        }
    }

    pub fn resize_height(
        &mut self,
        desired_height: u16,
        terminal_size: (u16, u16),
        anchor: InlineViewportAnchor,
        preserve_released_top: bool,
    ) -> io::Result<()> {
        let (columns, rows) = terminal_size;
        let desired_height = desired_height.clamp(1, rows.max(1));
        let size_changed = self.terminal_size != (columns, rows);
        if self.surface_mode == TerminalSurfaceMode::Alternate {
            let area = Rect::new(0, 0, columns.max(1), rows.max(1));
            if self.viewport != area || size_changed {
                self.terminal.resize(area)?;
                self.viewport = area;
                self.terminal_size = (columns, rows);
            }
            return Ok(());
        }
        if desired_height == self.viewport.height && !size_changed {
            return Ok(());
        }
        let anchor = if self.bottom_pinned {
            InlineViewportAnchor::Bottom
        } else {
            anchor
        };
        let resize = viewport_resize(self.viewport, (columns, rows), desired_height, anchor);
        let mut output = io::stdout();
        let clear_area = if preserve_released_top
            && anchor == InlineViewportAnchor::Bottom
            && resize.viewport.height < self.viewport.height
        {
            resize.viewport
        } else if resize.scroll_from_current_anchor {
            self.viewport
        } else {
            resize.clear_area
        };
        clear_rows(&mut output, clear_area)?;
        let initialization_row = match anchor {
            InlineViewportAnchor::Bottom => resize.viewport.y,
            InlineViewportAnchor::Top if resize.scroll_from_current_anchor => self.viewport.y,
            InlineViewportAnchor::Top => resize.viewport.y,
        };
        queue!(output, MoveTo(resize.viewport.x, initialization_row))?;
        output.flush()?;

        if self.mode == InlineTerminalMode::FixedFallback {
            self.terminal.resize(resize.viewport)?;
            self.viewport = resize.viewport;
            self.terminal_size = (columns, rows);
            self.bottom_pinned = self.viewport.bottom() >= rows;
            return Ok(());
        }

        match Terminal::with_options(
            CrosstermBackend::new(io::stdout()),
            TerminalOptions {
                viewport: Viewport::Inline(desired_height),
            },
        ) {
            Ok(terminal) => {
                self.terminal = terminal;
                self.viewport = self.terminal.get_frame().area();
                self.terminal_size = (columns, rows);
                self.bottom_pinned = self.viewport.bottom() >= rows;
                Ok(())
            }
            Err(error) => {
                let fallback = Terminal::with_options(
                    CrosstermBackend::new(io::stdout()),
                    TerminalOptions {
                        viewport: Viewport::Fixed(resize.viewport),
                    },
                )?;
                self.terminal = fallback;
                self.mode = InlineTerminalMode::FixedFallback;
                self.viewport = resize.viewport;
                self.terminal_size = (columns, rows);
                self.bottom_pinned = self.viewport.bottom() >= rows;
                eprintln!("warning: dynamic inline resize failed ({error}); using fixed viewport");
                Ok(())
            }
        }
    }

    pub fn update_anchor(&mut self, viewport: Rect) {
        self.viewport = viewport;
        self.bottom_pinned = viewport.bottom() >= self.terminal_size.1;
    }

    pub fn set_mouse_capture(&mut self, capture: MouseCaptureMode) -> io::Result<()> {
        if capture == self.mouse_capture {
            return Ok(());
        }
        match capture {
            MouseCaptureMode::Off => execute!(io::stdout(), DisableMouseCapture)?,
            MouseCaptureMode::Surface => execute!(io::stdout(), EnableMouseCapture)?,
        }
        self.mouse_capture = capture;
        Ok(())
    }

    pub fn begin_update(&self) -> io::Result<SynchronizedUpdateGuard> {
        SynchronizedUpdateGuard::begin()
    }

    pub fn finish_inline(&mut self) -> io::Result<()> {
        self.set_mouse_capture(MouseCaptureMode::Off)?;
        if self.surface_mode == TerminalSurfaceMode::Alternate {
            self.set_surface_mode(TerminalSurfaceMode::Inline)?;
        }
        let viewport = self.terminal.get_frame().area();
        self.update_anchor(viewport);
        self.terminal.clear()?;
        execute!(
            io::stdout(),
            MoveTo(0, self.viewport.y),
            Clear(ClearType::FromCursorDown)
        )
    }
}

impl Drop for InlineTerminal {
    fn drop(&mut self) {
        if self.mouse_capture == MouseCaptureMode::Surface {
            let _ = execute!(io::stdout(), DisableMouseCapture);
            self.mouse_capture = MouseCaptureMode::Off;
        }
        if self.surface_mode == TerminalSurfaceMode::Alternate {
            let _ = execute!(io::stdout(), LeaveAlternateScreen);
            self.surface_mode = TerminalSurfaceMode::Inline;
        }
    }
}

pub struct SynchronizedUpdateGuard {
    active: bool,
}

impl SynchronizedUpdateGuard {
    fn begin() -> io::Result<Self> {
        execute!(io::stdout(), BeginSynchronizedUpdate)?;
        Ok(Self { active: true })
    }

    pub fn finish(mut self) -> io::Result<()> {
        self.active = false;
        execute!(io::stdout(), EndSynchronizedUpdate)
    }
}

impl Drop for SynchronizedUpdateGuard {
    fn drop(&mut self) {
        if self.active {
            let _ = execute!(io::stdout(), EndSynchronizedUpdate);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_are_explicit_and_comparable() {
        assert_ne!(
            InlineTerminalMode::Dynamic,
            InlineTerminalMode::FixedFallback
        );
    }

    #[test]
    fn viewport_grows_downward_until_it_reaches_the_terminal_bottom() {
        let current = Rect::new(0, 6, 80, 4);
        let growing = viewport_resize(current, (80, 24), 8, InlineViewportAnchor::Top);
        assert_eq!(growing.viewport, Rect::new(0, 6, 80, 8));
        assert_eq!(growing.clear_area, Rect::new(0, 6, 80, 8));

        let touching = viewport_resize(
            Rect::new(0, 16, 80, 4),
            (80, 24),
            8,
            InlineViewportAnchor::Top,
        );
        assert_eq!(touching.viewport, Rect::new(0, 16, 80, 8));
    }

    #[test]
    fn viewport_scrolls_from_its_current_anchor_when_growth_would_cross_the_bottom() {
        let resize = viewport_resize(
            Rect::new(0, 18, 80, 4),
            (80, 24),
            8,
            InlineViewportAnchor::Top,
        );
        assert_eq!(resize.viewport, Rect::new(0, 16, 80, 8));
        assert!(resize.scroll_from_current_anchor);
        assert_eq!(resize.clear_area, Rect::new(0, 16, 80, 8));
    }

    #[test]
    fn viewport_shrink_keeps_its_top_and_releases_rows_below_the_ui() {
        let grown = viewport_resize(
            Rect::new(0, 14, 80, 10),
            (80, 24),
            10,
            InlineViewportAnchor::Top,
        );
        assert_eq!(grown.viewport, Rect::new(0, 14, 80, 10));

        let shrunk = viewport_resize(grown.viewport, (80, 24), 5, InlineViewportAnchor::Top);
        assert_eq!(shrunk.viewport, Rect::new(0, 14, 80, 5));
        assert_eq!(shrunk.clear_area, Rect::new(0, 14, 80, 10));
    }

    #[test]
    fn unpinned_viewport_preserves_its_top_across_physical_resize() {
        let resize = viewport_resize(
            Rect::new(0, 7, 80, 5),
            (100, 30),
            5,
            InlineViewportAnchor::Top,
        );
        assert_eq!(resize.viewport, Rect::new(0, 7, 100, 5));
    }

    #[test]
    fn command_menu_resizes_preserve_the_viewport_bottom() {
        let base = Rect::new(0, 12, 80, 5);
        let expanded = viewport_resize(base, (80, 24), 10, InlineViewportAnchor::Bottom);
        assert_eq!(expanded.viewport, Rect::new(0, 7, 80, 10));
        assert!(!expanded.scroll_from_current_anchor);

        let collapsed =
            viewport_resize(expanded.viewport, (80, 24), 5, InlineViewportAnchor::Bottom);
        assert_eq!(collapsed.viewport, base);
    }

    #[test]
    fn bottom_anchored_growth_uses_available_rows_before_moving_the_composer() {
        let expanded = viewport_resize(
            Rect::new(0, 2, 80, 4),
            (80, 24),
            8,
            InlineViewportAnchor::Bottom,
        );
        assert_eq!(expanded.viewport, Rect::new(0, 0, 80, 8));
    }
}
