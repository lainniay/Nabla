//! Incremental Markdown block scanning.
//!
//! This is deliberately a block scanner rather than a full Markdown parser:
//! its job is to decide which prefix cannot be reinterpreted by later tokens.

use std::{borrow::Cow, collections::VecDeque};

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use unicode_segmentation::UnicodeSegmentation;

use super::{
    palette,
    text::display_width,
    types::{CellStyle, Color, StyledCell, VisualRow},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkdownBlockKind {
    Paragraph,
    List,
    Quote,
    Fence,
    Table,
    Html,
    Heading,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownBlock {
    pub kind: MarkdownBlockKind,
    pub start: usize,
    pub end: usize,
    pub complete: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MarkdownScan {
    pub blocks: Vec<MarkdownBlock>,
    /// UTF-8 byte offset ending at a block boundary.
    pub stable_prefix_bytes: usize,
}

/// Append-oriented scanner state. Once a block has crossed
/// `stable_prefix_bytes`, later updates never scan it again.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IncrementalMarkdown {
    source: String,
    stable_blocks: Vec<MarkdownBlock>,
    stable_prefix_bytes: usize,
    scanned_bytes: usize,
}

impl IncrementalMarkdown {
    pub fn update(&mut self, source: &str, finished: bool) -> MarkdownScan {
        let append = source.starts_with(&self.source)
            && self.stable_prefix_bytes <= source.len()
            && source.is_char_boundary(self.stable_prefix_bytes);
        if !append {
            self.stable_blocks.clear();
            self.stable_prefix_bytes = 0;
        }

        let tail_start = self.stable_prefix_bytes;
        let tail = &source[tail_start..];
        self.scanned_bytes = self.scanned_bytes.saturating_add(tail.len());
        let tail_scan = scan(tail, finished);
        let mut blocks = self.stable_blocks.clone();
        blocks.extend(tail_scan.blocks.into_iter().map(|mut block| {
            block.start += tail_start;
            block.end += tail_start;
            block
        }));
        let stable_prefix_bytes = tail_start.saturating_add(tail_scan.stable_prefix_bytes);

        self.stable_blocks = blocks
            .iter()
            .take_while(|block| block.complete && block.end <= stable_prefix_bytes)
            .cloned()
            .collect();
        self.stable_prefix_bytes = stable_prefix_bytes;
        self.source.clear();
        self.source.push_str(source);

        debug_assert!(source.is_char_boundary(stable_prefix_bytes));
        MarkdownScan {
            blocks,
            stable_prefix_bytes,
        }
    }

    pub fn stable_prefix_bytes(&self) -> usize {
        self.stable_prefix_bytes
    }

    #[cfg(test)]
    fn scanned_bytes(&self) -> usize {
        self.scanned_bytes
    }
}

pub fn scan(source: &str, finished: bool) -> MarkdownScan {
    let lines = line_ranges(source);
    let mut blocks = Vec::new();
    let mut index = 0usize;

    while index < lines.len() {
        let (start, end) = lines[index];
        let line = trim_line(&source[start..end]);
        if line.trim().is_empty() {
            index += 1;
            continue;
        }

        if let Some(marker) = fence_marker(line) {
            let block_start = start;
            index += 1;
            let mut closed = false;
            let mut block_end = end;
            while index < lines.len() {
                let (row_start, row_end) = lines[index];
                block_end = row_end;
                if closes_fence(trim_line(&source[row_start..row_end]), marker) {
                    closed = true;
                    index += 1;
                    break;
                }
                index += 1;
            }
            blocks.push(MarkdownBlock {
                kind: MarkdownBlockKind::Fence,
                start: block_start,
                end: block_end,
                complete: finished || (closed && block_end < source.len()),
            });
            continue;
        }

        if html_block_start(line) {
            let block_start = start;
            let tag = html_tag(line);
            let mut block_end = end;
            let mut closed = tag
                .as_deref()
                .is_some_and(|tag| line.to_ascii_lowercase().contains(&format!("</{tag}>")))
                || line.trim_end().ends_with("-->");
            index += 1;
            while !closed && index < lines.len() {
                let (row_start, row_end) = lines[index];
                let row = trim_line(&source[row_start..row_end]);
                block_end = row_end;
                closed = tag
                    .as_deref()
                    .is_some_and(|tag| row.to_ascii_lowercase().contains(&format!("</{tag}>")))
                    || row.trim_end().ends_with("-->");
                index += 1;
            }
            blocks.push(MarkdownBlock {
                kind: MarkdownBlockKind::Html,
                start: block_start,
                end: block_end,
                complete: finished || (closed && block_end < source.len()),
            });
            continue;
        }

        if index + 1 < lines.len() {
            let (next_start, next_end) = lines[index + 1];
            let next = trim_line(&source[next_start..next_end]);
            if looks_like_table_row(line) && is_table_delimiter(next) {
                let block_start = start;
                let mut block_end = next_end;
                index += 2;
                while index < lines.len() {
                    let (row_start, row_end) = lines[index];
                    let row = trim_line(&source[row_start..row_end]);
                    if row.trim().is_empty() || !looks_like_table_row(row) {
                        break;
                    }
                    block_end = row_end;
                    index += 1;
                }
                let complete =
                    finished || index < lines.len() || source[..block_end].ends_with("\n\n");
                blocks.push(MarkdownBlock {
                    kind: MarkdownBlockKind::Table,
                    start: block_start,
                    end: block_end,
                    complete,
                });
                continue;
            }
        }

        if is_heading(line) {
            blocks.push(MarkdownBlock {
                kind: MarkdownBlockKind::Heading,
                start,
                end,
                complete: finished || end < source.len(),
            });
            index += 1;
            continue;
        }

        let (kind, continuation) = if is_list_item(line) {
            (MarkdownBlockKind::List, Continuation::List)
        } else if is_quote(line) {
            (MarkdownBlockKind::Quote, Continuation::Quote)
        } else {
            (MarkdownBlockKind::Paragraph, Continuation::Paragraph)
        };
        let block_start = start;
        let mut block_end = end;
        index += 1;
        while index < lines.len() {
            let (row_start, row_end) = lines[index];
            let row = trim_line(&source[row_start..row_end]);
            if row.trim().is_empty() {
                break;
            }
            let belongs = match continuation {
                Continuation::List => {
                    is_list_item(row) || row.starts_with("  ") || row.starts_with('\t')
                }
                Continuation::Quote => is_quote(row),
                Continuation::Paragraph => {
                    !is_heading(row)
                        && !is_list_item(row)
                        && !is_quote(row)
                        && fence_marker(row).is_none()
                        && !html_block_start(row)
                }
            };
            if !belongs {
                break;
            }
            block_end = row_end;
            index += 1;
        }
        let followed_by_blank = index < lines.len()
            && trim_line(&source[lines[index].0..lines[index].1])
                .trim()
                .is_empty();
        blocks.push(MarkdownBlock {
            kind,
            start: block_start,
            end: block_end,
            complete: finished || followed_by_blank,
        });
    }

    let mut stable_prefix_bytes = blocks
        .iter()
        .take_while(|block| block.complete)
        .last()
        .map_or(0, |block| block.end);
    if !finished
        && let Some(reference_sensitive) = blocks.iter().find(|block| {
            block.end <= stable_prefix_bytes
                && contains_reference_sensitive_syntax(&source[block.start..block.end])
        })
    {
        // CommonMark reference definitions are document-global: a later
        // `[label]: url` can change how an earlier `[text][label]` or
        // shortcut `[label]` renders. Keep that block and everything after it
        // mutable until the message is sealed.
        stable_prefix_bytes = reference_sensitive.start;
    }
    MarkdownScan {
        blocks,
        stable_prefix_bytes,
    }
}

fn contains_reference_sensitive_syntax(source: &str) -> bool {
    source.lines().any(|line| {
        let mut rest = line;
        while let Some(open) = rest.find('[') {
            rest = &rest[open + 1..];
            let Some(close) = rest.find(']') else {
                break;
            };
            let after = rest[close + 1..].chars().next();
            if after != Some('(') {
                return true;
            }
            rest = &rest[close + 1..];
        }
        false
    })
}

/// Render CommonMark plus the extensions used by Codex into terminal-native
/// rows. Parsing is intentionally repeated for the mutable streaming tail;
/// sealed transcript blocks subsequently move into native terminal history.
pub fn render(
    source: &str,
    component_id: &str,
    width: u16,
    base_style: CellStyle,
) -> Vec<VisualRow> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_FOOTNOTES);

    let normalized = unwrap_markdown_table_fences(source);
    let mut writer = MarkdownWriter::new(component_id, width, base_style);
    for event in Parser::new_ext(&normalized, options) {
        writer.handle_event(event);
    }
    writer.finish()
}

