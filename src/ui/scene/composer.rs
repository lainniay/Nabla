use crate::state::{AuthPromptKind, AuthState, TreePhase, UiModalKind};
use crate::ui::{
    palette,
    scene::{
        COMPOSER_CHROME_HEIGHT, append_text_cells, cells_width, composer_border_row,
        input_border_row, text_row, view_model::SceneViewModel,
    },
    text::{truncate, wrap_file_references, wrap_text},
    types::{CellStyle, Color, StyledCell, VisualRow},
};
use unicode_segmentation::UnicodeSegmentation;

pub(crate) struct ComposerRender {
    pub(crate) rows: Vec<VisualRow>,
    pub(crate) first_content_row: usize,
    pub(crate) content_column: u16,
    pub(crate) content_row: u16,
}

pub(crate) fn composer_content_width(width: u16) -> u16 {
    width.saturating_sub(4).max(1)
}

pub(crate) fn composer_rows(
    view: &SceneViewModel,
    width: u16,
    height: u16,
    cursor_row: usize,
) -> ComposerRender {
    let accent = if *view.plan_mode_active {
        CellStyle::foreground(Color::Magenta)
    } else {
        palette::input_border()
    };
    if height < 3 || width < 4 {
        let mut rows = wrap_file_references(
            "composer",
            view.editor.text(),
            width.saturating_sub(2).max(1),
            CellStyle::foreground(palette::TEXT),
        );
        rows.truncate(usize::from(height.max(1)));
        for row in &mut rows {
            let mut cells = vec![
                StyledCell::new("›", 1, accent),
                StyledCell::new(" ", 1, accent),
            ];
            cells.extend(std::mem::take(&mut row.cells));
            row.cells = cells;
        }
        return ComposerRender {
            rows,
            first_content_row: 0,
            content_column: 2,
            content_row: 0,
        };
    }

    let border = if *view.plan_mode_active {
        CellStyle::foreground(Color::Magenta).bold()
    } else {
        palette::input_border()
    };
    let text_style = CellStyle::foreground(palette::TEXT);
    let content_width = composer_content_width(width);
    let mut content =
        wrap_file_references("composer", view.editor.text(), content_width, text_style);
    while content.len() <= cursor_row {
        content.push(VisualRow::blank("composer"));
    }
    let content_capacity = usize::from(height.saturating_sub(COMPOSER_CHROME_HEIGHT).max(1));
    let maximum_start = content.len().saturating_sub(content_capacity);
    let first_content_row = cursor_row
        .saturating_sub(content_capacity.saturating_sub(1))
        .min(maximum_start);
    let visible_content =
        &content[first_content_row..content.len().min(first_content_row + content_capacity)];

    let mut rows = Vec::with_capacity(visible_content.len().saturating_add(2));
    rows.push(composer_border_row("╭", "╮", width, border));
    for (visible_index, row) in visible_content.iter().enumerate() {
        let mut cells = vec![StyledCell::new("│", 1, border)];
        if first_content_row + visible_index == 0 {
            cells.push(StyledCell::new("›", 1, accent));
            cells.push(StyledCell::new(" ", 1, accent));
        } else {
            cells.push(StyledCell::new("  ", 2, text_style));
        }

        if view.editor.text().is_empty() && first_content_row + visible_index == 0 {
            let placeholder = if *view.plan_mode_active {
                "Describe the plan you want Nabla to make"
            } else {
                "Ask Nabla to work on your code"
            };
            append_text_cells(
                &mut cells,
                &truncate(placeholder, usize::from(content_width)),
                CellStyle::foreground(palette::GRAY_MUTED),
            );
        } else {
            cells.extend(row.cells.clone());
        }
        let used = cells
            .iter()
            .fold(0u16, |total, cell| total.saturating_add(cell.width));
        let padding = width.saturating_sub(used).saturating_sub(1);
        if padding > 0 {
            cells.push(StyledCell::new(
                " ".repeat(usize::from(padding)),
                padding,
                text_style,
            ));
        }
        cells.push(StyledCell::new("│", 1, border));
        rows.push(VisualRow {
            component_id: "composer".to_owned(),
            logical_line: first_content_row + visible_index,
            wrap_index: row.wrap_index,
            cells,
        });
    }
    rows.push(composer_border_row("╰", "╯", width, border));
    ComposerRender {
        rows,
        first_content_row,
        content_column: 3,
        content_row: 1,
    }
}

