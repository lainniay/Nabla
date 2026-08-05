use std::{
    env,
    io::{self, Stdout, Write},
};

use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{DisableMouseCapture, EnableMouseCapture},
    queue,
    style::{
        Attribute, Color as CrosstermColor, ResetColor, SetAttribute, SetBackgroundColor,
        SetForegroundColor,
    },
    terminal::{
        BeginSynchronizedUpdate, Clear, ClearType, EndSynchronizedUpdate, EnterAlternateScreen,
        LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    },
};

use super::{
    palette,
    transcript::TranscriptStore,
    types::{
        CanonicalReflowProjection, CellStyle, Color, CommittedHistoryBlock, CursorPosition,
        FrameUpdate, PanelFrame, Rect, StyledCell, SurfaceKind, TerminalCommitPlan, TerminalSize,
        VisualFrame, VisualRow,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalCapabilities {
    pub synchronized_output: bool,
    pub true_color: bool,
    pub mouse: bool,
}

impl TerminalCapabilities {
    pub fn detect() -> Self {
        let term = env::var("TERM").unwrap_or_default();
        let color_term = env::var("COLORTERM").unwrap_or_default();
        Self {
            // Unknown CSI private modes are ignored by compatible terminals,
            // but avoid emitting them to explicitly minimal terminals.
            synchronized_output: !matches!(term.as_str(), "" | "dumb"),
            true_color: color_term.contains("truecolor")
                || color_term.contains("24bit")
                || env::var_os("KITTY_WINDOW_ID").is_some()
                || env::var_os("WEZTERM_PANE").is_some(),
            mouse: !matches!(term.as_str(), "" | "dumb"),
        }
    }
}

pub struct TerminalDriver<W: Write> {
    output: W,
    capabilities: TerminalCapabilities,
    surface: SurfaceKind,
    size: TerminalSize,
    primary_history_window: Option<Rect>,
    primary_screen: Vec<VisualRow>,
    active_panel: Option<Rect>,
    owned_footer_height: u16,
    claimed_primary: bool,
    mouse_enabled: bool,
    pending_wrap: bool,
    physical_cursor: Option<CursorPosition>,
    physical_valid: bool,
}

impl TerminalDriver<Stdout> {
    pub fn open(size: TerminalSize) -> io::Result<Self> {
        enable_raw_mode()?;
        let mut driver = Self::new(io::stdout(), TerminalCapabilities::detect(), size);
        if let Err(error) = driver.claim_primary_surface() {
            let _ = disable_raw_mode();
            return Err(error);
        }
        Ok(driver)
    }

    pub fn finish(&mut self) -> io::Result<()> {
        let terminal_result = self.finish_terminal();
        let raw_result = disable_raw_mode();
        terminal_result.and(raw_result)
    }
}

impl<W: Write> TerminalDriver<W> {
    pub fn new(output: W, capabilities: TerminalCapabilities, size: TerminalSize) -> Self {
        let primary_screen = blank_screen(size);
        Self {
            output,
            capabilities,
            surface: SurfaceKind::Primary,
            size,
            primary_history_window: None,
            primary_screen,
            active_panel: None,
            owned_footer_height: 2,
            claimed_primary: false,
            mouse_enabled: false,
            pending_wrap: false,
            physical_cursor: None,
            physical_valid: true,
        }
    }

    pub fn capabilities(&self) -> TerminalCapabilities {
        self.capabilities
    }

    pub fn surface(&self) -> SurfaceKind {
        self.surface
    }

    pub fn output_ref(&self) -> &W {
        &self.output
    }

    pub fn physical_valid(&self) -> bool {
        self.physical_valid
    }

    /// Makes the full visible primary screen application-owned by scrolling
    /// existing shell content upward. It never sends ED 3 / scrollback purge.
    pub fn claim_primary_surface(&mut self) -> io::Result<()> {
        if self.claimed_primary {
            return Ok(());
        }
        queue!(self.output, MoveTo(0, self.size.height.saturating_sub(1)))?;
        for _ in 0..self.size.height {
            self.output.write_all(b"\r\n")?;
        }
        queue!(self.output, MoveTo(0, 0), Hide)?;
        self.output.flush()?;
        self.primary_screen = blank_screen(self.size);
        self.active_panel = None;
        self.claimed_primary = true;
        Ok(())
    }

    pub fn set_mouse_capture(&mut self, enabled: bool) -> io::Result<()> {
        if enabled == self.mouse_enabled || !self.capabilities.mouse {
            return Ok(());
        }
        if enabled {
            queue!(self.output, EnableMouseCapture)?;
        } else {
            queue!(self.output, DisableMouseCapture)?;
        }
        self.output.flush()?;
        self.mouse_enabled = enabled;
        Ok(())
    }

    pub fn commit(&mut self, plan: &TerminalCommitPlan) -> io::Result<()> {
        self.commit_internal(plan, false)
    }

    pub fn commit_resize_reflow(&mut self, plan: &TerminalCommitPlan) -> io::Result<()> {
        if plan.surface != SurfaceKind::Primary {
            return Err(io::Error::other(
                "destructive resize reflow requires the primary surface",
            ));
        }
        self.commit_internal(plan, true)
    }

    pub fn clear_scrollback_and_visible_screen(&mut self) -> io::Result<()> {
        self.switch_surface(SurfaceKind::Primary)?;
        let synchronized = self.capabilities.synchronized_output;
        let begin_result = if synchronized {
            queue!(self.output, BeginSynchronizedUpdate)
        } else {
            Ok(())
        };
        let result = begin_result.and_then(|()| self.stage_destructive_reset());
        let end_result = if synchronized {
            queue!(self.output, EndSynchronizedUpdate)
        } else {
            Ok(())
        };
        let flush_result = self.output.flush();
        let result = result.and(end_result).and(flush_result);
        match result {
            Ok(()) => {
                self.reset_physical_projection_state();
                self.physical_valid = true;
                Ok(())
            }
            Err(error) => {
                self.physical_valid = false;
                Err(error)
            }
        }
    }

    pub fn begin_canonical_reflow(&mut self) -> io::Result<()> {
        self.clear_scrollback_and_visible_screen()?;
        // The screen is intentionally incomplete until every canonical batch
        // and the resident frame have been committed.
        self.physical_valid = false;
        Ok(())
    }

    pub fn replay_canonical_history_batch(
        &mut self,
        blocks: &[CommittedHistoryBlock],
    ) -> io::Result<()> {
        self.switch_surface(SurfaceKind::Primary)?;
        let synchronized = self.capabilities.synchronized_output;
        let begin_result = if synchronized {
            queue!(self.output, BeginSynchronizedUpdate)
        } else {
            Ok(())
        };
        let result = begin_result.and_then(|()| {
            let rows = blocks
                .iter()
                .flat_map(|block| block.rows.iter())
                .collect::<Vec<_>>();
            self.append_history_fullscreen(&rows)
        });
        let end_result = if synchronized {
            queue!(self.output, EndSynchronizedUpdate)
        } else {
            Ok(())
        };
        let flush_result = self.output.flush();
        let result = result.and(end_result).and(flush_result);
        if let Err(error) = result {
            self.physical_valid = false;
            return Err(error);
        }
        // A successful batch is still not a complete physical projection.
        self.physical_valid = false;
        Ok(())
    }

    pub fn commit_canonical_reflow_frame(&mut self, plan: &TerminalCommitPlan) -> io::Result<()> {
        if plan.surface != SurfaceKind::Primary || !plan.overflow_blocks.is_empty() {
            return Err(io::Error::other(
                "canonical reflow finalization requires a history-free primary frame",
            ));
        }
        self.commit_internal(plan, false)
    }

    fn commit_internal(
        &mut self,
        plan: &TerminalCommitPlan,
        destructive_reflow: bool,
    ) -> io::Result<()> {
        self.switch_surface(plan.surface)?;
        let previous_screen = self.primary_screen.clone();
        let previous_panel = self.active_panel;
        let previous_size = self.size;
        let previous_history_window = self.primary_history_window;
        let previous_footer_height = self.owned_footer_height;
        let previous_pending_wrap = self.pending_wrap;
        let previous_physical_cursor = self.physical_cursor;
        if let FrameUpdate::Full(frame) = &plan.frame_update {
            self.resize_primary_screen(frame.terminal_size);
            if plan.surface == SurfaceKind::Primary {
                self.owned_footer_height = frame
                    .main_layout
                    .composer
                    .height
                    .saturating_add(frame.main_layout.status.height)
                    .min(self.size.height);
            }
        }
        let synchronized = self.capabilities.synchronized_output;
        let begin_result = if synchronized {
            queue!(self.output, BeginSynchronizedUpdate)
        } else {
            Ok(())
        };

        let result: io::Result<()> = begin_result.and_then(|()| {
            if plan.surface == SurfaceKind::Primary {
                if destructive_reflow {
                    self.stage_destructive_reset()?;
                    self.reset_physical_projection_state();
                    if let FrameUpdate::Full(frame) = &plan.frame_update {
                        self.owned_footer_height = frame
                            .main_layout
                            .composer
                            .height
                            .saturating_add(frame.main_layout.status.height)
                            .min(self.size.height);
                    }
                    let rows = plan
                        .overflow_blocks
                        .iter()
                        .flat_map(|block| block.rows.iter())
                        .collect::<Vec<_>>();
                    self.append_history_fullscreen(&rows)?;
                    self.primary_history_window =
                        Some(normalize_history_window(plan.history_window, self.size));
                } else {
                    self.restore_active_panel()?;
                    self.primary_history_window =
                        Some(normalize_history_window(plan.history_window, self.size));
                    if plan.bootstrap_scroll_rows > 0 || !plan.overflow_blocks.is_empty() {
                        self.overflow_primary_history(plan)?;
                    }
                }
            }
            match &plan.frame_update {
                FrameUpdate::Full(frame) => self.draw_full(frame)?,
                FrameUpdate::Rows { rows, .. } => {
                    for (terminal_row, row) in rows {
                        if self.surface == SurfaceKind::Primary {
                            self.draw_base_row(*terminal_row, row)?;
                        } else {
                            self.draw_row(*terminal_row, row)?;
                        }
                    }
                }
            }
            if plan.surface == SurfaceKind::Primary
                && let Some(panel) = plan.panel.as_ref()
            {
                self.draw_panel(panel)?;
            }
            queue!(self.output, ResetColor, SetAttribute(Attribute::Reset))?;
            if let Some(cursor) = plan.cursor {
                queue!(self.output, MoveTo(cursor.column, cursor.row), Show)?;
            } else {
                queue!(self.output, Hide)?;
            }
            Ok(())
        });

        // Always terminate synchronized mode and flush even after a staged
        // write fails. `and` preserves the original error when cleanup also
        // fails.
        let end_result = if synchronized {
            queue!(self.output, EndSynchronizedUpdate)
        } else {
            Ok(())
        };
        let flush_result = self.output.flush();
        let result = result.and(end_result).and(flush_result);
        match result {
            Ok(()) => {
                if plan.surface == SurfaceKind::Primary {
                    self.active_panel = plan.panel.as_ref().map(|panel| panel.area);
                }
                self.pending_wrap = false;
                self.physical_cursor = plan.cursor;
                self.physical_valid = true;
                Ok(())
            }
            Err(error) => {
                self.primary_screen = previous_screen;
                self.active_panel = previous_panel;
                self.size = previous_size;
                self.primary_history_window = previous_history_window;
                self.owned_footer_height = previous_footer_height;
                self.pending_wrap = previous_pending_wrap;
                self.physical_cursor = previous_physical_cursor;
                self.physical_valid = false;
                Err(error)
            }
        }
    }

    fn stage_destructive_reset(&mut self) -> io::Result<()> {
        self.reset_scroll_region()?;
        queue!(
            self.output,
            ResetColor,
            SetAttribute(Attribute::Reset),
            MoveTo(0, 0),
            Clear(ClearType::All),
            Clear(ClearType::Purge),
            MoveTo(0, 0),
            Hide
        )
    }

    fn reset_physical_projection_state(&mut self) {
        self.primary_screen = blank_screen(self.size);
        self.primary_history_window = None;
        self.active_panel = None;
        self.owned_footer_height = 0;
        self.pending_wrap = false;
        self.physical_cursor = None;
    }

    fn overflow_primary_history(&mut self, plan: &TerminalCommitPlan) -> io::Result<()> {
        let history_window = normalize_history_window(plan.history_window, self.size);
        if history_window.height == 0 {
            return Ok(());
        }
        self.set_scroll_region(history_window)?;
        let result = (|| {
            for _ in 0..plan.bootstrap_scroll_rows {
                queue!(
                    self.output,
                    MoveTo(0, history_window.bottom().saturating_sub(1)),
                    ResetColor,
                    SetAttribute(Attribute::Reset)
                )?;
                self.output.write_all(b"\r\n")?;
                self.scroll_primary_screen_up(history_window, 1);
            }
            for row in plan
                .overflow_blocks
                .iter()
                .flat_map(|block| block.rows.iter())
            {
                self.draw_base_row(history_window.y, row)?;
                queue!(
                    self.output,
                    MoveTo(0, history_window.bottom().saturating_sub(1)),
                    ResetColor,
                    SetAttribute(Attribute::Reset)
                )?;
                self.output.write_all(b"\r\n")?;
                self.scroll_primary_screen_up(history_window, 1);
            }
            Ok(())
        })();
        let reset = self.reset_scroll_region();
        result.and(reset)
    }

    fn append_history_fullscreen(&mut self, rows: &[&VisualRow]) -> io::Result<()> {
        for chunk in rows.chunks(usize::from(self.size.height.max(1))) {
            for terminal_row in 0..self.size.height {
                let index = usize::from(terminal_row);
                if let Some(row) = chunk.get(index) {
                    self.draw_base_row(terminal_row, row)?;
                } else {
                    self.clear_base_row(terminal_row)?;
                }
            }
            queue!(self.output, MoveTo(0, self.size.height.saturating_sub(1)))?;
            for _ in 0..chunk.len() {
                self.output.write_all(b"\r\n")?;
                self.scroll_primary_screen_up(
                    Rect::new(0, 0, self.size.width, self.size.height),
                    1,
                );
            }
        }
        Ok(())
    }

    fn set_scroll_region(&mut self, area: Rect) -> io::Result<()> {
        write!(
            self.output,
            "\u{1b}[{};{}r",
            area.y.saturating_add(1),
            area.bottom().max(area.y.saturating_add(1))
        )
    }

    fn reset_scroll_region(&mut self) -> io::Result<()> {
        self.output.write_all(b"\x1b[r")
    }

    fn resize_primary_screen(&mut self, next_size: TerminalSize) {
        if next_size == self.size {
            return;
        }
        let previous_size = self.size;
        let previous = std::mem::take(&mut self.primary_screen);
        let mut next = blank_screen(next_size);
        let copied_rows = usize::from(previous_size.height.min(next_size.height));
        let previous_start = previous.len().saturating_sub(copied_rows);
        let next_start = next.len().saturating_sub(copied_rows);
        next[next_start..next_start + copied_rows]
            .clone_from_slice(&previous[previous_start..previous_start + copied_rows]);
        self.primary_screen = next;
        self.active_panel = self
            .active_panel
            .and_then(|area| translate_bottom_aligned(area, previous_size, next_size));
        self.size = next_size;
    }

    fn restore_active_panel(&mut self) -> io::Result<()> {
        let Some(area) = self.active_panel else {
            return Ok(());
        };
        for terminal_row in area.y..area.bottom().min(self.size.height) {
            let row = self
                .primary_screen
                .get(usize::from(terminal_row))
                .cloned()
                .unwrap_or_else(|| VisualRow::blank("surface"));
            self.draw_row(terminal_row, &row)?;
        }
        Ok(())
    }

    fn draw_panel(&mut self, panel: &PanelFrame) -> io::Result<()> {
        for terminal_row in panel.area.y..panel.area.bottom().min(self.size.height) {
            let offset = usize::from(terminal_row.saturating_sub(panel.area.y));
            let row = panel
                .rows
                .get(offset)
                .cloned()
                .unwrap_or_else(|| VisualRow::blank("panel"));
            self.draw_row(terminal_row, &row)?;
        }
        Ok(())
    }

    fn draw_full(&mut self, frame: &VisualFrame) -> io::Result<()> {
        let viewport = Rect::new(
            0,
            frame.viewport.y.min(self.size.height),
            self.size.width,
            frame
                .viewport
                .height
                .min(self.size.height.saturating_sub(frame.viewport.y)),
        );
        for terminal_row in viewport.y..viewport.bottom() {
            let row = frame
                .rows
                .get(usize::from(terminal_row))
                .cloned()
                .unwrap_or_else(|| VisualRow::blank("surface"));
            if self.surface == SurfaceKind::Primary {
                self.draw_base_row(terminal_row, &row)?;
            } else {
                self.draw_row(terminal_row, &row)?;
            }
        }
        Ok(())
    }

    fn draw_base_row(&mut self, terminal_row: u16, row: &VisualRow) -> io::Result<()> {
        if let Some(target) = self.primary_screen.get_mut(usize::from(terminal_row)) {
            *target = row.clone();
        }
        self.draw_row(terminal_row, row)
    }

    fn clear_base_row(&mut self, terminal_row: u16) -> io::Result<()> {
        let row = VisualRow::blank("surface");
        self.draw_base_row(terminal_row, &row)
    }

    fn scroll_primary_screen_up(&mut self, area: Rect, rows: u16) {
        let top = usize::from(area.y.min(self.size.height));
        let bottom = usize::from(area.bottom().min(self.size.height));
        for _ in 0..rows {
            if top >= bottom || self.primary_screen.len() < bottom {
                return;
            }
            for index in top..bottom.saturating_sub(1) {
                self.primary_screen[index] = self.primary_screen[index + 1].clone();
            }
            self.primary_screen[bottom.saturating_sub(1)] = VisualRow::blank("surface");
        }
    }

    fn draw_row(&mut self, terminal_row: u16, row: &VisualRow) -> io::Result<()> {
        queue!(
            self.output,
            MoveTo(0, terminal_row),
            ResetColor,
            SetAttribute(Attribute::Reset),
            Clear(ClearType::CurrentLine)
        )?;
        let mut column = 0u16;
        let mut previous_style = None;
        for cell in &row.cells {
            if column.saturating_add(cell.width) > self.size.width {
                break;
            }
            if previous_style != Some(cell.style) {
                self.write_style(cell.style)?;
                previous_style = Some(cell.style);
            }
            self.write_cell(cell)?;
            column = column.saturating_add(cell.width);
        }
        if column == self.size.width && self.size.width > 0 {
            // Writing the last terminal column leaves many terminals in a
            // pending-wrap state. An explicit cursor move cancels that state,
            // so the next history CRLF advances exactly one row.
            queue!(self.output, MoveTo(0, terminal_row))?;
        }
        Ok(())
    }

    fn write_cell(&mut self, cell: &StyledCell) -> io::Result<()> {
        self.output.write_all(cell.symbol.as_bytes())
    }

    fn write_style(&mut self, style: CellStyle) -> io::Result<()> {
        queue!(
            self.output,
            ResetColor,
            SetAttribute(Attribute::Reset),
            SetForegroundColor(map_color(style.foreground, self.capabilities.true_color)),
            SetBackgroundColor(map_color(style.background, self.capabilities.true_color))
        )?;
        if style.bold {
            queue!(self.output, SetAttribute(Attribute::Bold))?;
        }
        // Mocha's Subtext and Overlay shades already encode visual hierarchy.
        // ANSI Dim makes those colors illegible on many dark terminals, so the
        // semantic flag remains available to layout/tests but is not emitted.
        if style.italic {
            queue!(self.output, SetAttribute(Attribute::Italic))?;
        }
        if style.underlined {
            queue!(self.output, SetAttribute(Attribute::Underlined))?;
        }
        if style.crossed_out {
            queue!(self.output, SetAttribute(Attribute::CrossedOut))?;
        }
        if style.reversed {
            queue!(self.output, SetAttribute(Attribute::Reverse))?;
        }
        Ok(())
    }

    fn switch_surface(&mut self, surface: SurfaceKind) -> io::Result<()> {
        if surface == self.surface {
            return Ok(());
        }
        match surface {
            SurfaceKind::Alternate => queue!(self.output, EnterAlternateScreen, Hide)?,
            SurfaceKind::Primary => queue!(self.output, LeaveAlternateScreen, Hide)?,
        }
        self.surface = surface;
        Ok(())
    }

    fn finish_terminal(&mut self) -> io::Result<()> {
        if self.mouse_enabled {
            queue!(self.output, DisableMouseCapture)?;
            self.mouse_enabled = false;
        }
        if self.surface == SurfaceKind::Alternate {
            queue!(self.output, LeaveAlternateScreen)?;
            self.surface = SurfaceKind::Primary;
        }
        self.restore_active_panel()?;
        self.active_panel = None;
        // Remove only the application-owned composer/status rows. Never issue
        // Clear(All) or Clear(Purge).
        for row in self.size.height.saturating_sub(self.owned_footer_height)..self.size.height {
            queue!(self.output, MoveTo(0, row), Clear(ClearType::CurrentLine))?;
        }
        queue!(
            self.output,
            MoveTo(0, self.size.height.saturating_sub(1)),
            ResetColor,
            SetAttribute(Attribute::Reset),
            Show
        )?;
        self.output.write_all(b"\r\n")?;
        self.output.flush()
    }
}

impl<W: Write> Drop for TerminalDriver<W> {
    fn drop(&mut self) {
        if self.mouse_enabled {
            let _ = queue!(self.output, DisableMouseCapture);
        }
        if self.surface == SurfaceKind::Alternate {
            let _ = queue!(self.output, LeaveAlternateScreen);
        }
        let _ = queue!(
            self.output,
            ResetColor,
            SetAttribute(Attribute::Reset),
            Show
        );
        let _ = self.output.flush();
    }
}

fn blank_screen(size: TerminalSize) -> Vec<VisualRow> {
    (0..size.height)
        .map(|_| VisualRow::blank("surface"))
        .collect()
}

fn normalize_history_window(window: Rect, size: TerminalSize) -> Rect {
    let y = window.y.min(size.height);
    Rect::new(
        0,
        y,
        size.width,
        window.height.min(size.height.saturating_sub(y)),
    )
}

fn translate_bottom_aligned(
    area: Rect,
    previous_size: TerminalSize,
    next_size: TerminalSize,
) -> Option<Rect> {
    let distance_from_bottom = previous_size.height.saturating_sub(area.bottom());
    let bottom = next_size.height.saturating_sub(distance_from_bottom);
    let height = area.height.min(bottom);
    (height > 0).then_some(Rect::new(
        0,
        bottom.saturating_sub(height),
        next_size.width,
        height,
    ))
}

fn map_color(color: Color, true_color: bool) -> CrosstermColor {
    let resolved = match color {
        Color::Default => return CrosstermColor::Reset,
        Color::Black => palette::CRUST,
        Color::Red => palette::RED,
        Color::Green => palette::GREEN,
        Color::Yellow => palette::YELLOW,
        Color::Blue => palette::BLUE,
        Color::Magenta => palette::MAUVE,
        Color::Cyan => palette::SAPPHIRE,
        Color::White => palette::TEXT,
        Color::Gray => palette::GRAY_MUTED,
        color @ Color::Rgb(_, _, _) => color,
    };
    let Color::Rgb(red, green, blue) = resolved else {
        unreachable!("named colors resolve to a Catppuccin RGB constant")
    };
    if true_color {
        CrosstermColor::Rgb {
            r: red,
            g: green,
            b: blue,
        }
    } else {
        nearest_ansi(red, green, blue)
    }
}

fn nearest_ansi(red: u8, green: u8, blue: u8) -> CrosstermColor {
    if red.max(green).max(blue) < 96 {
        CrosstermColor::Black
    } else if red > green.saturating_add(32) && red > blue.saturating_add(32) {
        CrosstermColor::Red
    } else if green > red.saturating_add(32) && green > blue.saturating_add(32) {
        CrosstermColor::Green
    } else if blue > red.saturating_add(32) && blue > green.saturating_add(32) {
        CrosstermColor::Blue
    } else if red > 160 && green > 160 {
        CrosstermColor::Yellow
    } else if red > 140 && blue > 140 {
        CrosstermColor::Magenta
    } else if green > 140 && blue > 140 {
        CrosstermColor::Cyan
    } else {
        CrosstermColor::White
    }
}

#[derive(Debug, Default)]
pub struct FrameCoordinator {
    pub committed_revision: u64,
    pub previous_frame: Option<VisualFrame>,
    pub terminal_invalid: bool,
    previous_surface: SurfaceKind,
    previous_bootstrap_padding_rows: Option<usize>,
}

impl FrameCoordinator {
    pub fn plan(
        &self,
        frame: VisualFrame,
        surface: SurfaceKind,
        projection: Option<super::types::PrimaryTranscriptProjection>,
    ) -> TerminalCommitPlan {
        let (overflow_blocks, bootstrap_padding_rows) = projection.map_or_else(
            || (Vec::new(), 0),
            |projection| {
                (
                    projection.overflow_blocks,
                    projection.bootstrap_padding_rows,
                )
            },
        );
        let overflow_rows = overflow_blocks
            .iter()
            .map(|block| block.rows.len())
            .sum::<usize>();
        let full_redraw = self.terminal_invalid
            || self.previous_surface != surface
            || overflow_rows > 0
            || self.previous_frame.as_ref().is_none_or(|previous| {
                previous.terminal_size != frame.terminal_size
                    || previous.viewport != frame.viewport
                    || previous.main_layout != frame.main_layout
                    || previous.component_bounds != frame.component_bounds
                    || previous.hit_regions != frame.hit_regions
            });
        let cursor = frame.cursor;
        let panel = frame.panel.clone();
        let history_window = frame.main_layout.history_window;
        let geometry_unchanged = self.previous_surface == surface
            && self.previous_frame.as_ref().is_some_and(|previous| {
                previous.terminal_size == frame.terminal_size
                    && previous.main_layout.history_window == history_window
            });
        let bootstrap_scroll_rows = if surface == SurfaceKind::Primary && geometry_unchanged {
            self.previous_bootstrap_padding_rows
                .unwrap_or(bootstrap_padding_rows)
                .saturating_sub(bootstrap_padding_rows)
        } else {
            0
        };
        let revision = frame.revision;
        let frame_update = if full_redraw {
            FrameUpdate::Full(frame)
        } else {
            let previous = self.previous_frame.as_ref();
            let rows = frame
                .rows
                .iter()
                .enumerate()
                .filter(|(index, row)| {
                    previous.and_then(|previous| previous.rows.get(*index)) != Some(*row)
                })
                .map(|(index, row)| (index as u16, row.clone()))
                .collect();
            FrameUpdate::Rows { revision, rows }
        };
        TerminalCommitPlan {
            revision,
            surface,
            history_window,
            bootstrap_scroll_rows,
            bootstrap_padding_rows,
            overflow_blocks,
            frame_update,
            panel,
            cursor,
            full_redraw,
        }
    }

    pub fn commit<W: Write>(
        &mut self,
        driver: &mut TerminalDriver<W>,
        transcript: &mut TranscriptStore,
        plan: TerminalCommitPlan,
    ) -> io::Result<()> {
        let next_frame = match &plan.frame_update {
            FrameUpdate::Full(frame) => frame.clone(),
            FrameUpdate::Rows { revision, rows } => {
                let mut frame = self.previous_frame.clone().ok_or_else(|| {
                    io::Error::other("row update cannot be committed without a previous frame")
                })?;
                frame.revision = *revision;
                frame.cursor = plan.cursor;
                frame.panel = plan.panel.clone();
                for (terminal_row, row) in rows {
                    if let Some(target) = frame.rows.get_mut(usize::from(*terminal_row)) {
                        *target = row.clone();
                    }
                }
                frame
            }
        };
        match driver.commit(&plan) {
            Ok(()) => {
                transcript.acknowledge_overflow(&plan.overflow_blocks);
                self.committed_revision = plan.revision;
                self.previous_frame = Some(next_frame);
                self.previous_surface = plan.surface;
                self.previous_bootstrap_padding_rows =
                    (plan.surface == SurfaceKind::Primary).then_some(plan.bootstrap_padding_rows);
                self.terminal_invalid = false;
                Ok(())
            }
            Err(error) => {
                self.terminal_invalid = true;
                Err(error)
            }
        }
    }

    pub fn plan_resize_reflow(
        &self,
        frame: VisualFrame,
        projection: &CanonicalReflowProjection,
    ) -> TerminalCommitPlan {
        let cursor = frame.cursor;
        let panel = frame.panel.clone();
        let history_window = frame.main_layout.history_window;
        TerminalCommitPlan {
            revision: frame.revision,
            surface: SurfaceKind::Primary,
            history_window,
            bootstrap_scroll_rows: 0,
            bootstrap_padding_rows: 0,
            overflow_blocks: projection.history_blocks.clone(),
            frame_update: FrameUpdate::Full(frame),
            panel,
            cursor,
            full_redraw: true,
        }
    }

    pub fn plan_canonical_reflow_frame(
        &self,
        frame: VisualFrame,
        projection: &super::types::PrimaryTranscriptProjection,
    ) -> TerminalCommitPlan {
        let cursor = frame.cursor;
        let panel = frame.panel.clone();
        let history_window = frame.main_layout.history_window;
        TerminalCommitPlan {
            revision: frame.revision,
            surface: SurfaceKind::Primary,
            history_window,
            bootstrap_scroll_rows: 0,
            bootstrap_padding_rows: projection.bootstrap_padding_rows,
            overflow_blocks: Vec::new(),
            frame_update: FrameUpdate::Full(frame),
            panel,
            cursor,
            full_redraw: true,
        }
    }

    pub fn commit_resize_reflow<W: Write>(
        &mut self,
        driver: &mut TerminalDriver<W>,
        transcript: &mut TranscriptStore,
        plan: TerminalCommitPlan,
        projection: &CanonicalReflowProjection,
    ) -> io::Result<()> {
        let FrameUpdate::Full(next_frame) = &plan.frame_update else {
            return Err(io::Error::other(
                "resize reflow requires a complete canonical frame",
            ));
        };
        match driver.commit_resize_reflow(&plan) {
            Ok(()) if transcript.apply_reflow_projection(projection) => {
                self.committed_revision = plan.revision;
                self.previous_frame = Some(next_frame.clone());
                self.previous_surface = SurfaceKind::Primary;
                self.previous_bootstrap_padding_rows = Some(plan.bootstrap_padding_rows);
                self.terminal_invalid = false;
                Ok(())
            }
            Ok(()) => {
                self.terminal_invalid = true;
                Err(io::Error::other(
                    "canonical transcript changed during resize reflow",
                ))
            }
            Err(error) => {
                self.terminal_invalid = true;
                Err(error)
            }
        }
    }

    pub fn finish_canonical_reflow<W: Write>(
        &mut self,
        driver: &mut TerminalDriver<W>,
        transcript: &mut TranscriptStore,
        plan: TerminalCommitPlan,
        projection: &CanonicalReflowProjection,
    ) -> io::Result<()> {
        let FrameUpdate::Full(next_frame) = &plan.frame_update else {
            return Err(io::Error::other(
                "canonical reflow requires a complete resident frame",
            ));
        };
        match driver.commit_canonical_reflow_frame(&plan) {
            Ok(()) if transcript.apply_reflow_projection(projection) => {
                self.committed_revision = plan.revision;
                self.previous_frame = Some(next_frame.clone());
                self.previous_surface = SurfaceKind::Primary;
                self.previous_bootstrap_padding_rows = Some(plan.bootstrap_padding_rows);
                self.terminal_invalid = false;
                Ok(())
            }
            Ok(()) => {
                self.terminal_invalid = true;
                Err(io::Error::other(
                    "canonical transcript changed incompatibly during replay",
                ))
            }
            Err(error) => {
                self.terminal_invalid = true;
                Err(error)
            }
        }
    }

    pub fn invalidate(&mut self) {
        self.terminal_invalid = true;
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;
    use crate::ui::test_support::VirtualTerminal;
    use crate::ui::types::{MainLayout, TerminalSize};
    use crate::{
        rpc::PiState,
        state::{AppState, TranscriptItem},
    };

    fn frame(revision: u64, text: &str) -> VisualFrame {
        let history_window = Rect::new(0, 0, 20, 2);
        VisualFrame {
            revision,
            terminal_size: TerminalSize::new(20, 4),
            rows: vec![
                VisualRow {
                    component_id: "row".to_owned(),
                    logical_line: 0,
                    wrap_index: 0,
                    cells: vec![StyledCell::new(
                        text,
                        text.len() as u16,
                        CellStyle::default(),
                    )],
                },
                VisualRow::blank("surface"),
                VisualRow::blank("composer"),
                VisualRow::blank("status"),
            ],
            panel: None,
            viewport: crate::ui::types::Rect::new(0, 0, 20, 4),
            component_bounds: Default::default(),
            hit_regions: Vec::new(),
            cursor: None,
            main_layout: MainLayout {
                transcript: history_window,
                history_window,
                owned_surface: Rect::new(0, 0, 20, 4),
                panel: None,
                composer: Rect::new(0, 2, 20, 1),
                status: Rect::new(0, 3, 20, 1),
            },
        }
    }

    #[test]
    fn named_terminal_colors_resolve_to_the_application_theme() {
        assert_eq!(
            map_color(Color::White, true),
            CrosstermColor::Rgb {
                r: 205,
                g: 214,
                b: 244
            }
        );
        assert_eq!(
            map_color(Color::Gray, true),
            CrosstermColor::Rgb {
                r: 108,
                g: 112,
                b: 134
            }
        );
        assert_eq!(
            map_color(Color::Magenta, true),
            CrosstermColor::Rgb {
                r: 203,
                g: 166,
                b: 247
            }
        );
    }

    #[test]
    fn full_width_rows_cancel_the_terminal_pending_wrap_state() {
        let mut driver = TerminalDriver::new(
            Vec::<u8>::new(),
            TerminalCapabilities {
                synchronized_output: false,
                true_color: false,
                mouse: false,
            },
            TerminalSize::new(20, 4),
        );
        let row = VisualRow {
            component_id: "full-width".to_owned(),
            logical_line: 0,
            wrap_index: 0,
            cells: vec![StyledCell::new("x".repeat(20), 20, CellStyle::default())],
        };

        driver.draw_row(1, &row).unwrap();

        let output = String::from_utf8_lossy(driver.output_ref());
        assert!(
            output.ends_with("\u{1b}[2;1H"),
            "full-width row must explicitly cancel pending wrap: {output:?}"
        );
    }

    #[test]
    fn claiming_primary_scrolls_without_purging_scrollback() {
        let mut driver = TerminalDriver::new(
            Vec::<u8>::new(),
            TerminalCapabilities {
                synchronized_output: false,
                true_color: false,
                mouse: false,
            },
            TerminalSize::new(20, 4),
        );
        driver.claim_primary_surface().unwrap();
        let output = String::from_utf8_lossy(driver.output_ref());
        assert!(output.matches("\r\n").count() >= 4);
        assert!(!output.contains("\u{1b}[3J"));
    }

    #[test]
    fn failed_terminal_write_never_advances_the_previous_frame() {
        struct FailingWriter {
            remaining: usize,
        }
        impl Write for FailingWriter {
            fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
                if self.remaining == 0 {
                    return Err(io::Error::other("injected failure"));
                }
                let written = buffer.len().min(self.remaining);
                self.remaining -= written;
                Ok(written)
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let mut coordinator = FrameCoordinator::default();
        let plan = coordinator.plan(frame(1, "hello"), SurfaceKind::Primary, None);
        let mut driver = TerminalDriver::new(
            FailingWriter { remaining: 2 },
            TerminalCapabilities {
                synchronized_output: false,
                true_color: false,
                mouse: false,
            },
            TerminalSize::new(20, 4),
        );
        let mut transcript = TranscriptStore::default();
        assert!(
            coordinator
                .commit(&mut driver, &mut transcript, plan)
                .is_err()
        );
        assert_eq!(coordinator.committed_revision, 0);
        assert!(coordinator.previous_frame.is_none());
        assert!(coordinator.terminal_invalid);
    }

    #[test]
    fn failed_overflow_write_does_not_advance_the_physical_scrollback_cursor() {
        #[derive(Default)]
        struct FailOnScroll {
            output: Vec<u8>,
        }

        impl Write for FailOnScroll {
            fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
                if buffer.windows(2).any(|window| window == b"\r\n") {
                    return Err(io::Error::other("injected overflow failure"));
                }
                self.output.extend_from_slice(buffer);
                Ok(buffer.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let mut state = AppState::new(PiState {
            model: None,
            thinking_level: "off".to_owned(),
            is_streaming: false,
            is_compacting: false,
            steering_mode: "one-at-a-time".to_owned(),
            follow_up_mode: "one-at-a-time".to_owned(),
            session_file: None,
            session_id: "overflow-failure".to_owned(),
            session_name: None,
            auto_compaction_enabled: true,
            message_count: 0,
            pending_message_count: 0,
        });
        state.transcript = vec![
            TranscriptItem::Notice("one".to_owned()),
            TranscriptItem::Notice("two".to_owned()),
            TranscriptItem::Notice("three".to_owned()),
        ];
        let mut transcript = TranscriptStore::default();
        transcript.sync(&state);
        let projection = transcript.project_primary(20, 2, 1, 100, usize::MAX, 0);
        assert_eq!(projection.overflow_blocks.len(), 1);
        let mut coordinator = FrameCoordinator::default();
        let plan = coordinator.plan(frame(1, "resident"), SurfaceKind::Primary, Some(projection));
        let mut driver = TerminalDriver::new(
            FailOnScroll::default(),
            TerminalCapabilities {
                synchronized_output: false,
                true_color: false,
                mouse: false,
            },
            TerminalSize::new(20, 4),
        );

        assert!(
            coordinator
                .commit(&mut driver, &mut transcript, plan)
                .is_err()
        );
        assert_eq!(transcript.scrollback_cursor(), 0);
        assert_eq!(transcript.scrollback_row_offset(), 0);
        assert!(coordinator.terminal_invalid);
        assert!(!driver.physical_valid());
    }

    #[test]
    fn successful_commit_then_diff_preserves_revision_consistency() {
        let capabilities = TerminalCapabilities {
            synchronized_output: false,
            true_color: false,
            mouse: false,
        };
        let mut driver =
            TerminalDriver::new(Vec::<u8>::new(), capabilities, TerminalSize::new(20, 4));
        let mut coordinator = FrameCoordinator::default();
        let mut transcript = TranscriptStore::default();
        let first = coordinator.plan(frame(1, "one"), SurfaceKind::Primary, None);
        coordinator
            .commit(&mut driver, &mut transcript, first)
            .unwrap();
        let second = coordinator.plan(frame(2, "two"), SurfaceKind::Primary, None);
        assert!(matches!(second.frame_update, FrameUpdate::Rows { .. }));
        coordinator
            .commit(&mut driver, &mut transcript, second)
            .unwrap();
        assert_eq!(coordinator.committed_revision, 2);
        assert_eq!(coordinator.previous_frame.as_ref().unwrap().revision, 2);
    }

    #[test]
    fn overflow_redraw_keeps_the_primary_shadow_equal_to_the_visible_terminal() {
        let capabilities = TerminalCapabilities {
            synchronized_output: true,
            true_color: false,
            mouse: false,
        };
        let size = TerminalSize::new(20, 4);
        let mut driver = TerminalDriver::new(Vec::<u8>::new(), capabilities, size);
        let mut first = frame(1, "one");
        first.rows[1] = VisualRow {
            component_id: "two".to_owned(),
            logical_line: 0,
            wrap_index: 0,
            cells: vec![StyledCell::new("two", 3, CellStyle::default())],
        };
        driver
            .commit(&TerminalCommitPlan {
                revision: 1,
                surface: SurfaceKind::Primary,
                history_window: Rect::new(0, 0, 20, 2),
                bootstrap_scroll_rows: 0,
                bootstrap_padding_rows: 0,
                overflow_blocks: Vec::new(),
                frame_update: FrameUpdate::Full(first),
                panel: None,
                cursor: None,
                full_redraw: true,
            })
            .unwrap();

        let mut second = frame(2, "two");
        second.rows[1] = VisualRow {
            component_id: "three".to_owned(),
            logical_line: 0,
            wrap_index: 0,
            cells: vec![StyledCell::new("three", 5, CellStyle::default())],
        };
        driver
            .commit(&TerminalCommitPlan {
                revision: 2,
                surface: SurfaceKind::Primary,
                history_window: Rect::new(0, 0, 20, 2),
                bootstrap_scroll_rows: 0,
                bootstrap_padding_rows: 0,
                overflow_blocks: vec![CommittedHistoryBlock {
                    component_id: "one".to_owned(),
                    source_revision: 2,
                    row_offset: 0,
                    total_rows: 1,
                    rows: vec![VisualRow {
                        component_id: "one".to_owned(),
                        logical_line: 0,
                        wrap_index: 0,
                        cells: vec![StyledCell::new("one", 3, CellStyle::default())],
                    }],
                }],
                frame_update: FrameUpdate::Full(second),
                panel: None,
                cursor: None,
                full_redraw: true,
            })
            .unwrap();

        let mut terminal = VirtualTerminal::new(size);
        terminal.feed(driver.output_ref());
        assert_eq!(terminal.scrollback(), &["one"]);
        assert_eq!(
            terminal.visible_lines(),
            driver
                .primary_screen
                .iter()
                .map(VisualRow::plain_text)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn alternate_screen_round_trip_uses_save_and_restore_sequences() {
        let capabilities = TerminalCapabilities {
            synchronized_output: false,
            true_color: false,
            mouse: false,
        };
        let mut driver =
            TerminalDriver::new(Vec::<u8>::new(), capabilities, TerminalSize::new(20, 4));
        let alternate = TerminalCommitPlan {
            revision: 1,
            surface: SurfaceKind::Alternate,
            history_window: Rect::new(0, 0, 20, 2),
            bootstrap_scroll_rows: 0,
            bootstrap_padding_rows: 0,
            overflow_blocks: Vec::new(),
            frame_update: FrameUpdate::Full(frame(1, "alt")),
            panel: None,
            cursor: None,
            full_redraw: true,
        };
        driver.commit(&alternate).unwrap();
        let primary = TerminalCommitPlan {
            revision: 2,
            surface: SurfaceKind::Primary,
            history_window: Rect::new(0, 0, 20, 2),
            bootstrap_scroll_rows: 0,
            bootstrap_padding_rows: 0,
            overflow_blocks: Vec::new(),
            frame_update: FrameUpdate::Full(frame(2, "main")),
            panel: None,
            cursor: None,
            full_redraw: true,
        };
        driver.commit(&primary).unwrap();
        let output = String::from_utf8_lossy(driver.output_ref());
        assert!(output.contains("\u{1b}[?1049h"));
        assert!(output.contains("\u{1b}[?1049l"));
    }

    #[test]
    fn resize_is_applied_before_history_staging() {
        let capabilities = TerminalCapabilities {
            synchronized_output: false,
            true_color: false,
            mouse: false,
        };
        let mut driver =
            TerminalDriver::new(Vec::<u8>::new(), capabilities, TerminalSize::new(20, 8));
        let mut resized = frame(2, "resized");
        resized.terminal_size = TerminalSize::new(20, 3);
        resized.rows.truncate(3);
        let history_row = VisualRow {
            component_id: "history".to_owned(),
            logical_line: 0,
            wrap_index: 0,
            cells: vec![StyledCell::new("history", 7, CellStyle::default())],
        };
        let plan = TerminalCommitPlan {
            revision: 2,
            surface: SurfaceKind::Primary,
            history_window: Rect::new(0, 0, 20, 2),
            bootstrap_scroll_rows: 0,
            bootstrap_padding_rows: 0,
            overflow_blocks: vec![CommittedHistoryBlock {
                component_id: "history".to_owned(),
                source_revision: 2,
                row_offset: 0,
                total_rows: 1,
                rows: vec![history_row],
            }],
            frame_update: FrameUpdate::Full(resized),
            panel: None,
            cursor: None,
            full_redraw: true,
        };

        driver.commit(&plan).unwrap();

        assert_eq!(driver.size, TerminalSize::new(20, 3));
        assert_eq!(
            driver.primary_history_window,
            Some(crate::ui::types::Rect::new(0, 0, 20, 2))
        );
        let output = String::from_utf8_lossy(driver.output_ref());
        assert!(
            !output.contains("\u{1b}[4;1H"),
            "history staging addressed a row below the resized viewport"
        );
    }

    #[test]
    fn clear_scrollback_resets_all_owned_physical_state_in_order() {
        let mut driver = TerminalDriver::new(
            Vec::<u8>::new(),
            TerminalCapabilities {
                synchronized_output: true,
                true_color: false,
                mouse: false,
            },
            TerminalSize::new(20, 4),
        );
        driver.primary_screen[0] = VisualRow {
            component_id: "old".to_owned(),
            logical_line: 0,
            wrap_index: 0,
            cells: vec![StyledCell::new("old", 3, CellStyle::default())],
        };
        driver.primary_history_window = Some(Rect::new(0, 0, 20, 2));
        driver.active_panel = Some(Rect::new(1, 1, 5, 2));
        driver.owned_footer_height = 3;
        driver.pending_wrap = true;
        driver.physical_cursor = Some(CursorPosition { column: 4, row: 2 });

        driver.clear_scrollback_and_visible_screen().unwrap();

        let output = String::from_utf8_lossy(driver.output_ref());
        let scroll_region = output.find("\u{1b}[r").unwrap();
        let clear_screen = output.find("\u{1b}[2J").unwrap();
        let clear_scrollback = output.find("\u{1b}[3J").unwrap();
        assert!(scroll_region < clear_screen);
        assert!(clear_screen < clear_scrollback);
        assert!(output.contains("\u{1b}[?2026h"));
        assert!(output.contains("\u{1b}[?2026l"));
        assert!(
            driver
                .primary_screen
                .iter()
                .all(|row| row.plain_text().is_empty())
        );
        assert_eq!(driver.primary_history_window, None);
        assert_eq!(driver.active_panel, None);
        assert_eq!(driver.owned_footer_height, 0);
        assert!(!driver.pending_wrap);
        assert_eq!(driver.physical_cursor, None);
        assert!(driver.physical_valid());
    }

    #[test]
    fn synchronized_update_is_ended_when_destructive_reset_fails() {
        #[derive(Default)]
        struct FailOnClearWriter {
            output: Vec<u8>,
            failed: bool,
        }

        impl Write for FailOnClearWriter {
            fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
                if !self.failed && buffer.windows(3).any(|window| window == b"[2J") {
                    self.failed = true;
                    return Err(io::Error::other("injected clear failure"));
                }
                self.output.extend_from_slice(buffer);
                Ok(buffer.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let mut driver = TerminalDriver::new(
            FailOnClearWriter::default(),
            TerminalCapabilities {
                synchronized_output: true,
                true_color: false,
                mouse: false,
            },
            TerminalSize::new(20, 4),
        );

        assert!(driver.clear_scrollback_and_visible_screen().is_err());

        let output = String::from_utf8_lossy(&driver.output_ref().output);
        assert!(output.contains("\u{1b}[?2026h"));
        assert!(output.contains("\u{1b}[?2026l"));
        assert!(!driver.physical_valid());
    }

    #[test]
    fn destructive_reflow_purges_before_replaying_canonical_history() {
        let mut driver = TerminalDriver::new(
            Vec::<u8>::new(),
            TerminalCapabilities {
                synchronized_output: false,
                true_color: false,
                mouse: false,
            },
            TerminalSize::new(80, 4),
        );
        let history_row = VisualRow {
            component_id: "canonical".to_owned(),
            logical_line: 0,
            wrap_index: 0,
            cells: vec![StyledCell::new(
                "canonical-history",
                17,
                CellStyle::default(),
            )],
        };
        let mut resized = frame(2, "active");
        resized.terminal_size = TerminalSize::new(40, 4);
        resized.viewport = Rect::new(0, 0, 40, 4);
        let plan = TerminalCommitPlan {
            revision: 2,
            surface: SurfaceKind::Primary,
            history_window: Rect::new(0, 0, 20, 2),
            bootstrap_scroll_rows: 0,
            bootstrap_padding_rows: 0,
            overflow_blocks: vec![CommittedHistoryBlock {
                component_id: "canonical".to_owned(),
                source_revision: 2,
                row_offset: 0,
                total_rows: 1,
                rows: vec![history_row],
            }],
            frame_update: FrameUpdate::Full(resized),
            panel: None,
            cursor: None,
            full_redraw: true,
        };

        driver.commit_resize_reflow(&plan).unwrap();

        let output = String::from_utf8_lossy(driver.output_ref());
        let purge = output.find("\u{1b}[3J").unwrap();
        let replay = output.find("canonical-history").unwrap();
        assert!(purge < replay);
        assert_eq!(driver.size, TerminalSize::new(40, 4));
        assert!(driver.physical_valid());
    }

    #[test]
    fn failed_resize_reflow_keeps_canonical_cursor_for_a_clean_retry() {
        #[derive(Default)]
        struct FailOnPurgeWriter {
            output: Vec<u8>,
            failed: bool,
        }

        impl Write for FailOnPurgeWriter {
            fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
                if !self.failed && buffer.windows(3).any(|window| window == b"[3J") {
                    self.failed = true;
                    return Err(io::Error::other("injected purge failure"));
                }
                self.output.extend_from_slice(buffer);
                Ok(buffer.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let mut state = AppState::new(PiState {
            model: None,
            thinking_level: "off".to_owned(),
            is_streaming: false,
            is_compacting: false,
            steering_mode: "one-at-a-time".to_owned(),
            follow_up_mode: "one-at-a-time".to_owned(),
            session_file: None,
            session_id: "resize-test".to_owned(),
            session_name: None,
            auto_compaction_enabled: true,
            message_count: 0,
            pending_message_count: 0,
        });
        state
            .transcript
            .push(TranscriptItem::Notice("canonical".to_owned()));
        let mut transcript = TranscriptStore::default();
        transcript.sync(&state);
        let projection = transcript.canonical_reflow_projection(40, 0, 7, 0);
        let mut resized = frame(7, "active");
        resized.terminal_size = TerminalSize::new(40, 4);
        resized.viewport = Rect::new(0, 0, 40, 4);
        let mut coordinator = FrameCoordinator::default();
        let failed_plan = coordinator.plan_resize_reflow(resized.clone(), &projection);
        let mut failing_driver = TerminalDriver::new(
            FailOnPurgeWriter::default(),
            TerminalCapabilities {
                synchronized_output: true,
                true_color: false,
                mouse: false,
            },
            TerminalSize::new(80, 4),
        );

        assert!(
            coordinator
                .commit_resize_reflow(
                    &mut failing_driver,
                    &mut transcript,
                    failed_plan,
                    &projection,
                )
                .is_err()
        );
        assert_eq!(transcript.scrollback_cursor(), 0);
        assert!(coordinator.previous_frame.is_none());
        assert!(coordinator.terminal_invalid);

        let retry_plan = coordinator.plan_resize_reflow(resized, &projection);
        let mut retry_driver = TerminalDriver::new(
            Vec::<u8>::new(),
            TerminalCapabilities {
                synchronized_output: false,
                true_color: false,
                mouse: false,
            },
            TerminalSize::new(80, 4),
        );
        coordinator
            .commit_resize_reflow(&mut retry_driver, &mut transcript, retry_plan, &projection)
            .unwrap();
        assert_eq!(transcript.scrollback_cursor(), 1);
        assert_eq!(coordinator.previous_frame.as_ref().unwrap().revision, 7);
        assert!(!coordinator.terminal_invalid);
    }

    #[test]
    fn session_replacement_purges_previous_native_history() {
        fn block(id: &str, text: &str) -> CommittedHistoryBlock {
            CommittedHistoryBlock {
                component_id: id.to_owned(),
                source_revision: 1,
                row_offset: 0,
                total_rows: 1,
                rows: vec![VisualRow {
                    component_id: id.to_owned(),
                    logical_line: 0,
                    wrap_index: 0,
                    cells: vec![StyledCell::new(
                        text,
                        text.len() as u16,
                        CellStyle::default(),
                    )],
                }],
            }
        }

        let mut driver = TerminalDriver::new(
            Vec::<u8>::new(),
            TerminalCapabilities {
                synchronized_output: false,
                true_color: false,
                mouse: false,
            },
            TerminalSize::new(40, 4),
        );
        driver.begin_canonical_reflow().unwrap();
        driver
            .replay_canonical_history_batch(&[block("a", "session-A")])
            .unwrap();
        driver.begin_canonical_reflow().unwrap();
        driver
            .replay_canonical_history_batch(&[block("b", "session-B")])
            .unwrap();

        let output = String::from_utf8_lossy(driver.output_ref());
        let current_projection = output.rsplit("\u{1b}[3J").next().unwrap();
        assert!(!current_projection.contains("session-A"));
        assert_eq!(current_projection.matches("session-B").count(), 1);

        driver.begin_canonical_reflow().unwrap();
        driver
            .replay_canonical_history_batch(&[block("b", "session-B")])
            .unwrap();
        let output = String::from_utf8_lossy(driver.output_ref());
        let repeated_projection = output.rsplit("\u{1b}[3J").next().unwrap();
        assert_eq!(repeated_projection.matches("session-B").count(), 1);
    }

    #[test]
    fn terminal_failure_after_partial_history_batch_requires_a_fresh_destructive_replay() {
        #[derive(Default)]
        struct FailOnceOnSecondRow {
            output: Vec<u8>,
            failed: bool,
        }

        impl Write for FailOnceOnSecondRow {
            fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
                if !self.failed && buffer.windows(6).any(|window| window == b"second") {
                    self.failed = true;
                    return Err(io::Error::other("injected row failure"));
                }
                self.output.extend_from_slice(buffer);
                Ok(buffer.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let rows = ["first", "second"]
            .into_iter()
            .enumerate()
            .map(|(index, text)| VisualRow {
                component_id: "large".to_owned(),
                logical_line: index,
                wrap_index: 0,
                cells: vec![StyledCell::new(
                    text,
                    text.len() as u16,
                    CellStyle::default(),
                )],
            })
            .collect::<Vec<_>>();
        let batch = vec![CommittedHistoryBlock {
            component_id: "large".to_owned(),
            source_revision: 1,
            row_offset: 0,
            total_rows: 2,
            rows,
        }];
        let mut driver = TerminalDriver::new(
            FailOnceOnSecondRow::default(),
            TerminalCapabilities {
                synchronized_output: true,
                true_color: false,
                mouse: false,
            },
            TerminalSize::new(40, 4),
        );
        driver.begin_canonical_reflow().unwrap();

        assert!(driver.replay_canonical_history_batch(&batch).is_err());
        assert!(!driver.physical_valid());
        assert!(String::from_utf8_lossy(&driver.output_ref().output).contains("\u{1b}[?2026l"));

        driver.begin_canonical_reflow().unwrap();
        driver.replay_canonical_history_batch(&batch).unwrap();
        let output = String::from_utf8_lossy(&driver.output_ref().output);
        let recovered = output.rsplit("\u{1b}[3J").next().unwrap();
        assert_eq!(recovered.matches("first").count(), 1);
        assert_eq!(recovered.matches("second").count(), 1);
    }

    #[test]
    fn flush_failure_ends_synchronized_output_and_invalidates_physical_state() {
        #[derive(Default)]
        struct FailFirstFlush {
            output: Vec<u8>,
            failed: bool,
        }

        impl Write for FailFirstFlush {
            fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
                self.output.extend_from_slice(buffer);
                Ok(buffer.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                if !self.failed {
                    self.failed = true;
                    return Err(io::Error::other("injected flush failure"));
                }
                Ok(())
            }
        }

        let mut driver = TerminalDriver::new(
            FailFirstFlush::default(),
            TerminalCapabilities {
                synchronized_output: true,
                true_color: false,
                mouse: false,
            },
            TerminalSize::new(40, 4),
        );

        assert!(driver.begin_canonical_reflow().is_err());
        let output = String::from_utf8_lossy(&driver.output_ref().output);
        assert!(output.contains("\u{1b}[?2026h"));
        assert!(output.contains("\u{1b}[?2026l"));
        assert!(!driver.physical_valid());
    }
}