/// LLMs often place a Markdown table inside a `md`/`markdown` code fence.
/// Codex treats such a fence as presentation guidance and renders the table
/// natively. Other code fences and incomplete streaming fences remain intact.
fn unwrap_markdown_table_fences(source: &str) -> Cow<'_, str> {
    if !source.contains("```") && !source.contains("~~~") {
        return Cow::Borrowed(source);
    }
    let ranges = line_ranges(source);
    let mut output = String::with_capacity(source.len());
    let mut changed = false;
    let mut index = 0usize;
    while index < ranges.len() {
        let (start, end) = ranges[index];
        let line = trim_line(&source[start..end]);
        let Some((marker, marker_len, markdown)) = markdown_fence(line) else {
            output.push_str(&source[start..end]);
            index += 1;
            continue;
        };
        if !markdown {
            output.push_str(&source[start..end]);
            index += 1;
            continue;
        }
        let mut close = None;
        for (candidate, &(row_start, row_end)) in ranges.iter().enumerate().skip(index + 1) {
            let row = trim_line(&source[row_start..row_end]).trim();
            let run = row
                .chars()
                .take_while(|character| *character == marker)
                .count();
            if run >= marker_len && row.chars().all(|character| character == marker) {
                close = Some(candidate);
                break;
            }
        }
        let Some(close) = close else {
            output.push_str(&source[start..]);
            break;
        };
        let body_start = ranges[index + 1].0;
        let body_end = ranges[close].0;
        let body = &source[body_start..body_end];
        if contains_table(body) {
            output.push_str(body);
            changed = true;
        } else {
            output.push_str(&source[start..ranges[close].1]);
        }
        index = close + 1;
    }
    if changed {
        Cow::Owned(output)
    } else {
        Cow::Borrowed(source)
    }
}

fn markdown_fence(line: &str) -> Option<(char, usize, bool)> {
    let trimmed = line.trim_start();
    let marker = trimmed.chars().next()?;
    if !matches!(marker, '`' | '~') {
        return None;
    }
    let marker_len = trimmed
        .chars()
        .take_while(|character| *character == marker)
        .count();
    if marker_len < 3 {
        return None;
    }
    let info = trimmed
        .get(marker.len_utf8().saturating_mul(marker_len)..)?
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    Some((
        marker,
        marker_len,
        matches!(info.as_str(), "md" | "markdown"),
    ))
}

fn contains_table(source: &str) -> bool {
    let lines = source.lines().collect::<Vec<_>>();
    lines.windows(2).any(|pair| {
        looks_like_table_row(pair[0]) && !is_table_delimiter(pair[0]) && is_table_delimiter(pair[1])
    })
}

#[derive(Debug, Clone)]
struct RichLine {
    prefix: Vec<StyledCell>,
    continuation_prefix: Vec<StyledCell>,
    content: Vec<StyledCell>,
    wrap: bool,
}

#[derive(Debug, Clone)]
struct IndentContext {
    first: Vec<StyledCell>,
    continuation: Vec<StyledCell>,
    first_pending: bool,
    list_item: bool,
}

#[derive(Debug, Clone, Copy)]
struct ListState {
    next: Option<u64>,
}

#[derive(Debug, Clone)]
struct LinkState {
    destination: String,
}

#[derive(Debug, Clone, Default)]
struct TableCell {
    cells: Vec<StyledCell>,
}

#[derive(Debug)]
struct TableState {
    alignments: Vec<Alignment>,
    header: Option<Vec<TableCell>>,
    rows: Vec<Vec<TableCell>>,
    current_row: Option<Vec<TableCell>>,
    current_cell: Option<TableCell>,
    in_header: bool,
}

impl TableState {
    fn new(alignments: Vec<Alignment>) -> Self {
        Self {
            alignments,
            header: None,
            rows: Vec::new(),
            current_row: None,
            current_cell: None,
            in_header: false,
        }
    }

