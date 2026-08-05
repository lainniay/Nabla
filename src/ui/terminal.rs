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
    primary_viewport: Option<Rect>,
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
            primary_viewport: None,
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

    fn commit_internal(
        &mut self,
        plan: &TerminalCommitPlan,
        destructive_reflow: bool,
    ) -> io::Result<()> {
        self.switch_surface(plan.surface)?;
        let previous_screen = self.primary_screen.clone();
        let previous_panel = self.active_panel;
        let previous_size = self.size;
        let previous_viewport = self.primary_viewport;
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
            let mut released_viewport_rows = None;
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
                        .history_blocks
                        .iter()
                        .flat_map(|block| block.rows.iter())
                        .collect::<Vec<_>>();
                    self.append_history_fullscreen(&rows)?;
                    if let FrameUpdate::Full(frame) = &plan.frame_update {
                        self.primary_viewport = Some(normalize_viewport(frame.viewport, self.size));
                    }
                } else {
                    self.restore_active_panel()?;
                    if let FrameUpdate::Full(frame) = &plan.frame_update {
                        released_viewport_rows = self.prepare_primary_viewport(frame.viewport)?;
                    }
                    if plan.history_scroll_rows > 0 {
                        self.append_history(plan, released_viewport_rows)?;
                    } else if let Some(released) = released_viewport_rows {
                        self.shift_history_region_down(released.bottom(), released.height)?;
                    }
                }
            }
            match &plan.frame_update {
                FrameUpdate::Full(frame) => self.draw_full(frame)?,
                FrameUpdate::Rows { rows, .. } => {
                    for (terminal_row, row) in rows {
                        if self.surface == SurfaceKind::Primary
                            && self.primary_viewport.is_some_and(|viewport| {
                                *terminal_row < viewport.y || *terminal_row >= viewport.bottom()
                            })
                        {
                            continue;
                        }
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
                self.primary_viewport = previous_viewport;
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
        self.primary_viewport = None;
        self.active_panel = None;
        self.owned_footer_height = 0;
        self.pending_wrap = false;
        self.physical_cursor = None;
    }

    fn append_history(
        &mut self,
        plan: &TerminalCommitPlan,
        released_viewport_rows: Option<Rect>,
    ) -> io::Result<()> {
        let rows = plan
            .history_blocks
            .iter()
            .flat_map(|block| block.rows.iter())
            .take(plan.history_scroll_rows)
            .collect::<Vec<_>>();
        let replacement_rows = released_viewport_rows
            .map_or(0, |released| usize::from(released.height))
            .min(rows.len());
        let overflow_rows = rows.len().saturating_sub(replacement_rows);
        if overflow_rows > 0 {
            let history_bottom = released_viewport_rows.map_or_else(
                || {
                    self.primary_viewport
                        .map_or(self.size.height, |viewport| viewport.y)
                },
                |released| released.y,
            );
            if history_bottom > 0 {
                self.append_history_above_viewport(&rows[..overflow_rows], history_bottom)?;
            } else {
                self.append_history_fullscreen(&rows[..overflow_rows])?;
            }
        }
        if replacement_rows > 0
            && let Some(released) = released_viewport_rows
        {
            let replacement_start = released
                .bottom()
                .saturating_sub(u16::try_from(replacement_rows).unwrap_or(released.height));
            let unused_rows = released
                .height
                .saturating_sub(u16::try_from(replacement_rows).unwrap_or(released.height));
            if unused_rows > 0 {
                self.shift_history_region_down(replacement_start, unused_rows)?;
            }
            for (offset, row) in rows[overflow_rows..].iter().enumerate() {
                self.draw_base_row(
                    replacement_start.saturating_add(u16::try_from(offset).unwrap_or(u16::MAX)),
                    row,
                )?;
            }
        }
        if replacement_rows == 0
            && let Some(released) = released_viewport_rows
        {
            self.shift_history_region_down(released.bottom(), released.height)?;
        }
        Ok(())
    }

    fn append_history_above_viewport(
        &mut self,
        rows: &[&VisualRow],
        viewport_top: u16,
    ) -> io::Result<()> {
        self.set_scroll_region(viewport_top)?;
        let result = (|| {
            queue!(
                self.output,
                MoveTo(0, viewport_top.saturating_sub(1)),
                ResetColor,
                SetAttribute(Attribute::Reset)
            )?;
            for row in rows {
                self.output.write_all(b"\r\n")?;
                self.scroll_primary_screen_up(viewport_top, 1);
                self.draw_base_row(viewport_top.saturating_sub(1), row)?;
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
                self.scroll_primary_screen_up(self.size.height, 1);
            }
        }
        Ok(())
    }

    fn prepare_primary_viewport(&mut self, requested: Rect) -> io::Result<Option<Rect>> {
        let requested_height = requested.height.min(self.size.height);
        let next = Rect::new(
            0,
            self.size.height.saturating_sub(requested_height),
            self.size.width,
            requested_height,
        );
        let previous_height = self
            .primary_viewport
            .map_or(0, |viewport| viewport.height.min(self.size.height));
        let previous = Rect::new(
            0,
            self.size.height.saturating_sub(previous_height),
            self.size.width,
            previous_height,
        );

        let released = if next.y < previous.y {
            self.scroll_history_region(previous.y, previous.y.saturating_sub(next.y))?;
            None
        } else if next.y > previous.y {
            for terminal_row in previous.y..next.y {
                self.clear_base_row(terminal_row)?;
            }
            Some(Rect::new(
                0,
                previous.y,
                self.size.width,
                next.y.saturating_sub(previous.y),
            ))
        } else {
            None
        };
        self.primary_viewport = Some(next);
        Ok(released)
    }

    fn scroll_history_region(&mut self, bottom: u16, rows: u16) -> io::Result<()> {
        if bottom == 0 || rows == 0 {
            return Ok(());
        }
        self.set_scroll_region(bottom)?;
        let result = (|| {
            queue!(
                self.output,
                MoveTo(0, bottom.saturating_sub(1)),
                ResetColor,
                SetAttribute(Attribute::Reset)
            )?;
            for _ in 0..rows {
                self.output.write_all(b"\r\n")?;
                self.scroll_primary_screen_up(bottom, 1);
            }
            Ok(())
        })();
        let reset = self.reset_scroll_region();
        result.and(reset)
    }

    fn shift_history_region_down(&mut self, bottom: u16, rows: u16) -> io::Result<()> {
        if bottom == 0 || rows == 0 {
            return Ok(());
        }
        self.set_scroll_region(bottom)?;
        let result = (|| {
            queue!(
                self.output,
                MoveTo(0, 0),
                ResetColor,
                SetAttribute(Attribute::Reset)
            )?;
            for _ in 0..rows {
                self.output.write_all(b"\x1bM")?;
                self.scroll_primary_screen_down(bottom, 1);
            }
            Ok(())
        })();
        let reset = self.reset_scroll_region();
        result.and(reset)
    }

    fn set_scroll_region(&mut self, bottom: u16) -> io::Result<()> {
        write!(self.output, "\u{1b}[1;{}r", bottom.max(1))
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

    fn scroll_primary_screen_up(&mut self, bottom: u16, rows: u16) {
        let bottom = usize::from(bottom.min(self.size.height));
        for _ in 0..rows {
            if bottom == 0 || self.primary_screen.len() < bottom {
                return;
            }
            self.primary_screen.remove(0);
            self.primary_screen
                .insert(bottom.saturating_sub(1), VisualRow::blank("surface"));
        }
    }

    fn scroll_primary_screen_down(&mut self, bottom: u16, rows: u16) {
        let bottom = usize::from(bottom.min(self.size.height));
        for _ in 0..rows {
            if bottom == 0 || self.primary_screen.len() < bottom {
                return;
            }
            self.primary_screen.insert(0, VisualRow::blank("surface"));
            self.primary_screen.remove(bottom);
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

fn normalize_viewport(viewport: Rect, size: TerminalSize) -> Rect {
    let height = viewport.height.min(size.height);
    Rect::new(0, size.height.saturating_sub(height), size.width, height)
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
}

impl FrameCoordinator {
    pub fn plan(
        &self,
        frame: VisualFrame,
        surface: SurfaceKind,
        history_blocks: Vec<CommittedHistoryBlock>,
    ) -> TerminalCommitPlan {
        let history_scroll_rows = history_blocks
            .iter()
            .flat_map(|block| block.rows.iter())
            .count();
        let full_redraw = self.terminal_invalid
            || self.previous_surface != surface
            || history_scroll_rows > 0
            || self.previous_frame.as_ref().is_none_or(|previous| {
                previous.terminal_size != frame.terminal_size
                    || previous.viewport != frame.viewport
                    || previous.main_layout != frame.main_layout
                    || previous.component_bounds != frame.component_bounds
                    || previous.hit_regions != frame.hit_regions
            });
        let cursor = frame.cursor;
        let panel = frame.panel.clone();
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
            history_scroll_rows,
            history_blocks,
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
                transcript.acknowledge_history(&plan.history_blocks);
                self.committed_revision = plan.revision;
                self.previous_frame = Some(next_frame);
                self.previous_surface = plan.surface;
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
        TerminalCommitPlan {
            revision: frame.revision,
            surface: SurfaceKind::Primary,
            history_scroll_rows: projection
                .history_blocks
                .iter()
                .map(|block| block.rows.len())
                .sum(),
            history_blocks: projection.history_blocks.clone(),
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

    pub fn invalidate(&mut self) {
        self.terminal_invalid = true;
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;
    use crate::ui::types::{MainLayout, TerminalSize};
    use crate::{
        rpc::PiState,
        state::{AppState, TranscriptItem},
    };

    fn frame(revision: u64, text: &str) -> VisualFrame {
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
            main_layout: MainLayout::default(),
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
        let plan = coordinator.plan(frame(1, "hello"), SurfaceKind::Primary, Vec::new());
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
        let first = coordinator.plan(frame(1, "one"), SurfaceKind::Primary, Vec::new());
        coordinator
            .commit(&mut driver, &mut transcript, first)
            .unwrap();
        let second = coordinator.plan(frame(2, "two"), SurfaceKind::Primary, Vec::new());
        assert!(matches!(second.frame_update, FrameUpdate::Rows { .. }));
        coordinator
            .commit(&mut driver, &mut transcript, second)
            .unwrap();
        assert_eq!(coordinator.committed_revision, 2);
        assert_eq!(coordinator.previous_frame.as_ref().unwrap().revision, 2);
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
            history_scroll_rows: 0,
            history_blocks: Vec::new(),
            frame_update: FrameUpdate::Full(frame(1, "alt")),
            panel: None,
            cursor: None,
            full_redraw: true,
        };
        driver.commit(&alternate).unwrap();
        let primary = TerminalCommitPlan {
            revision: 2,
            surface: SurfaceKind::Primary,
            history_scroll_rows: 0,
            history_blocks: Vec::new(),
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
            history_scroll_rows: 1,
            history_blocks: vec![CommittedHistoryBlock {
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
            driver.primary_viewport,
            Some(crate::ui::types::Rect::new(0, 0, 20, 3))
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
        driver.primary_viewport = Some(Rect::new(0, 0, 20, 2));
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
        assert_eq!(driver.primary_viewport, None);
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
    fn resize_reflow_purges_before_replaying_canonical_history() {
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
            history_scroll_rows: 1,
            history_blocks: vec![CommittedHistoryBlock {
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
        let projection = transcript.canonical_reflow_projection(40, 7, 0);
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
        assert_eq!(transcript.committed_cursor(), 0);
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
        assert_eq!(transcript.committed_cursor(), 1);
        assert_eq!(coordinator.previous_frame.as_ref().unwrap().revision, 7);
        assert!(!coordinator.terminal_invalid);
    }
}