pub(crate) struct AlternateInputModel {
    pub(crate) text: String,
    pub(crate) cursor: usize,
    pub(crate) placeholder: String,
    pub(crate) focused: bool,
    pub(crate) secret: bool,
}

impl AlternateInputModel {
    pub(crate) fn display_text(&self) -> String {
        if self.secret {
            "•".repeat(self.text.graphemes(true).count())
        } else {
            self.text.clone()
        }
    }
}

pub(crate) fn alternate_input_model(view: &SceneViewModel) -> AlternateInputModel {
    match view.active_modal_kind() {
        Some(UiModalKind::SessionBrowser) => view.session_browser.as_ref().map_or_else(
            || alternate_placeholder("Search sessions", false),
            |browser| AlternateInputModel {
                text: browser.query.text().to_owned(),
                cursor: browser.query.cursor(),
                placeholder: "Search sessions".to_owned(),
                focused: browser.search_active,
                secret: false,
            },
        ),
        Some(UiModalKind::TreeBrowser) => view.tree_browser.as_ref().map_or_else(
            || alternate_placeholder("Search tree", false),
            |browser| match &browser.phase {
                TreePhase::EditLabel { editor, .. } => AlternateInputModel {
                    text: editor.text().to_owned(),
                    cursor: editor.cursor(),
                    placeholder: "Edit branch label".to_owned(),
                    focused: true,
                    secret: false,
                },
                TreePhase::CustomSummary { editor, .. } => AlternateInputModel {
                    text: editor.text().to_owned(),
                    cursor: editor.cursor(),
                    placeholder: "Describe the branch summary".to_owned(),
                    focused: true,
                    secret: false,
                },
                _ => AlternateInputModel {
                    text: browser.query.text().to_owned(),
                    cursor: browser.query.cursor(),
                    placeholder: "Search tree".to_owned(),
                    focused: browser.search_active,
                    secret: false,
                },
            },
        ),
        Some(UiModalKind::Transcript) => view.transcript_viewer.as_ref().map_or_else(
            || alternate_placeholder("Search transcript", false),
            |viewer| AlternateInputModel {
                text: viewer.search_query.text().to_owned(),
                cursor: viewer.search_query.cursor(),
                placeholder: "Search transcript".to_owned(),
                focused: viewer.search_active,
                secret: false,
            },
        ),
        Some(UiModalKind::Auth) => match &view.auth_state {
            AuthState::Selecting {
                filter,
                search_active,
                ..
            } => AlternateInputModel {
                text: filter.text().to_owned(),
                cursor: filter.cursor(),
                placeholder: "Search login providers".to_owned(),
                focused: *search_active,
                secret: false,
            },
            AuthState::Running(flow) => flow.prompt.as_ref().map_or_else(
                || alternate_placeholder(&flow.status, false),
                |prompt| {
                    if prompt.kind == AuthPromptKind::Select {
                        alternate_placeholder("Select an authentication option", false)
                    } else {
                        AlternateInputModel {
                            text: prompt.editor.text().to_owned(),
                            cursor: prompt.editor.cursor(),
                            placeholder: prompt
                                .placeholder
                                .clone()
                                .unwrap_or_else(|| prompt.message.clone()),
                            focused: true,
                            secret: prompt.kind == AuthPromptKind::Secret,
                        }
                    }
                },
            ),
            AuthState::LoadingProviders => alternate_placeholder("Loading providers…", false),
            AuthState::Inactive => alternate_placeholder("Authentication", false),
        },
        _ => alternate_placeholder("Search", false),
    }
}

fn alternate_placeholder(placeholder: &str, focused: bool) -> AlternateInputModel {
    AlternateInputModel {
        text: String::new(),
        cursor: 0,
        placeholder: placeholder.to_owned(),
        focused,
        secret: false,
    }
}