    fn finish_cell(&mut self) {
        if let Some(cell) = self.current_cell.take() {
            self.current_row.get_or_insert_with(Vec::new).push(cell);
        }
    }

    fn finish_row(&mut self) {
        self.finish_cell();
        let Some(row) = self.current_row.take() else {
            return;
        };
        if self.in_header {
            self.header = Some(row);
        } else {
            self.rows.push(row);
        }
    }
}

struct MarkdownWriter<'a> {
    component_id: &'a str,
    width: u16,
    base_style: CellStyle,
    inline_styles: Vec<CellStyle>,
    indent_stack: Vec<IndentContext>,
    list_stack: Vec<ListState>,
    links: Vec<LinkState>,
    lines: Vec<RichLine>,
    current: Option<RichLine>,
    needs_blank: bool,
    in_code_block: bool,
    code_indent: bool,
    table: Option<TableState>,
}

impl<'a> MarkdownWriter<'a> {
    fn new(component_id: &'a str, width: u16, base_style: CellStyle) -> Self {
        Self {
            component_id,
            width: width.max(1),
            base_style,
            inline_styles: Vec::new(),
            indent_stack: Vec::new(),
            list_stack: Vec::new(),
            links: Vec::new(),
            lines: Vec::new(),
            current: None,
            needs_blank: false,
            in_code_block: false,
            code_indent: false,
            table: None,
        }
    }

    fn finish(mut self) -> Vec<VisualRow> {
        self.flush_line();
        while self.lines.last().is_some_and(|line| {
            line.prefix.is_empty() && line.continuation_prefix.is_empty() && line.content.is_empty()
        }) {
            self.lines.pop();
        }
        let mut rows = Vec::new();
        for (logical_line, line) in self.lines.into_iter().enumerate() {
            let wrapped = wrap_rich_line(line, usize::from(self.width));
            for (wrap_index, cells) in wrapped.into_iter().enumerate() {
                rows.push(VisualRow {
                    component_id: self.component_id.to_owned(),
                    logical_line,
                    wrap_index,
                    cells,
                });
            }
        }
        if rows.is_empty() {
            rows.push(VisualRow::blank(self.component_id));
        }
        rows
    }

