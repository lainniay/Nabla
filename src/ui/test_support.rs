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
    fn history_commit_remains_visible_above_the_inline_viewport() {
        let size = TerminalSize::new(20, 4);
        let mut driver = TerminalDriver::new(Vec::<u8>::new(), capabilities(), size);
        let viewport = Rect::new(0, 2, 20, 2);
        let plan = TerminalCommitPlan {
            revision: 2,
            surface: SurfaceKind::Primary,
            history_scroll_rows: 1,
            history_blocks: vec![CommittedHistoryBlock {
                component_id: "sealed".to_owned(),
                source_revision: 2,
                rows: vec![row("sealed", "sealed once")],
            }],
            frame_update: FrameUpdate::Full(frame_in_viewport(2, size, viewport, "live")),
            panel: None,
            cursor: None,
            full_redraw: true,
        };
        driver.commit(&plan).unwrap();
        let mut vt = VirtualTerminal::new(size);
        vt.feed(driver.output_ref());
        assert_eq!(vt.visible_lines()[1], "sealed once");
        assert_eq!(vt.visible_lines()[2], "live");
        assert!(
            !vt.scrollback()
                .iter()
                .any(|line| line.contains("sealed once"))
        );
        assert!(vt.visible_lines().iter().any(|line| line == "live"));
    }

    #[test]
    fn repeated_history_naturally_pushes_the_oldest_line_into_scrollback() {
        let size = TerminalSize::new(20, 5);
        let viewport = Rect::new(0, 3, 20, 2);
        let mut driver = TerminalDriver::new(Vec::<u8>::new(), capabilities(), size);
        for (index, text) in ["one", "two", "three", "four"].into_iter().enumerate() {
            driver
                .commit(&TerminalCommitPlan {
                    revision: index as u64 + 1,
                    surface: SurfaceKind::Primary,
                    history_scroll_rows: 1,
                    history_blocks: vec![CommittedHistoryBlock {
                        component_id: format!("sealed-{index}"),
                        source_revision: index as u64 + 1,
                        rows: vec![row("sealed", text)],
                    }],
                    frame_update: FrameUpdate::Full(frame_in_viewport(
                        index as u64 + 1,
                        size,
                        viewport,
                        "live",
                    )),
                    panel: None,
                    cursor: None,
                    full_redraw: true,
                })
                .unwrap();
        }

        let mut vt = VirtualTerminal::new(size);
        vt.feed(driver.output_ref());
        assert_eq!(
            vt.scrollback()
                .iter()
                .filter(|line| line.as_str() == "one")
                .count(),
            1
        );
        assert_eq!(&vt.visible_lines()[0..3], ["two", "three", "four"]);
        assert_eq!(vt.visible_lines()[3], "live");
    }

    #[test]
    fn sealing_a_live_tail_reuses_its_rows_without_scrolling_history_twice() {
        let size = TerminalSize::new(20, 5);
        let mut driver = TerminalDriver::new(Vec::<u8>::new(), capabilities(), size);
        driver
            .commit(&TerminalCommitPlan {
                revision: 1,
                surface: SurfaceKind::Primary,
                history_scroll_rows: 1,
                history_blocks: vec![CommittedHistoryBlock {
                    component_id: "older".to_owned(),
                    source_revision: 1,
                    rows: vec![row("older", "older")],
                }],
                frame_update: FrameUpdate::Full(frame_in_viewport(
                    1,
                    size,
                    Rect::new(0, 4, 20, 1),
                    "footer",
                )),
                panel: None,
                cursor: None,
                full_redraw: true,
            })
            .unwrap();
        driver
            .commit(&TerminalCommitPlan {
                revision: 2,
                surface: SurfaceKind::Primary,
                history_scroll_rows: 0,
                history_blocks: Vec::new(),
                frame_update: FrameUpdate::Full(frame_in_viewport(
                    2,
                    size,
                    Rect::new(0, 2, 20, 3),
                    "stream",
                )),
                panel: None,
                cursor: None,
                full_redraw: true,
            })
            .unwrap();
        driver
            .commit(&TerminalCommitPlan {
                revision: 3,
                surface: SurfaceKind::Primary,
                history_scroll_rows: 2,
                history_blocks: vec![CommittedHistoryBlock {
                    component_id: "final".to_owned(),
                    source_revision: 3,
                    rows: vec![row("final", "final-a"), row("final", "final-b")],
                }],
                frame_update: FrameUpdate::Full(frame_in_viewport(
                    3,
                    size,
                    Rect::new(0, 4, 20, 1),
                    "footer",
                )),
                panel: None,
                cursor: None,
                full_redraw: true,
            })
            .unwrap();

        let mut vt = VirtualTerminal::new(size);
        vt.feed(driver.output_ref());
        assert_eq!(
            vt.visible_lines(),
            ["", "older", "final-a", "final-b", "footer"]
        );
        assert!(
            !vt.scrollback()
                .iter()
                .any(|line| matches!(line.as_str(), "older" | "final-a" | "final-b"))
        );
    }

    #[test]
    fn a_temporary_viewport_expansion_returns_visible_history_to_the_bottom() {
        let size = TerminalSize::new(20, 5);
        let mut driver = TerminalDriver::new(Vec::<u8>::new(), capabilities(), size);
        driver
            .commit(&TerminalCommitPlan {
                revision: 1,
                surface: SurfaceKind::Primary,
                history_scroll_rows: 1,
                history_blocks: vec![CommittedHistoryBlock {
                    component_id: "history".to_owned(),
                    source_revision: 1,
                    rows: vec![row("history", "history")],
                }],
                frame_update: FrameUpdate::Full(frame_in_viewport(
                    1,
                    size,
                    Rect::new(0, 4, 20, 1),
                    "footer",
                )),
                panel: None,
                cursor: None,
                full_redraw: true,
            })
            .unwrap();
        for (revision, viewport) in [(2, Rect::new(0, 1, 20, 4)), (3, Rect::new(0, 4, 20, 1))] {
            driver
                .commit(&TerminalCommitPlan {
                    revision,
                    surface: SurfaceKind::Primary,
                    history_scroll_rows: 0,
                    history_blocks: Vec::new(),
                    frame_update: FrameUpdate::Full(frame_in_viewport(
                        revision, size, viewport, "footer",
                    )),
                    panel: None,
                    cursor: None,
                    full_redraw: true,
                })
                .unwrap();
        }

        let mut vt = VirtualTerminal::new(size);
        vt.feed(driver.output_ref());
        assert_eq!(vt.visible_lines()[3], "history");
        assert_eq!(vt.visible_lines()[4], "footer");
    }

    #[test]
    fn floating_panel_covers_and_restores_history_without_scrolling() {
        let size = TerminalSize::new(20, 5);
        let viewport = Rect::new(0, 4, 20, 1);
        let mut driver = TerminalDriver::new(Vec::<u8>::new(), capabilities(), size);
        driver
            .commit(&TerminalCommitPlan {
                revision: 1,
                surface: SurfaceKind::Primary,
                history_scroll_rows: 1,
                history_blocks: vec![CommittedHistoryBlock {
                    component_id: "history".to_owned(),
                    source_revision: 1,
                    rows: vec![row("history", "history")],
                }],
                frame_update: FrameUpdate::Full(frame_in_viewport(1, size, viewport, "footer")),
                panel: None,
                cursor: None,
                full_redraw: true,
            })
            .unwrap();
        let panel_only_output = driver.output_ref().len();

        driver
            .commit(&TerminalCommitPlan {
                revision: 2,
                surface: SurfaceKind::Primary,
                history_scroll_rows: 0,
                history_blocks: Vec::new(),
                frame_update: FrameUpdate::Full(frame_in_viewport(2, size, viewport, "footer")),
                panel: Some(panel(Rect::new(0, 2, 20, 2), &["first", "second"])),
                cursor: None,
                full_redraw: true,
            })
            .unwrap();
        driver
            .commit(&TerminalCommitPlan {
                revision: 3,
                surface: SurfaceKind::Primary,
                history_scroll_rows: 0,
                history_blocks: Vec::new(),
                frame_update: FrameUpdate::Full(frame_in_viewport(3, size, viewport, "footer")),
                panel: None,
                cursor: None,
                full_redraw: true,
            })
            .unwrap();

        let emitted = &driver.output_ref()[panel_only_output..];
        let emitted = String::from_utf8_lossy(emitted);
        assert!(!emitted.contains("\r\n"));
        assert!(!emitted.contains("\u{1b}M"));
        assert!(!emitted.contains("\u{1b}[1;"));

        let mut vt = VirtualTerminal::new(size);
        vt.feed(driver.output_ref());
        assert_eq!(vt.visible_lines()[3], "history");
        assert_eq!(vt.visible_lines()[4], "footer");
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
                history_scroll_rows: 0,
                history_blocks: Vec::new(),
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
                history_scroll_rows: 0,
                history_blocks: Vec::new(),
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
                history_scroll_rows: 0,
                history_blocks: Vec::new(),
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
                    history_scroll_rows: 0,
                    history_blocks: Vec::new(),
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
        let viewport = Rect::new(0, 2, 20, 2);
        let mut driver = TerminalDriver::new(Vec::<u8>::new(), capabilities(), size);
        driver
            .commit(&TerminalCommitPlan {
                revision: 1,
                surface: SurfaceKind::Primary,
                history_scroll_rows: 1,
                history_blocks: vec![CommittedHistoryBlock {
                    component_id: "history".to_owned(),
                    source_revision: 1,
                    rows: vec![row("history", "history")],
                }],
                frame_update: FrameUpdate::Full(frame_in_viewport(1, size, viewport, "main")),
                panel: Some(panel(Rect::new(0, 1, 20, 1), &["panel"])),
                cursor: None,
                full_redraw: true,
            })
            .unwrap();
        driver
            .commit(&TerminalCommitPlan {
                revision: 2,
                surface: SurfaceKind::Alternate,
                history_scroll_rows: 0,
                history_blocks: Vec::new(),
                frame_update: FrameUpdate::Full(frame(2, size, "browser")),
                panel: None,
                cursor: None,
                full_redraw: true,
            })
            .unwrap();
        driver
            .commit(&TerminalCommitPlan {
                revision: 3,
                surface: SurfaceKind::Primary,
                history_scroll_rows: 0,
                history_blocks: Vec::new(),
                frame_update: FrameUpdate::Full(frame_in_viewport(
                    3,
                    size,
                    viewport,
                    "main restored",
                )),
                panel: None,
                cursor: None,
                full_redraw: true,
            })
            .unwrap();

        let mut vt = VirtualTerminal::new(size);
        vt.feed(driver.output_ref());
        assert!(!vt.in_alternate_screen());
        assert_eq!(vt.visible_lines()[1], "history");
        assert_eq!(vt.visible_lines()[2], "main restored");
    }
}
