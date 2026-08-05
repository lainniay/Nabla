use super::*;

#[derive(Debug, Clone, PartialEq)]
pub enum TranscriptItem {
    User(UserMessage),
    Assistant(AssistantMessage),
    Tool(ToolExecution),
    Plan(PlanArtifact),
    Context(ContextSnapshot),
    Resources(ResourceSnapshot),
    Agents(AgentsSnapshot),
    Subagent(SubagentTranscript),
    Compaction(CompactionRecord),
    TurnSeparator(TurnSeparator),
    BranchSummary(String),
    SessionBoundary {
        action: String,
        label: String,
        cwd: String,
    },
    Notice(String),
    Error(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditorState {
    text: String,
    /// Cursor position in extended grapheme clusters, never bytes or scalar values.
    cursor: usize,
    preferred_visual_column: Option<usize>,
}

impl EditorState {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn insert_char(&mut self, character: char) {
        let byte_index = self.byte_index();
        self.text.insert(byte_index, character);
        self.cursor = self.grapheme_index_after_byte(byte_index + character.len_utf8());
        self.preferred_visual_column = None;
    }

    pub fn insert_text(&mut self, text: &str) {
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        let byte_index = self.byte_index();
        self.text.insert_str(byte_index, &normalized);
        self.cursor = self.grapheme_index_after_byte(byte_index + normalized.len());
        self.preferred_visual_column = None;
    }

    pub fn insert_newline(&mut self) {
        self.insert_text("\n");
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let end = self.byte_index();
        self.cursor -= 1;
        let start = self.byte_index();
        self.text.replace_range(start..end, "");
        self.preferred_visual_column = None;
    }

    pub fn delete(&mut self) {
        if self.cursor == self.grapheme_count() {
            return;
        }
        let start = self.byte_index();
        self.cursor += 1;
        let end = self.byte_index();
        self.cursor -= 1;
        self.text.replace_range(start..end, "");
        self.preferred_visual_column = None;
    }

    pub fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
        self.preferred_visual_column = None;
    }

    pub fn move_right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.grapheme_count());
        self.preferred_visual_column = None;
    }

    pub fn move_home(&mut self) {
        let byte = self.byte_index();
        let line_start = self.text[..byte].rfind('\n').map_or(0, |index| index + 1);
        self.cursor = self.grapheme_index_after_byte(line_start);
        self.preferred_visual_column = None;
    }

    pub fn move_end(&mut self) {
        let byte = self.byte_index();
        let line_end = self.text[byte..]
            .find('\n')
            .map_or(self.text.len(), |index| byte + index);
        self.cursor = self.grapheme_index_after_byte(line_end);
        self.preferred_visual_column = None;
    }

    pub fn move_up(&mut self, width: usize) {
        self.move_vertical(width, -1);
    }

    pub fn move_down(&mut self, width: usize) {
        self.move_vertical(width, 1);
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.preferred_visual_column = None;
    }

    pub(crate) fn replace(&mut self, text: String) {
        self.cursor = text.graphemes(true).count();
        self.text = text;
        self.preferred_visual_column = None;
    }

    pub fn replace_byte_range(&mut self, range: std::ops::Range<usize>, replacement: &str) {
        if range.start > range.end
            || range.end > self.text.len()
            || !self.text.is_char_boundary(range.start)
            || !self.text.is_char_boundary(range.end)
        {
            return;
        }
        self.text.replace_range(range.clone(), replacement);
        self.cursor = self.text[..range.start + replacement.len()]
            .graphemes(true)
            .count();
        self.preferred_visual_column = None;
    }

    pub(crate) fn take(&mut self) -> String {
        self.cursor = 0;
        self.preferred_visual_column = None;
        std::mem::take(&mut self.text)
    }

    fn byte_index(&self) -> usize {
        self.text
            .grapheme_indices(true)
            .nth(self.cursor)
            .map_or(self.text.len(), |(index, _)| index)
    }

    fn grapheme_count(&self) -> usize {
        self.text.graphemes(true).count()
    }

    fn grapheme_index_after_byte(&self, byte: usize) -> usize {
        self.text[..byte.min(self.text.len())]
            .graphemes(true)
            .count()
    }

    fn visual_positions(&self, width: usize) -> Vec<(usize, usize)> {
        let width = width.max(1);
        let mut row = 0usize;
        let mut column = 0usize;
        let mut positions = vec![(row, column)];
        for grapheme in self.text.graphemes(true) {
            if grapheme == "\n" {
                row += 1;
                column = 0;
            } else {
                let grapheme_width = UnicodeWidthStr::width(grapheme);
                if column > 0 && column.saturating_add(grapheme_width) > width {
                    row += 1;
                    column = 0;
                }
                column = column.saturating_add(grapheme_width);
                if column >= width {
                    row += column / width;
                    column %= width;
                }
            }
            positions.push((row, column));
        }
        positions
    }

    fn move_vertical(&mut self, width: usize, delta: isize) {
        let positions = self.visual_positions(width);
        let (row, column) = positions.get(self.cursor).copied().unwrap_or_default();
        let preferred = self.preferred_visual_column.unwrap_or(column);
        self.preferred_visual_column = Some(preferred);
        let target_row = if delta < 0 {
            row.saturating_sub(delta.unsigned_abs())
        } else {
            row.saturating_add(delta as usize)
        };
        if target_row == row {
            return;
        }
        let candidates = positions
            .iter()
            .enumerate()
            .filter(|(_, (candidate_row, _))| *candidate_row == target_row);
        if let Some((index, _)) =
            candidates.min_by_key(|(_, (_, candidate_column))| candidate_column.abs_diff(preferred))
        {
            self.cursor = index;
        }
    }
}