    fn handle_event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start_tag(tag),
            Event::End(tag) => self.end_tag(tag),
            Event::Text(text) => self.push_text(&text),
            Event::Code(code) => {
                self.push_styled_text(&code, CellStyle::foreground(palette::SAPPHIRE))
            }
            Event::Html(html) | Event::InlineHtml(html) => self.push_text(&html),
            Event::SoftBreak | Event::HardBreak => {
                if self.in_table_cell() {
                    self.push_table_text(" ");
                } else {
                    self.flush_line();
                }
            }
            Event::Rule => {
                self.begin_block();
                self.push_styled_text("———", CellStyle::foreground(palette::GRAY_FAINT));
                self.flush_line();
                self.needs_blank = true;
            }
            Event::FootnoteReference(label) => {
                self.push_styled_text(
                    &format!("[^{label}]"),
                    CellStyle::foreground(palette::SAPPHIRE),
                );
            }
            Event::TaskListMarker(checked) => {
                self.push_styled_text(
                    if checked { "[x] " } else { "[ ] " },
                    CellStyle::foreground(palette::GRAY_TEXT),
                );
            }
        }
    }

    fn start_tag(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => self.begin_block(),
            Tag::Heading { level, .. } => {
                self.begin_block();
                let style = heading_style(level);
                self.push_styled_text(&format!("{} ", "#".repeat(level as usize)), style);
                self.push_inline_style(style);
            }
            Tag::BlockQuote => {
                self.begin_block();
                let prefix = styled_cells("> ", CellStyle::foreground(palette::GREEN));
                self.indent_stack.push(IndentContext {
                    first: prefix.clone(),
                    continuation: prefix,
                    first_pending: true,
                    list_item: false,
                });
            }
            Tag::CodeBlock(kind) => {
                self.begin_block();
                self.in_code_block = true;
                self.code_indent = matches!(kind, CodeBlockKind::Indented);
                if self.code_indent {
                    let prefix = styled_cells("    ", self.base_style);
                    self.indent_stack.push(IndentContext {
                        first: prefix.clone(),
                        continuation: prefix,
                        first_pending: true,
                        list_item: false,
                    });
                }
                self.push_inline_style(CellStyle::foreground(palette::SAPPHIRE));
            }
            Tag::List(start) => {
                if self.list_stack.is_empty() {
                    self.begin_block();
                } else {
                    self.flush_line();
                }
                self.list_stack.push(ListState { next: start });
            }
            Tag::Item => {
                self.flush_line();
                let marker = match self.list_stack.last_mut() {
                    Some(ListState { next: Some(next) }) => {
                        let marker = format!("{next}. ");
                        *next = next.saturating_add(1);
                        marker
                    }
                    _ => "- ".to_owned(),
                };
                let marker_style = if marker.starts_with('-') {
                    self.base_style
                } else {
                    CellStyle::foreground(palette::BLUE)
                };
                let continuation =
                    styled_cells(&" ".repeat(display_width(&marker)), self.base_style);
                self.indent_stack.push(IndentContext {
                    first: styled_cells(&marker, marker_style),
                    continuation,
                    first_pending: true,
                    list_item: true,
                });
                self.needs_blank = false;
            }
            Tag::Emphasis => self.push_inline_style(CellStyle::default().italic()),
            Tag::Strong => self.push_inline_style(CellStyle::default().bold()),
            Tag::Strikethrough => self.push_inline_style(CellStyle::default().crossed_out()),
            Tag::Link { dest_url, .. } => {
                self.links.push(LinkState {
                    destination: dest_url.to_string(),
                });
                self.push_inline_style(CellStyle::foreground(palette::SAPPHIRE).underlined());
            }
            Tag::Image { .. } => {
                self.push_inline_style(CellStyle::foreground(palette::GRAY_TEXT).italic());
            }
            Tag::Table(alignments) => {
                self.begin_block();
                self.table = Some(TableState::new(alignments));
            }
            Tag::TableHead => {
                if let Some(table) = self.table.as_mut() {
                    table.in_header = true;
                    table.current_row = Some(Vec::new());
                }
            }
            Tag::TableRow => {
                if let Some(table) = self.table.as_mut() {
                    table.current_row = Some(Vec::new());
                }
            }
            Tag::TableCell => {
                if let Some(table) = self.table.as_mut() {
                    table.current_cell = Some(TableCell::default());
                }
            }
            Tag::FootnoteDefinition(label) => {
                self.begin_block();
                let marker = format!("[^{label}]: ");
                let continuation =
                    styled_cells(&" ".repeat(display_width(&marker)), self.base_style);
                self.indent_stack.push(IndentContext {
                    first: styled_cells(&marker, CellStyle::foreground(palette::SAPPHIRE)),
                    continuation,
                    first_pending: true,
                    list_item: false,
                });
            }
            Tag::HtmlBlock => self.begin_block(),
            Tag::MetadataBlock(_) => {}
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.flush_line();
                self.needs_blank = !self.inside_list_item();
            }
            TagEnd::Heading(_) => {
                self.flush_line();
                self.pop_inline_style();
                self.needs_blank = true;
            }
            TagEnd::BlockQuote => {
                self.flush_line();
                self.indent_stack.pop();
                self.needs_blank = true;
            }
            TagEnd::CodeBlock => {
                self.flush_line();
                self.pop_inline_style();
                if self.code_indent {
                    self.indent_stack.pop();
                }
                self.code_indent = false;
                self.in_code_block = false;
                self.needs_blank = true;
            }
            TagEnd::List(_) => {
                self.flush_line();
                self.list_stack.pop();
                self.needs_blank = self.list_stack.is_empty();
            }
            TagEnd::Item => {
                self.flush_line();
                self.indent_stack.pop();
                self.needs_blank = false;
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => self.pop_inline_style(),
            TagEnd::Link => {
                self.pop_inline_style();
                if let Some(link) = self.links.pop()
                    && !link.destination.is_empty()
                {
                    self.push_text(" (");
                    self.push_styled_text(
                        &link.destination,
                        CellStyle::foreground(palette::SAPPHIRE).underlined(),
                    );
                    self.push_text(")");
                }
            }
            TagEnd::Image => self.pop_inline_style(),
            TagEnd::Table => {
                if let Some(mut table) = self.table.take() {
                    table.finish_row();
                    self.render_table(table);
                }
                self.needs_blank = true;
            }
            TagEnd::TableHead => {
                if let Some(table) = self.table.as_mut() {
                    table.finish_row();
                    table.in_header = false;
                }
            }
            TagEnd::TableRow => {
                if let Some(table) = self.table.as_mut() {
                    table.finish_row();
                }
            }
            TagEnd::TableCell => {
                if let Some(table) = self.table.as_mut() {
                    table.finish_cell();
                }
            }
            TagEnd::FootnoteDefinition => {
                self.flush_line();
                self.indent_stack.pop();
                self.needs_blank = true;
            }
            TagEnd::HtmlBlock => {
                self.flush_line();
                self.needs_blank = true;
            }
            TagEnd::MetadataBlock(_) => {}
        }
    }

    fn begin_block(&mut self) {
        self.flush_line();
        if self.needs_blank && !self.inside_list_item() {
            self.push_blank_line();
        }
        self.needs_blank = false;
    }

    fn inside_list_item(&self) -> bool {
        self.indent_stack.iter().any(|context| context.list_item)
    }

    fn in_table_cell(&self) -> bool {
        self.table
            .as_ref()
            .and_then(|table| table.current_cell.as_ref())
            .is_some()
    }

    fn push_text(&mut self, text: &str) {
        self.push_styled_text(text, CellStyle::default());
    }

    fn push_styled_text(&mut self, text: &str, overlay: CellStyle) {
        let style = patch_style(self.current_style(), overlay);
        if self.in_table_cell() {
            self.push_table_cells(styled_cells(text, style));
            return;
        }
        if self.in_code_block {
            self.push_code_text(text, style);
            return;
        }
        self.ensure_line(true);
        if let Some(line) = self.current.as_mut() {
            line.content.extend(styled_cells(text, style));
        }
    }

    fn push_code_text(&mut self, text: &str, style: CellStyle) {
        for (index, line) in text.split('\n').enumerate() {
            if index > 0 {
                self.flush_line();
            }
            if !line.is_empty() {
                self.ensure_line(false);
                if let Some(current) = self.current.as_mut() {
                    current.content.extend(styled_cells(line, style));
                }
            }
        }
    }

    fn push_table_text(&mut self, text: &str) {
        let style = self.current_style();
        self.push_table_cells(styled_cells(text, style));
    }

    fn push_table_cells(&mut self, cells: Vec<StyledCell>) {
        if let Some(table) = self.table.as_mut()
            && let Some(cell) = table.current_cell.as_mut()
        {
            cell.cells.extend(cells);
        }
    }

    fn ensure_line(&mut self, wrap: bool) {
        if self.current.is_some() {
            return;
        }
        let (prefix, continuation_prefix) = self.take_prefixes();
        self.current = Some(RichLine {
            prefix,
            continuation_prefix,
            content: Vec::new(),
            wrap,
        });
    }

    fn take_prefixes(&mut self) -> (Vec<StyledCell>, Vec<StyledCell>) {
        let mut first = Vec::new();
        let mut continuation = Vec::new();
        for context in &mut self.indent_stack {
            if context.first_pending {
                first.extend(context.first.clone());
                context.first_pending = false;
            } else {
                first.extend(context.continuation.clone());
            }
            continuation.extend(context.continuation.clone());
        }
        (first, continuation)
    }

    fn continuation_prefix_width(&self) -> usize {
        self.indent_stack
            .iter()
            .flat_map(|context| &context.continuation)
            .map(|cell| usize::from(cell.width))
            .sum()
    }

    fn flush_line(&mut self) {
        if let Some(line) = self.current.take() {
            self.lines.push(line);
        }
    }

    fn push_blank_line(&mut self) {
        self.flush_line();
        if self.lines.last().is_some_and(|line| {
            line.prefix.is_empty() && line.continuation_prefix.is_empty() && line.content.is_empty()
        }) {
            return;
        }
        let (prefix, continuation_prefix) = self.take_prefixes();
        self.lines.push(RichLine {
            prefix,
            continuation_prefix,
            content: Vec::new(),
            wrap: false,
        });
    }

    fn push_inline_style(&mut self, overlay: CellStyle) {
        let style = patch_style(self.current_style(), overlay);
        self.inline_styles.push(style);
    }

    fn pop_inline_style(&mut self) {
        self.inline_styles.pop();
    }

    fn current_style(&self) -> CellStyle {
        self.inline_styles
            .last()
            .copied()
            .unwrap_or(self.base_style)
    }

    fn render_table(&mut self, table: TableState) {
        let mut header = table.header.unwrap_or_default();
        let column_count = table
            .alignments
            .len()
            .max(header.len())
            .max(table.rows.iter().map(Vec::len).max().unwrap_or(0));
        if column_count == 0 {
            return;
        }
        header.resize_with(column_count, TableCell::default);
        let mut rows = table.rows;
        for row in &mut rows {
            row.resize_with(column_count, TableCell::default);
        }
        let alignments = (0..column_count)
            .map(|index| {
                table
                    .alignments
                    .get(index)
                    .copied()
                    .unwrap_or(Alignment::None)
            })
            .collect::<Vec<_>>();
        let available = usize::from(self.width)
            .saturating_sub(self.continuation_prefix_width())
            .max(1);
        let gap = 2usize;
        let minimum = 3usize;
        let mut widths = (0..column_count)
            .map(|column| {
                std::iter::once(&header[column])
                    .chain(rows.iter().map(|row| &row[column]))
                    .map(|cell| cells_width(&cell.cells))
                    .max()
                    .unwrap_or(0)
                    .max(minimum)
            })
            .collect::<Vec<_>>();
        let gaps = gap.saturating_mul(column_count.saturating_sub(1));
        if column_count.saturating_mul(minimum).saturating_add(gaps) > available {
            self.render_table_as_records(&header, &rows);
            return;
        }
        while widths.iter().sum::<usize>().saturating_add(gaps) > available {
            let Some((index, _)) = widths
                .iter()
                .enumerate()
                .filter(|(_, width)| **width > minimum)
                .max_by_key(|(_, width)| **width)
            else {
                break;
            };
            widths[index] -= 1;
        }

        self.render_table_row(&header, &widths, &alignments, true);
        let mut separator = Vec::new();
        for (index, width) in widths.iter().enumerate() {
            if index > 0 {
                separator.extend(styled_cells(
                    &" ".repeat(gap),
                    CellStyle::foreground(palette::GRAY_FAINT),
                ));
            }
            separator.extend(styled_cells(
                &"━".repeat(*width),
                CellStyle::foreground(palette::GRAY_FAINT),
            ));
        }
        self.push_preformatted(separator);
        for row in &rows {
            self.render_table_row(row, &widths, &alignments, false);
        }
    }

    fn render_table_row(
        &mut self,
        row: &[TableCell],
        widths: &[usize],
        alignments: &[Alignment],
        header: bool,
    ) {
        let wrapped = row
            .iter()
            .zip(widths)
            .map(|(cell, width)| wrap_cells(&cell.cells, *width))
            .collect::<Vec<_>>();
        let height = wrapped.iter().map(Vec::len).max().unwrap_or(1);
        for line_index in 0..height {
            let mut output = Vec::new();
            for column in 0..widths.len() {
                if column > 0 {
                    output.extend(styled_cells("  ", self.base_style));
                }
                let mut cells = wrapped[column].get(line_index).cloned().unwrap_or_default();
                if header {
                    for cell in &mut cells {
                        cell.style = patch_style(cell.style, CellStyle::default().bold());
                    }
                }
                output.extend(align_cells(
                    cells,
                    widths[column],
                    alignments[column],
                    self.base_style,
                ));
            }
            self.push_preformatted(output);
        }
    }

    fn render_table_as_records(&mut self, header: &[TableCell], rows: &[Vec<TableCell>]) {
        for (row_index, row) in rows.iter().enumerate() {
            if row_index > 0 {
                self.push_preformatted(styled_cells(
                    &"─".repeat(usize::from(self.width).min(12)),
                    CellStyle::foreground(palette::GRAY_FAINT),
                ));
            }
            for (column, value) in row.iter().enumerate() {
                let key = plain_cells(
                    header
                        .get(column)
                        .map(|cell| cell.cells.as_slice())
                        .unwrap_or_default(),
                );
                let mut line = styled_cells(
                    &format!("{}: ", if key.is_empty() { column + 1 } else { 0 }),
                    CellStyle::foreground(palette::GRAY_TEXT).bold(),
                );
                if !key.is_empty() {
                    line = styled_cells(
                        &format!("{key}: "),
                        CellStyle::foreground(palette::GRAY_TEXT).bold(),
                    );
                }
                line.extend(value.cells.clone());
                self.ensure_line(true);
                if let Some(current) = self.current.as_mut() {
                    current.content.extend(line);
                }
                self.flush_line();
            }
        }
    }

    fn push_preformatted(&mut self, cells: Vec<StyledCell>) {
        self.flush_line();
        let (prefix, continuation_prefix) = self.take_prefixes();
        self.lines.push(RichLine {
            prefix,
            continuation_prefix,
            content: cells,
            wrap: false,
        });
    }
}

