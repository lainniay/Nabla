//! A small deterministic VT used by multi-frame terminal tests.

use unicode_width::UnicodeWidthChar;

use super::types::TerminalSize;

#[derive(Debug, Clone)]
pub struct VirtualTerminal {
    size: TerminalSize,
    primary: Vec<Vec<String>>,
    alternate: Option<Vec<Vec<String>>>,
    scrollback: Vec<String>,
    row: usize,
    column: usize,
    scroll_top: usize,
    scroll_bottom: usize,
    saved_primary_cursor: (usize, usize),
}

impl VirtualTerminal {
    pub fn new(size: TerminalSize) -> Self {
        Self {
            size,
            primary: blank_grid(size),
            alternate: None,
            scrollback: Vec::new(),
            row: 0,
            column: 0,
            scroll_top: 0,
            scroll_bottom: usize::from(size.height),
            saved_primary_cursor: (0, 0),
        }
    }

    pub fn with_shell_lines(size: TerminalSize, lines: &[&str]) -> Self {
        let mut terminal = Self::new(size);
        for (row, line) in lines.iter().take(usize::from(size.height)).enumerate() {
            terminal.write_line(row, line);
        }
        terminal.row = lines.len().min(usize::from(size.height)).saturating_sub(1);
        terminal.column = lines
            .last()
            .map_or(0, |line| line.width())
            .min(usize::from(size.width).saturating_sub(1));
        terminal
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        let input = String::from_utf8_lossy(bytes);
        let characters = input.chars().collect::<Vec<_>>();
        let mut index = 0usize;
        while index < characters.len() {
            match characters[index] {
                '\u{1b}' if characters.get(index + 1) == Some(&'[') => {
                    let start = index + 2;
                    index = start;
                    while index < characters.len()
                        && !matches!(characters[index] as u32, 0x40..=0x7e)
                    {
                        index += 1;
                    }
                    if index < characters.len() {
                        let parameters = characters[start..index].iter().collect::<String>();
                        self.csi(&parameters, characters[index]);
                    }
                }
                '\u{1b}' if characters.get(index + 1) == Some(&'M') => {
                    self.reverse_index();
                    index += 1;
                }
                '\r' => self.column = 0,
                '\n' => self.newline(),
                character if !character.is_control() => self.put(character),
                _ => {}
            }
            index += 1;
        }
    }