pub(crate) fn alternate_status(view: &SceneViewModel, input_focused: bool) -> &'static str {
    if let Some(browser) = view.tree_browser.as_ref() {
        return match &browser.phase {
            TreePhase::EditLabel { .. } => "Esc cancel · Enter save label · ←→ edit",
            TreePhase::CustomSummary { .. } => "Esc back · Enter navigate · ←→ edit",
            TreePhase::Navigating {
                summarizing: true,
                aborting: false,
                ..
            } => "Esc cancel navigation",
            TreePhase::Navigating { .. } => "Navigation in progress",
            _ if input_focused => "Esc clear · Enter return to tree · ←→ edit",
            _ => "/ search · Esc close · Tab/⇧Tab/Ctrl+N/P select · Enter",
        };
    }
    if let AuthState::Running(flow) = &view.auth_state
        && flow
            .prompt
            .as_ref()
            .is_some_and(|prompt| prompt.kind != AuthPromptKind::Select)
    {
        return "Esc cancel login · Enter submit · ←→ edit";
    }
    if input_focused {
        return "Esc clear · Enter return to list · ←→ edit";
    }
    if view.transcript_viewer.is_some() {
        "/ search · Esc close · Tab/⇧Tab tools · Enter expand · ↑↓/PgUp/PgDn scroll"
    } else {
        "/ search · Esc close · Tab/⇧Tab/Ctrl+N/P select · Enter"
    }
}

pub(crate) fn alternate_composer_rows(
    input: &AlternateInputModel,
    width: u16,
    height: u16,
    cursor_row: usize,
) -> ComposerRender {
    let display_text = input.display_text();
    let border = palette::input_border();
    if height < 3 || width < 4 {
        let visible = if display_text.is_empty() {
            input.placeholder.as_str()
        } else {
            display_text.as_str()
        };
        let mut row = text_row(
            "alternate-input",
            visible,
            if display_text.is_empty() {
                CellStyle::foreground(palette::GRAY_MUTED)
            } else {
                CellStyle::foreground(palette::TEXT)
            },
            width.saturating_sub(2).max(1),
        );
        let mut cells = vec![
            StyledCell::new("›", 1, border),
            StyledCell::new(" ", 1, border),
        ];
        cells.extend(row.cells);
        row.cells = cells;
        return ComposerRender {
            rows: vec![row],
            first_content_row: 0,
            content_column: 2,
            content_row: 0,
        };
    }

    let content_width = composer_content_width(width);
    let text_style = CellStyle::foreground(palette::TEXT);
    let mut content = wrap_text("alternate-input", &display_text, content_width, text_style);
    while content.len() <= cursor_row {
        content.push(VisualRow::blank("alternate-input"));
    }
    let capacity = usize::from(height.saturating_sub(COMPOSER_CHROME_HEIGHT).max(1));
    let maximum_start = content.len().saturating_sub(capacity);
    let first_content_row = cursor_row
        .saturating_sub(capacity.saturating_sub(1))
        .min(maximum_start);
    let visible = &content[first_content_row..content.len().min(first_content_row + capacity)];
    let mut rows = vec![input_border_row("alternate-input", "╭", "╮", width, border)];
    for (visible_index, row) in visible.iter().enumerate() {
        let mut cells = vec![
            StyledCell::new("│", 1, border),
            StyledCell::new(
                if first_content_row + visible_index == 0 {
                    "›"
                } else {
                    " "
                },
                1,
                border,
            ),
            StyledCell::new(" ", 1, border),
        ];
        if display_text.is_empty() && first_content_row + visible_index == 0 {
            append_text_cells(
                &mut cells,
                &truncate(&input.placeholder, usize::from(content_width)),
                CellStyle::foreground(palette::GRAY_MUTED),
            );
        } else {
            cells.extend(row.cells.clone());
        }
        let used = cells_width(&cells);
        let padding = width.saturating_sub(used).saturating_sub(1);
        if padding > 0 {
            cells.push(StyledCell::new(
                " ".repeat(usize::from(padding)),
                padding,
                text_style,
            ));
        }
        cells.push(StyledCell::new("│", 1, border));
        rows.push(VisualRow {
            component_id: "alternate-input".to_owned(),
            logical_line: first_content_row + visible_index,
            wrap_index: row.wrap_index,
            cells,
        });
    }
    rows.push(input_border_row("alternate-input", "╰", "╯", width, border));
    ComposerRender {
        rows,
        first_content_row,
        content_column: 3,
        content_row: 1,
    }
}