fn heading_style(level: HeadingLevel) -> CellStyle {
    match level {
        HeadingLevel::H1 => CellStyle::default().bold(),
        HeadingLevel::H2 => CellStyle::default().bold(),
        HeadingLevel::H3 => CellStyle::default().bold().italic(),
        HeadingLevel::H4 | HeadingLevel::H5 | HeadingLevel::H6 => CellStyle::default().italic(),
    }
}

fn patch_style(base: CellStyle, overlay: CellStyle) -> CellStyle {
    CellStyle {
        foreground: if overlay.foreground == Color::Default {
            base.foreground
        } else {
            overlay.foreground
        },
        background: if overlay.background == Color::Default {
            base.background
        } else {
            overlay.background
        },
        bold: base.bold || overlay.bold,
        dim: base.dim || overlay.dim,
        italic: base.italic || overlay.italic,
        underlined: base.underlined || overlay.underlined,
        crossed_out: base.crossed_out || overlay.crossed_out,
        reversed: base.reversed || overlay.reversed,
    }
}

fn styled_cells(text: &str, style: CellStyle) -> Vec<StyledCell> {
    text.graphemes(true)
        .map(|grapheme| {
            let width = display_width(grapheme).max(1);
            StyledCell::new(grapheme, u16::try_from(width).unwrap_or(u16::MAX), style)
        })
        .collect()
}