    pub fn visible_lines(&self) -> Vec<String> {
        self.grid()
            .iter()
            .map(|row| {
                row.iter()
                    .map(String::as_str)
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect()
    }

    pub fn scrollback(&self) -> &[String] {
        &self.scrollback
    }

    pub fn in_alternate_screen(&self) -> bool {
        self.alternate.is_some()
    }

    fn csi(&mut self, parameters: &str, final_character: char) {
        match final_character {
            'H' | 'f' => {
                let mut values = parameters
                    .split(';')
                    .map(|value| value.parse::<usize>().unwrap_or(1));
                self.row = values
                    .next()
                    .unwrap_or(1)
                    .saturating_sub(1)
                    .min(usize::from(self.size.height).saturating_sub(1));
                self.column = values
                    .next()
                    .unwrap_or(1)
                    .saturating_sub(1)
                    .min(usize::from(self.size.width).saturating_sub(1));
            }
            'K' => {
                let mode = parameters.parse::<u8>().unwrap_or(0);
                let width = usize::from(self.size.width);
                let column = self.column.min(width);
                let row = self.row;
                let grid = self.grid_mut();
                if let Some(cells) = grid.get_mut(row) {
                    match mode {
                        1 => cells[..=column.min(width.saturating_sub(1))].fill(String::new()),
                        2 => cells.fill(String::new()),
                        _ => cells[column..].fill(String::new()),
                    }
                }
            }
            'J' => {
                let mode = parameters.parse::<u8>().unwrap_or(0);
                assert_ne!(mode, 3, "the UI must never purge native scrollback");
                if mode == 2 {
                    self.grid_mut()
                        .iter_mut()
                        .for_each(|row| row.fill(String::new()));
                }
            }
            'r' => {
                let height = usize::from(self.size.height);
                if parameters.is_empty() {
                    self.scroll_top = 0;
                    self.scroll_bottom = height;
                } else {
                    let mut values = parameters
                        .split(';')
                        .map(|value| value.parse::<usize>().unwrap_or(1));
                    let top = values.next().unwrap_or(1).saturating_sub(1);
                    let bottom = values.next().unwrap_or(height).min(height);
                    if top < bottom {
                        self.scroll_top = top;
                        self.scroll_bottom = bottom;
                    } else {
                        self.scroll_top = 0;
                        self.scroll_bottom = height;
                    }
                }
                self.row = 0;
                self.column = 0;
            }
            'h' if parameters == "?1049" => {
                if self.alternate.is_none() {
                    self.saved_primary_cursor = (self.row, self.column);
                    self.alternate = Some(blank_grid(self.size));
                    self.row = 0;
                    self.column = 0;
                }
            }
            'l' if parameters == "?1049" => {
                if self.alternate.take().is_some() {
                    (self.row, self.column) = self.saved_primary_cursor;
                }
            }
            _ => {}
        }
    }

    fn put(&mut self, character: char) {
        let width = character.width().unwrap_or(0);
        if width == 0 {
            if self.column > 0 {
                let row = self.row;
                let column = self.column - 1;
                if let Some(cell) = self
                    .grid_mut()
                    .get_mut(row)
                    .and_then(|cells| cells.get_mut(column))
                {
                    cell.push(character);
                }
            }
            return;
        }
        if self.column.saturating_add(width) > usize::from(self.size.width) {
            self.newline();
        }
        let row = self.row;
        let column = self.column;
        if let Some(cells) = self.grid_mut().get_mut(row) {
            if let Some(cell) = cells.get_mut(column) {
                *cell = character.to_string();
            }
            for continuation in 1..width {
                if let Some(cell) = cells.get_mut(column + continuation) {
                    cell.clear();
                }
            }
        }
        self.column = self.column.saturating_add(width);
    }

    fn newline(&mut self) {
        self.column = 0;
        if self.row == self.scroll_bottom.saturating_sub(1) && self.scroll_top < self.scroll_bottom
        {
            self.scroll_region_up();
            self.row = self.scroll_bottom.saturating_sub(1);
            return;
        }
        if self.row + 1 < usize::from(self.size.height) {
            self.row += 1;
            return;
        }
        self.scroll_region_up();
        self.row = usize::from(self.size.height).saturating_sub(1);
    }

    fn scroll_region_up(&mut self) {
        let primary = self.alternate.is_none();
        let width = usize::from(self.size.width);
        let top = self.scroll_top;
        let bottom = self.scroll_bottom.min(usize::from(self.size.height));
        if top >= bottom {
            return;
        }
        let removed = {
            let grid = self.grid_mut();
            let removed = grid.remove(top);
            grid.insert(bottom.saturating_sub(1), vec![String::new(); width]);
            removed
        };
        if primary && top == 0 {
            self.scrollback.push(
                removed
                    .iter()
                    .map(String::as_str)
                    .collect::<String>()
                    .trim_end()
                    .to_owned(),
            );
        }
    }

    fn reverse_index(&mut self) {
        if self.row > self.scroll_top {
            self.row -= 1;
            return;
        }
        let width = usize::from(self.size.width);
        let top = self.scroll_top;
        let bottom = self.scroll_bottom.min(usize::from(self.size.height));
        if top >= bottom {
            return;
        }
        let grid = self.grid_mut();
        grid.insert(top, vec![String::new(); width]);
        grid.remove(bottom);
        self.row = top;
    }

    fn write_line(&mut self, row: usize, line: &str) {
        self.row = row;
        self.column = 0;
        for character in line.chars() {
            self.put(character);
        }
    }

    fn grid(&self) -> &Vec<Vec<String>> {
        self.alternate.as_ref().unwrap_or(&self.primary)
    }

    fn grid_mut(&mut self) -> &mut Vec<Vec<String>> {
        self.alternate.as_mut().unwrap_or(&mut self.primary)
    }
}

fn blank_grid(size: TerminalSize) -> Vec<Vec<String>> {
    vec![vec![String::new(); usize::from(size.width)]; usize::from(size.height)]
}

trait StringWidth {
    fn width(&self) -> usize;
}

impl StringWidth for str {
    fn width(&self) -> usize {
        unicode_width::UnicodeWidthStr::width(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{
        terminal::{TerminalCapabilities, TerminalDriver},
        types::{
            CellStyle, CommittedHistoryBlock, FrameUpdate, MainLayout, PanelFrame, Rect,
            StyledCell, SurfaceKind, TerminalCommitPlan, VisualFrame, VisualRow,
        },
    };

    fn row(id: &str, text: &str) -> VisualRow {
        VisualRow {
            component_id: id.to_owned(),
            logical_line: 0,
            wrap_index: 0,
            cells: vec![StyledCell::new(
                text,
                unicode_width::UnicodeWidthStr::width(text) as u16,
                CellStyle::default(),
            )],
        }
    }

    fn frame(revision: u64, size: TerminalSize, first: &str) -> VisualFrame {
        frame_in_viewport(
            revision,
            size,
            Rect::new(0, 0, size.width, size.height),
            first,
        )
    }

    fn frame_in_viewport(
        revision: u64,
        size: TerminalSize,
        viewport: Rect,
        first: &str,
    ) -> VisualFrame {
        let mut rows = (0..size.height)
            .map(|_| VisualRow::blank("surface"))
            .collect::<Vec<_>>();
        if let Some(target) = rows.get_mut(usize::from(viewport.y)) {
            *target = row("content", first);
        }
        VisualFrame {
            revision,
            terminal_size: size,
            rows,
            viewport,
            panel: None,
            component_bounds: Default::default(),
            hit_regions: Vec::new(),
            cursor: None,
            main_layout: MainLayout::default(),
        }
    }

    fn fixed_primary_frame(
        revision: u64,
        size: TerminalSize,
        history: &[&str],
        footer: &str,
    ) -> VisualFrame {
        let history_height = size.height.saturating_sub(2);
        let history_window = Rect::new(0, 0, size.width, history_height);
        let mut rows = (0..size.height)
            .map(|_| VisualRow::blank("surface"))
            .collect::<Vec<_>>();
        let history_len = history.len().min(usize::from(history_height));
        let start = usize::from(history_height).saturating_sub(history_len);
        for (offset, text) in history[history.len().saturating_sub(history_len)..]
            .iter()
            .enumerate()
        {
            rows[start + offset] = row("history", text);
        }
        if let Some(composer) = rows.get_mut(usize::from(history_height)) {
            *composer = row("composer", footer);
        }
        VisualFrame {
            revision,
            terminal_size: size,
            rows,
            viewport: Rect::new(0, 0, size.width, size.height),
            panel: None,
            component_bounds: Default::default(),
            hit_regions: Vec::new(),
            cursor: None,
            main_layout: MainLayout {
                transcript: history_window,
                history_window,
                owned_surface: Rect::new(0, 0, size.width, size.height),
                panel: None,
                composer: Rect::new(0, history_height, size.width, 1),
                status: Rect::new(0, history_height.saturating_add(1), size.width, 1),
            },
        }
    }

    fn plan(
        frame: VisualFrame,
        bootstrap_scroll_rows: usize,
        bootstrap_padding_rows: usize,
        overflow_blocks: Vec<CommittedHistoryBlock>,
    ) -> TerminalCommitPlan {
        TerminalCommitPlan {
            revision: frame.revision,
            surface: SurfaceKind::Primary,
            history_window: frame.main_layout.history_window,
            bootstrap_scroll_rows,
            bootstrap_padding_rows,
            overflow_blocks,
            panel: frame.panel.clone(),
            cursor: frame.cursor,
            frame_update: FrameUpdate::Full(frame),
            full_redraw: true,
        }
    }

    fn overflow(id: &str, revision: u64, offset: usize, text: &str) -> CommittedHistoryBlock {
        CommittedHistoryBlock {
            component_id: id.to_owned(),
            source_revision: revision,
            row_offset: offset,
            total_rows: offset.saturating_add(1),
            rows: vec![row(id, text)],
        }
    }

    fn capabilities() -> TerminalCapabilities {
        TerminalCapabilities {
            synchronized_output: true,
            true_color: false,
            mouse: false,
        }
    }

    fn panel(area: Rect, labels: &[&str]) -> PanelFrame {
        let mut rows = labels
            .iter()
            .map(|label| row("panel", label))
            .collect::<Vec<_>>();
        rows.resize_with(usize::from(area.height), || VisualRow::blank("panel"));
        PanelFrame { area, rows }
    }

    #[test]
    fn startup_keeps_prefilled_shell_output_in_native_scrollback() {
        let size = TerminalSize::new(20, 4);
        let mut vt = VirtualTerminal::with_shell_lines(size, &["one", "two", "three", "$ nabla"]);
        let mut driver = TerminalDriver::new(Vec::<u8>::new(), capabilities(), size);
        driver.claim_primary_surface().unwrap();
        vt.feed(driver.output_ref());
        assert!(vt.scrollback().iter().any(|line| line == "one"));
        assert!(vt.scrollback().iter().any(|line| line == "$ nabla"));
    }

    #[test]
    fn bootstrap_blank_rows_move_into_scrollback_as_resident_history_grows() {
        let size = TerminalSize::new(20, 5);
        let mut driver = TerminalDriver::new(Vec::<u8>::new(), capabilities(), size);
        driver
            .commit(&plan(
                fixed_primary_frame(1, size, &["one"], "composer"),
                0,
                2,
                Vec::new(),
            ))
            .unwrap();
        driver
            .commit(&plan(
                fixed_primary_frame(2, size, &["one", "two"], "composer"),
                1,
                1,
                Vec::new(),
            ))
            .unwrap();
        let mut vt = VirtualTerminal::new(size);
        vt.feed(driver.output_ref());
        assert_eq!(vt.scrollback(), &[""]);
        assert_eq!(&vt.visible_lines()[0..3], ["", "one", "two"]);
        assert_eq!(vt.visible_lines()[3], "composer");
    }

    #[test]
    fn monotonic_overflow_commits_rows_once_and_never_scrolls_the_footer() {
        let size = TerminalSize::new(20, 5);
        let mut driver = TerminalDriver::new(Vec::<u8>::new(), capabilities(), size);
        driver
            .commit(&plan(
                fixed_primary_frame(1, size, &["one", "two", "three"], "composer"),
                0,
                0,
                Vec::new(),
            ))
            .unwrap();
        driver
            .commit(&plan(
                fixed_primary_frame(2, size, &["two", "three", "four"], "composer"),
                0,
                0,
                vec![overflow("component", 2, 0, "one")],
            ))
            .unwrap();
        driver
            .commit(&plan(
                fixed_primary_frame(3, size, &["three", "four", "five"], "composer"),
                0,
                0,
                vec![overflow("component", 3, 1, "two")],
            ))
            .unwrap();

        let mut vt = VirtualTerminal::new(size);
        vt.feed(driver.output_ref());
        assert_eq!(vt.scrollback(), &["one", "two"]);
        assert_eq!(&vt.visible_lines()[0..3], ["three", "four", "five"]);
        assert_eq!(vt.visible_lines()[3], "composer");
        assert!(!vt.scrollback().iter().any(|line| line == "composer"));
        assert!(!String::from_utf8_lossy(driver.output_ref()).contains("\u{1b}M"));
    }

    #[test]
    fn stable_rows_do_not_overflow_until_they_leave_the_resident_window() {
        let size = TerminalSize::new(20, 5);
        let mut driver = TerminalDriver::new(Vec::<u8>::new(), capabilities(), size);
        driver
            .commit(&plan(
                fixed_primary_frame(1, size, &["stable", "live"], "composer"),
                0,
                1,
                Vec::new(),
            ))
            .unwrap();
        driver
            .commit(&plan(
                fixed_primary_frame(2, size, &["stable", "live", "new"], "composer"),
                1,
                0,
                Vec::new(),
            ))
            .unwrap();
        let mut vt = VirtualTerminal::new(size);
        vt.feed(driver.output_ref());
        assert_eq!(vt.scrollback(), &[""]);
        assert_eq!(&vt.visible_lines()[0..3], ["stable", "live", "new"]);
    }

    #[test]
    fn floating_panel_covers_and_restores_history_without_scrolling() {
        let size = TerminalSize::new(20, 5);
        let mut driver = TerminalDriver::new(Vec::<u8>::new(), capabilities(), size);
        let baseline = fixed_primary_frame(1, size, &["history"], "footer");
        driver
            .commit(&plan(baseline.clone(), 0, 2, Vec::new()))
            .unwrap();
        let panel_only_output = driver.output_ref().len();

        let mut opened = baseline.clone();
        opened.revision = 2;
        opened.panel = Some(panel(Rect::new(0, 1, 20, 2), &["first", "second"]));
        driver.commit(&plan(opened, 0, 2, Vec::new())).unwrap();
        let mut restored = baseline;
        restored.revision = 3;
        driver.commit(&plan(restored, 0, 2, Vec::new())).unwrap();

        let emitted = &driver.output_ref()[panel_only_output..];
        let emitted = String::from_utf8_lossy(emitted);
        assert!(!emitted.contains("\r\n"));
        assert!(!emitted.contains("\u{1b}M"));
        assert!(!emitted.contains("\u{1b}[1;3r"));

        let mut vt = VirtualTerminal::new(size);
        vt.feed(driver.output_ref());
        assert_eq!(vt.visible_lines()[2], "history");
        assert_eq!(vt.visible_lines()[3], "footer");
        assert!(!vt.visible_lines().iter().any(|line| line == "first"));
        assert!(!vt.visible_lines().iter().any(|line| line == "second"));
    }

    #[test]
    fn hidden_base_updates_are_restored_after_panel_closes() {
        let size = TerminalSize::new(20, 5);
        let viewport = Rect::new(0, 2, 20, 3);
        let mut driver = TerminalDriver::new(Vec::<u8>::new(), capabilities(), size);
        let mut opened = frame_in_viewport(1, size, viewport, "old");
        opened.panel = Some(panel(Rect::new(0, 2, 20, 2), &["panel-a", "panel-b"]));
        driver
            .commit(&TerminalCommitPlan {
                revision: 1,
                surface: SurfaceKind::Primary,
                history_window: Rect::new(0, 0, 20, 2),
                bootstrap_scroll_rows: 0,
                bootstrap_padding_rows: 0,
                overflow_blocks: Vec::new(),
                frame_update: FrameUpdate::Full(opened.clone()),
                panel: opened.panel.clone(),
                cursor: None,
                full_redraw: true,
            })
            .unwrap();

        let mut updated = frame_in_viewport(2, size, viewport, "new");
        updated.panel = opened.panel.clone();
        driver
            .commit(&TerminalCommitPlan {
                revision: 2,
                surface: SurfaceKind::Primary,
                history_window: Rect::new(0, 0, 20, 2),
                bootstrap_scroll_rows: 0,
                bootstrap_padding_rows: 0,
                overflow_blocks: Vec::new(),
                frame_update: FrameUpdate::Full(updated),
                panel: opened.panel.clone(),
                cursor: None,
                full_redraw: true,
            })
            .unwrap();
        driver
            .commit(&TerminalCommitPlan {
                revision: 3,
                surface: SurfaceKind::Primary,
                history_window: Rect::new(0, 0, 20, 2),
                bootstrap_scroll_rows: 0,
                bootstrap_padding_rows: 0,
                overflow_blocks: Vec::new(),
                frame_update: FrameUpdate::Full(frame_in_viewport(3, size, viewport, "new")),
                panel: None,
                cursor: None,
                full_redraw: true,
            })
            .unwrap();

        let mut vt = VirtualTerminal::new(size);
        vt.feed(driver.output_ref());
        assert_eq!(vt.visible_lines()[2], "new");
        assert!(!vt.visible_lines().iter().any(|line| line == "panel-a"));
    }

    #[test]
    fn alternate_screen_restores_the_primary_surface_before_redraw() {
        let size = TerminalSize::new(20, 4);
        let mut driver = TerminalDriver::new(Vec::<u8>::new(), capabilities(), size);
        for (revision, surface, text) in [
            (1, SurfaceKind::Primary, "main"),
            (2, SurfaceKind::Alternate, "browser"),
            (3, SurfaceKind::Primary, "main restored"),
        ] {
            driver
                .commit(&TerminalCommitPlan {
                    revision,
                    surface,
                    history_window: Rect::new(0, 0, 20, 2),
                    bootstrap_scroll_rows: 0,
                    bootstrap_padding_rows: 0,
                    overflow_blocks: Vec::new(),
                    frame_update: FrameUpdate::Full(frame(revision, size, text)),
                    panel: None,
                    cursor: None,
                    full_redraw: true,
                })
                .unwrap();
        }
        let mut vt = VirtualTerminal::new(size);
        vt.feed(driver.output_ref());
        assert!(!vt.in_alternate_screen());
        assert_eq!(vt.visible_lines()[0], "main restored");
    }

    #[test]
    fn alternate_round_trip_restores_a_primary_panel_underlay() {
        let size = TerminalSize::new(20, 4);
        let mut driver = TerminalDriver::new(Vec::<u8>::new(), capabilities(), size);
        let mut primary = fixed_primary_frame(1, size, &["history"], "main");
        primary.panel = Some(panel(Rect::new(0, 1, 20, 1), &["panel"]));
        driver.commit(&plan(primary, 0, 1, Vec::new())).unwrap();
        driver
            .commit(&TerminalCommitPlan {
                revision: 2,
                surface: SurfaceKind::Alternate,
                history_window: Rect::new(0, 0, 20, 2),
                bootstrap_scroll_rows: 0,
                bootstrap_padding_rows: 0,
                overflow_blocks: Vec::new(),
                frame_update: FrameUpdate::Full(frame(2, size, "browser")),
                panel: None,
                cursor: None,
                full_redraw: true,
            })
            .unwrap();
        driver
            .commit(&plan(
                fixed_primary_frame(3, size, &["history"], "main restored"),
                0,
                1,
                Vec::new(),
            ))
            .unwrap();

        let mut vt = VirtualTerminal::new(size);
        vt.feed(driver.output_ref());
        assert!(!vt.in_alternate_screen());
        assert_eq!(vt.visible_lines()[1], "history");
        assert_eq!(vt.visible_lines()[2], "main restored");
    }
}