fn cells_width(cells: &[StyledCell]) -> usize {
    cells.iter().map(|cell| usize::from(cell.width)).sum()
}

fn plain_cells(cells: &[StyledCell]) -> String {
    cells.iter().map(|cell| cell.symbol.as_str()).collect()
}

fn wrap_rich_line(line: RichLine, width: usize) -> Vec<Vec<StyledCell>> {
    if line.content.is_empty() {
        return vec![clip_cells(line.prefix, width)];
    }
    if !line.wrap {
        let mut cells = line.prefix;
        cells.extend(line.content);
        return vec![clip_cells(cells, width)];
    }

    let first_width = width.saturating_sub(cells_width(&line.prefix)).max(1);
    let next_width = width
        .saturating_sub(cells_width(&line.continuation_prefix))
        .max(1);
    let mut remaining = VecDeque::from(line.content);
    let mut rows = Vec::new();
    let mut first = true;
    while !remaining.is_empty() {
        let prefix = if first {
            line.prefix.clone()
        } else {
            line.continuation_prefix.clone()
        };
        let available = if first { first_width } else { next_width };
        let content = take_wrapped_chunk(&mut remaining, available);
        let mut row = prefix;
        row.extend(content);
        rows.push(clip_cells(row, width));
        first = false;
    }
    rows
}

fn wrap_cells(cells: &[StyledCell], width: usize) -> Vec<Vec<StyledCell>> {
    if cells.is_empty() {
        return vec![Vec::new()];
    }
    let mut remaining = VecDeque::from(cells.to_vec());
    let mut lines = Vec::new();
    while !remaining.is_empty() {
        lines.push(take_wrapped_chunk(&mut remaining, width.max(1)));
    }
    lines
}

fn take_wrapped_chunk(remaining: &mut VecDeque<StyledCell>, width: usize) -> Vec<StyledCell> {
    while remaining
        .front()
        .is_some_and(|cell| cell.symbol.chars().all(char::is_whitespace))
    {
        remaining.pop_front();
    }
    let mut chunk = Vec::new();
    let mut used = 0usize;
    let mut last_space = None;
    for cell in remaining.iter() {
        let next = used.saturating_add(usize::from(cell.width));
        if !chunk.is_empty() && next > width {
            break;
        }
        used = next;
        chunk.push(cell.clone());
        if cell.symbol.chars().all(char::is_whitespace) {
            last_space = Some(chunk.len() - 1);
        }
        if used >= width {
            break;
        }
    }
    if chunk.len() < remaining.len()
        && let Some(space) = last_space
        && space > 0
    {
        let ends_inside_word = chunk
            .last()
            .is_some_and(|cell| !cell.symbol.chars().all(char::is_whitespace))
            && remaining
                .get(chunk.len())
                .is_some_and(|cell| !cell.symbol.chars().all(char::is_whitespace));
        if used < width || ends_inside_word {
            chunk.truncate(space);
        }
    }
    let consumed = chunk.len();
    for _ in 0..consumed.max(1) {
        remaining.pop_front();
    }
    while chunk
        .last()
        .is_some_and(|cell| cell.symbol.chars().all(char::is_whitespace))
    {
        chunk.pop();
    }
    chunk
}

fn clip_cells(cells: Vec<StyledCell>, width: usize) -> Vec<StyledCell> {
    let mut used = 0usize;
    cells
        .into_iter()
        .take_while(|cell| {
            let next = used.saturating_add(usize::from(cell.width));
            if next > width {
                false
            } else {
                used = next;
                true
            }
        })
        .collect()
}

fn align_cells(
    cells: Vec<StyledCell>,
    width: usize,
    alignment: Alignment,
    style: CellStyle,
) -> Vec<StyledCell> {
    let content_width = cells_width(&cells).min(width);
    let remaining = width.saturating_sub(content_width);
    let (left, right) = match alignment {
        Alignment::Right => (remaining, 0),
        Alignment::Center => (remaining / 2, remaining - remaining / 2),
        Alignment::Left | Alignment::None => (0, remaining),
    };
    let mut output = styled_cells(&" ".repeat(left), style);
    output.extend(clip_cells(cells, width));
    output.extend(styled_cells(&" ".repeat(right), style));
    output
}

#[derive(Debug, Clone, Copy)]
enum Continuation {
    List,
    Quote,
    Paragraph,
}

fn line_ranges(source: &str) -> Vec<(usize, usize)> {
    let mut rows = Vec::new();
    let mut start = 0usize;
    for (index, character) in source.char_indices() {
        if character == '\n' {
            rows.push((start, index + 1));
            start = index + 1;
        }
    }
    if start < source.len() || source.is_empty() {
        rows.push((start, source.len()));
    }
    rows
}

fn trim_line(line: &str) -> &str {
    line.trim_end_matches(['\r', '\n'])
}

fn fence_marker(line: &str) -> Option<char> {
    let trimmed = line.trim_start();
    let marker = trimmed.chars().next()?;
    if !matches!(marker, '`' | '~') {
        return None;
    }
    (trimmed
        .chars()
        .take_while(|character| *character == marker)
        .count()
        >= 3)
        .then_some(marker)
}

fn closes_fence(line: &str, marker: char) -> bool {
    let trimmed = line.trim();
    trimmed.chars().all(|character| character == marker) && trimmed.chars().count() >= 3
}

fn is_heading(line: &str) -> bool {
    let trimmed = line.trim_start();
    let hashes = trimmed
        .chars()
        .take_while(|character| *character == '#')
        .count();
    (1..=6).contains(&hashes) && trimmed.chars().nth(hashes) == Some(' ')
}

fn is_list_item(line: &str) -> bool {
    let trimmed = line.trim_start();
    if ["- ", "* ", "+ "]
        .iter()
        .any(|marker| trimmed.starts_with(marker))
    {
        return true;
    }
    let digits = trimmed
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .count();
    digits > 0
        && trimmed
            .get(digits..)
            .is_some_and(|tail| tail.starts_with(". ") || tail.starts_with(") "))
}

fn is_quote(line: &str) -> bool {
    line.trim_start().starts_with('>')
}

fn looks_like_table_row(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.contains('|')
        && trimmed
            .split('|')
            .filter(|cell| !cell.trim().is_empty())
            .count()
            >= 2
}

fn is_table_delimiter(line: &str) -> bool {
    let trimmed = line.trim().trim_matches('|');
    let cells = trimmed.split('|').map(str::trim).collect::<Vec<_>>();
    cells.len() >= 2
        && cells.iter().all(|cell| {
            let cell = cell.trim_matches(':');
            cell.len() >= 3 && cell.chars().all(|character| character == '-')
        })
}

fn html_block_start(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("<!--")
        || html_tag(trimmed).is_some_and(|tag| {
            matches!(
                tag.as_str(),
                "address"
                    | "article"
                    | "aside"
                    | "blockquote"
                    | "details"
                    | "dialog"
                    | "div"
                    | "footer"
                    | "header"
                    | "main"
                    | "nav"
                    | "pre"
                    | "script"
                    | "section"
                    | "style"
                    | "table"
            )
        })
}

fn html_tag(line: &str) -> Option<String> {
    let trimmed = line.trim_start().strip_prefix('<')?;
    if trimmed.starts_with(['!', '?', '/']) {
        return None;
    }
    let tag = trimmed
        .chars()
        .take_while(|character| character.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    (!tag.is_empty()).then_some(tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unfinished_paragraph_stays_mutable_until_a_blank_line() {
        let partial = scan("Hello | world", false);
        assert_eq!(partial.blocks[0].kind, MarkdownBlockKind::Paragraph);
        assert_eq!(partial.stable_prefix_bytes, 0);

        let stable = scan("Hello | world\n\nnext", false);
        assert!(stable.stable_prefix_bytes >= "Hello | world\n".len());
    }

    #[test]
    fn stable_paragraph_is_not_reparsed_after_tail_growth() {
        let mut incremental = IncrementalMarkdown::default();
        let first = "stable paragraph\n\nmutable";
        let scan = incremental.update(first, false);
        assert_eq!(scan.stable_prefix_bytes, "stable paragraph\n".len());
        let scanned = incremental.scanned_bytes();

        let second = "stable paragraph\n\nmutable tail";
        incremental.update(second, false);
        assert_eq!(
            incremental.scanned_bytes() - scanned,
            second.len() - "stable paragraph\n".len()
        );
    }

    #[test]
    fn unfinished_paragraph_remains_mutable() {
        let mut incremental = IncrementalMarkdown::default();
        assert_eq!(
            incremental
                .update("still growing", false)
                .stable_prefix_bytes,
            0
        );
    }

    #[test]
    fn unfinished_fenced_code_remains_mutable() {
        let mut incremental = IncrementalMarkdown::default();
        assert_eq!(
            incremental
                .update("```rust\nfn main()", false)
                .stable_prefix_bytes,
            0
        );
    }

    #[test]
    fn table_rows_remain_mutable_until_table_end() {
        let mut incremental = IncrementalMarkdown::default();
        let table = "| a | b |\n|---|---|\n| 1 | 2 |";
        assert_eq!(incremental.update(table, false).stable_prefix_bytes, 0);
        assert_eq!(
            incremental
                .update(&format!("{table}\n\nnext"), false)
                .stable_prefix_bytes,
            table.len() + 1
        );
    }

    #[test]
    fn utf8_stable_boundary_is_valid() {
        let source = "你好，世界\n\n尾部";
        let mut incremental = IncrementalMarkdown::default();
        let scan = incremental.update(source, false);
        assert!(source.is_char_boundary(scan.stable_prefix_bytes));
        assert_eq!(&source[..scan.stable_prefix_bytes], "你好，世界\n");
    }

    #[test]
    fn completion_seals_remaining_tail() {
        let mut incremental = IncrementalMarkdown::default();
        let source = "unfinished paragraph";
        assert_eq!(incremental.update(source, false).stable_prefix_bytes, 0);
        assert_eq!(
            incremental.update(source, true).stable_prefix_bytes,
            source.len()
        );
    }

    #[test]
    fn fences_tables_lists_quotes_and_html_have_structural_completion() {
        let source = concat!(
            "```rust\nfn main() {}\n```\n\n",
            "| a | b |\n|---|---|\n| 1 | 2 |\n\n",
            "- one\n  continuation\n\n",
            "> quote\n> continued\n\n",
            "<div>\nbody\n</div>\n\n"
        );
        let result = scan(source, false);
        assert!(result.blocks.iter().all(|block| block.complete));
        assert_eq!(
            result
                .blocks
                .iter()
                .map(|block| block.kind)
                .collect::<Vec<_>>(),
            vec![
                MarkdownBlockKind::Fence,
                MarkdownBlockKind::Table,
                MarkdownBlockKind::List,
                MarkdownBlockKind::Quote,
                MarkdownBlockKind::Html,
            ]
        );
    }

    #[test]
    fn stable_prefix_rows_are_suffix_invariant_across_markdown_structures() {
        let cases = [
            ("- item\n  continuation\n\nmutable", " tail"),
            ("> quote\n> continuation\n\nmutable", " tail"),
            ("```rust\nfn main() {}\n```\n\nmutable", " tail"),
            ("| a | b |\n|---|---|\n| 1 | 2 |\n\nmutable", " tail"),
            ("<div>\nbody\n</div>\n\nmutable", " tail"),
            ("你好 👩🏽‍💻 wide text\n\nmutable", " 尾部"),
        ];
        for (prefix, suffix) in cases {
            let mut incremental = IncrementalMarkdown::default();
            let scan = incremental.update(prefix, false);
            assert!(
                scan.stable_prefix_bytes > 0,
                "case did not expose a stable prefix: {prefix:?}"
            );
            let stable = render(
                &prefix[..scan.stable_prefix_bytes],
                "suffix-invariance",
                18,
                CellStyle::foreground(Color::White),
            );
            let full = render(
                &format!("{prefix}{suffix}"),
                "suffix-invariance",
                18,
                CellStyle::foreground(Color::White),
            );
            assert_eq!(
                full.get(..stable.len()),
                Some(stable.as_slice()),
                "stable rows changed after suffix growth for {prefix:?}"
            );
        }
    }

    #[test]
    fn reference_style_links_remain_mutable_until_completion() {
        let source = "Earlier [documentation][docs].\n\nmutable";
        let partial = scan(source, false);
        assert_eq!(partial.stable_prefix_bytes, 0);

        let completed = format!("{source}\n\n[docs]: https://example.com");
        assert_eq!(scan(&completed, true).stable_prefix_bytes, completed.len());
    }

    #[test]
    fn unterminated_structures_only_seal_when_the_message_finishes() {
        assert_eq!(scan("```\npartial", false).stable_prefix_bytes, 0);
        assert_eq!(scan("<div>\npartial", false).stable_prefix_bytes, 0);
        assert!(scan("```\npartial", true).stable_prefix_bytes > 0);
    }

    fn style_for(rows: &[VisualRow], needle: &str) -> Option<CellStyle> {
        let symbols = needle.graphemes(true).collect::<Vec<_>>();
        rows.iter().find_map(|row| {
            row.cells.windows(symbols.len()).find_map(|window| {
                window
                    .iter()
                    .zip(&symbols)
                    .all(|(cell, symbol)| cell.symbol == *symbol)
                    .then_some(window[0].style)
            })
        })
    }

    #[test]
    fn renders_codex_style_blocks_and_inline_formatting() {
        let source = concat!(
            "# Heading\n\n",
            "Text with **bold**, *italic*, ~~removed~~, `code`, and [docs](https://example.com).\n\n",
            "> quoted **text**\n\n",
            "- [x] complete\n",
            "  - nested item\n\n",
            "1. ordered\n\n",
            "---\n\n",
            "```rust\nfn main() {}\n```\n"
        );
        let rows = render(source, "markdown", 80, CellStyle::foreground(Color::White));
        let text = rows
            .iter()
            .map(VisualRow::plain_text)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("# Heading"));
        assert!(text.contains("> quoted text"));
        assert!(text.contains("- [x] complete"));
        assert!(text.contains("  - nested item"));
        assert!(text.contains("1. ordered"));
        assert!(text.contains("———"));
        assert!(text.contains("fn main() {}"));
        assert!(!text.contains("```"));
        assert!(text.contains("docs (https://example.com)"));
        assert!(style_for(&rows, "Heading").unwrap().bold);
        assert!(style_for(&rows, "bold").unwrap().bold);
        assert!(style_for(&rows, "italic").unwrap().italic);
        assert!(style_for(&rows, "removed").unwrap().crossed_out);
        assert_eq!(
            style_for(&rows, "code").unwrap().foreground,
            palette::SAPPHIRE
        );
        assert!(style_for(&rows, "https://example.com").unwrap().underlined);
    }

    #[test]
    fn wraps_lists_and_quotes_with_stable_continuation_indentation() {
        let list = render(
            "- first second third fourth",
            "list",
            14,
            CellStyle::foreground(Color::White),
        );
        assert_eq!(
            list.iter().map(VisualRow::plain_text).collect::<Vec<_>>(),
            vec!["- first second", "  third fourth"]
        );

        let quote = render(
            "> block quote with content that wraps",
            "quote",
            18,
            CellStyle::foreground(Color::White),
        );
        assert!(
            quote
                .iter()
                .skip(1)
                .all(|row| row.plain_text().starts_with("> "))
        );
        assert!(
            list.iter()
                .chain(&quote)
                .all(|row| row.display_width() <= 18)
        );
    }

    #[test]
    fn renders_tables_as_columns_or_width_safe_records() {
        let source = "| Name | State |\n|:-----|------:|\n| parser | ready |\n";
        let wide = render(source, "table", 40, CellStyle::foreground(Color::White));
        let wide_text = wide
            .iter()
            .map(VisualRow::plain_text)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(wide_text.contains("Name"));
        assert!(wide_text.contains("━━━━"));
        assert!(wide_text.contains("parser"));

        let narrow = render(
            "| Path | Description | State |\n|---|---|---|\n| src/ui/markdown.rs | Markdown renderer | ready |\n",
            "table",
            12,
            CellStyle::foreground(Color::White),
        );
        let narrow_text = narrow
            .iter()
            .map(VisualRow::plain_text)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(narrow_text.contains("Path:"));
        assert!(narrow_text.contains("Description:"));
        assert!(narrow.iter().all(|row| row.display_width() <= 12));
    }

    #[test]
    fn markdown_fenced_tables_render_natively_like_codex() {
        let rows = render(
            "```markdown\n| Key | Value |\n|---|---|\n| mode | fast |\n```\n",
            "table",
            40,
            CellStyle::foreground(Color::White),
        );
        let text = rows
            .iter()
            .map(VisualRow::plain_text)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("Key"));
        assert!(text.contains("━━━━"));
        assert!(text.contains("mode"));
        assert!(!text.contains("```"));

        let incomplete = unwrap_markdown_table_fences("```markdown\n| Key | Value |\n|---|---|\n");
        assert!(incomplete.starts_with("```markdown"));
    }

    #[test]
    fn renders_setext_images_html_entities_and_footnotes() {
        let rows = render(
            concat!(
                "Setext title\n============\n\n",
                "Image: ![diagram](diagram.png) and &amp; entity.  \n",
                "hard break[^note]\n\n",
                "<div>literal html</div>\n\n",
                "[^note]: footnote **body**\n"
            ),
            "extended",
            60,
            CellStyle::foreground(Color::White),
        );
        let text = rows
            .iter()
            .map(VisualRow::plain_text)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("# Setext title"));
        assert!(text.contains("Image: diagram and & entity."));
        assert!(text.contains("hard break[^note]"));
        assert!(text.contains("<div>literal html</div>"));
        assert!(text.contains("[^note]: footnote body"));
        assert!(!style_for(&rows, "Setext title").unwrap().underlined);
        assert!(style_for(&rows, "body").unwrap().bold);
    }

    #[test]
    fn incomplete_streaming_markdown_degrades_without_losing_text() {
        for source in ["**partial", "[link](https://example", "```rust\nfn main("] {
            let rows = render(source, "stream", 32, CellStyle::foreground(Color::White));
            assert!(!rows.is_empty());
            assert!(rows.iter().all(|row| row.display_width() <= 32));
        }
    }
}
