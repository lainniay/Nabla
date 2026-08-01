use std::io::{self, Write};

use ratatui::{
    Frame, Terminal,
    backend::Backend,
    layout::{Alignment, Constraint, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Widget, Wrap},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{
    browser::is_safe_web_url,
    selection::centered_visible_start,
    state::{
        AppState, AssistantMessage, AuthPromptKind, AuthState, ConnectionState, ContextCategory,
        ContextSnapshot, ContextUsageState, EditorState, PlanReviewState, PruneReason, RunState,
        SessionScope, ToolStatus, TranscriptItem, TranscriptViewMode, TranscriptViewerState,
        TreeItem, TreePhase, UiModalKind, UserMessageStatus, matching_auth_choice_indices,
    },
    theme::THEME,
    ui_types::{MouseCaptureMode, RenderOutcome, UiHitMap, UiHitTarget, UiLayoutMetrics},
};

pub const MIN_INLINE_VIEWPORT_HEIGHT: u16 = 16;
pub const MAX_INLINE_VIEWPORT_HEIGHT: u16 = 32;
pub const SELECTOR_CHROME_HEIGHT: u16 = 6;
const RECENT_HISTORY_LINE_LIMIT: usize = 64;
pub const LIVE_TRANSCRIPT_TAIL_HEIGHT: u16 = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MainViewLayout {
    output_height: u16,
    auxiliary_height: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LayoutSurface {
    Main,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiLayoutRequest {
    terminal_columns: u16,
    terminal_rows: u16,
    desired_height: u16,
    active_output_height: u16,
    requested_auxiliary_height: u16,
    composer_height: u16,
    footer_height: u16,
    surface: LayoutSurface,
}

impl UiLayoutRequest {
    pub fn desired_height(self) -> u16 {
        self.desired_height
    }

    pub fn terminal_rows(self) -> u16 {
        self.terminal_rows
    }

    pub fn resolve_layout(self, applied_viewport_height: u16) -> UiLayoutMetrics {
        let applied_viewport_height = applied_viewport_height.clamp(1, self.terminal_rows.max(1));
        if self.surface == LayoutSurface::Full {
            return UiLayoutMetrics {
                terminal_columns: self.terminal_columns,
                terminal_rows: self.terminal_rows,
                desired_height: applied_viewport_height,
                body_height: applied_viewport_height.saturating_sub(self.footer_height),
                footer_height: self.footer_height,
                ..UiLayoutMetrics::default()
            };
        }

        let layout = main_view_layout(
            applied_viewport_height,
            self.active_output_height,
            self.requested_auxiliary_height,
            self.composer_height,
            self.footer_height,
        );
        UiLayoutMetrics {
            terminal_columns: self.terminal_columns,
            terminal_rows: self.terminal_rows,
            desired_height: applied_viewport_height,
            output_height: layout.output_height,
            auxiliary_height: layout.auxiliary_height,
            composer_height: self.composer_height,
            footer_height: self.footer_height,
            body_height: layout.output_height.saturating_add(layout.auxiliary_height),
        }
    }
}

fn main_view_layout(
    viewport_height: u16,
    active_output_height: u16,
    requested_auxiliary_height: u16,
    composer_height: u16,
    footer_height: u16,
) -> MainViewLayout {
    let available = viewport_height.saturating_sub(composer_height + footer_height);
    let minimum_active_output = u16::from(active_output_height > 0);
    let auxiliary_height =
        requested_auxiliary_height.min(available.saturating_sub(minimum_active_output));
    let output_height = available.saturating_sub(auxiliary_height);
    MainViewLayout {
        output_height,
        auxiliary_height,
    }
}

pub fn inline_viewport_height(terminal_rows: u16) -> u16 {
    if terminal_rows == 0 {
        return 0;
    }
    let preferred = ((u32::from(terminal_rows) * 2) / 3) as u16;
    preferred
        .clamp(MIN_INLINE_VIEWPORT_HEIGHT, MAX_INLINE_VIEWPORT_HEIGHT)
        .min(terminal_rows)
}

const TEXT: Color = THEME.text;
const MUTED: Color = THEME.muted;
const BORDER: Color = THEME.border;
const CYAN: Color = THEME.user;
const VIOLET: Color = THEME.primary;
const ORANGE: Color = THEME.warning;
const GREEN: Color = THEME.success;
const TEAL: Color = THEME.goal;
const RED: Color = THEME.error;
const MENU_SELECTED: Color = THEME.surface0;

#[derive(Debug, Clone)]
struct ChoiceItemView {
    label: String,
    description: String,
    enabled: bool,
}

#[derive(Debug, Clone)]
struct ChoicePanelView {
    title: String,
    prompt: String,
    items: Vec<ChoiceItemView>,
    selected: usize,
    submitting: bool,
    status: String,
    cancel_label: &'static str,
}

fn choice_item(label: impl Into<String>, description: impl Into<String>) -> ChoiceItemView {
    ChoiceItemView {
        label: label.into(),
        description: description.into(),
        enabled: true,
    }
}

fn short_choice_panel(state: &AppState) -> Option<ChoicePanelView> {
    match state.active_modal_kind()? {
        UiModalKind::Question => {
            let flow = state.question.as_ref()?;
            let question = flow.current_question()?;
            let mut items = question
                .options
                .iter()
                .map(|option| {
                    choice_item(
                        option.label.clone(),
                        option.description.clone().unwrap_or_default(),
                    )
                })
                .collect::<Vec<_>>();
            items.push(choice_item("Other…", "enter a custom answer"));
            Some(ChoicePanelView {
                title: format!("Question {}/{}", flow.current + 1, flow.questions.len()),
                prompt: question.prompt.clone(),
                items,
                selected: flow.selected,
                submitting: flow.replying,
                status: if flow.replying {
                    "Submitting answers…".to_owned()
                } else {
                    "Choose an option".to_owned()
                },
                cancel_label: "interrupt",
            })
        }
        UiModalKind::Approval => {
            let approval = state.approval.as_ref()?;
            let mut items = vec![choice_item(
                "Allow once",
                "run this command for the current request",
            )];
            if approval.goal_id.is_some() {
                items.push(choice_item(
                    "Allow for this Goal",
                    "grant the capability for the current Goal",
                ));
            }
            items.push(choice_item("Deny", "do not run this command"));
            let context = [
                approval
                    .agent_profile
                    .as_deref()
                    .map(|profile| format!("agent {profile}")),
                approval.risk.as_deref().map(|risk| format!("risk {risk}")),
                approval.reason.clone(),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
            Some(ChoicePanelView {
                title: "Command approval".to_owned(),
                prompt: format!(
                    "Allow {}?{}",
                    approval.tool_name,
                    if context.is_empty() {
                        String::new()
                    } else {
                        format!("  {context}")
                    }
                ),
                items,
                selected: approval.selected,
                submitting: approval.replying,
                status: if approval.replying {
                    "Sending decision…".to_owned()
                } else {
                    "Choose an approval decision".to_owned()
                },
                cancel_label: "deny + interrupt",
            })
        }
        UiModalKind::GoalApproval => {
            let approval = state.goal_approval.as_ref()?;
            let summary = state
                .goal
                .as_ref()
                .and_then(|snapshot| snapshot.goal.as_ref())
                .and_then(|goal| goal.spec.as_ref())
                .map_or("Goal specification is ready", |spec| spec.summary.as_str());
            Some(ChoicePanelView {
                title: "Goal approval".to_owned(),
                prompt: summary.to_owned(),
                items: vec![
                    choice_item(
                        "Approve Goal specification",
                        "grant the Goal capability lease",
                    ),
                    choice_item("Keep awaiting approval", "close without approving"),
                ],
                selected: approval.selected,
                submitting: approval.submitting,
                status: if approval.submitting {
                    "Granting the Goal capability lease…".to_owned()
                } else {
                    "Choose how to continue".to_owned()
                },
                cancel_label: "keep awaiting",
            })
        }
        UiModalKind::PlanReview => {
            let review = state.plan_review.as_ref()?;
            match review {
                PlanReviewState::Menu { selected } => Some(ChoicePanelView {
                    title: "Plan review".to_owned(),
                    prompt: "Choose how to continue with the submitted Plan".to_owned(),
                    items: vec![
                        choice_item(
                            "Execute · current context",
                            "leave Plan mode and implement in this conversation",
                        ),
                        choice_item(
                            "Execute · fresh context",
                            "execute only the submitted plan artifact",
                        ),
                        choice_item("Keep discussing", "stay in Plan mode"),
                    ],
                    selected: *selected,
                    submitting: false,
                    status: "Choose a Plan action".to_owned(),
                    cancel_label: "stay",
                }),
                PlanReviewState::Confirm {
                    target,
                    selected,
                    submitting,
                } => Some(ChoicePanelView {
                    title: "Confirm Plan execution".to_owned(),
                    prompt: match target {
                        crate::state::PlanExecutionTarget::Current => {
                            "Leave Plan mode and execute in the current context?".to_owned()
                        }
                        crate::state::PlanExecutionTarget::Fresh => {
                            "Execute in a fresh session without previous messages?".to_owned()
                        }
                    },
                    items: vec![
                        choice_item("Execute Plan", target.label()),
                        choice_item("Back", "return to Plan actions"),
                    ],
                    selected: *selected,
                    submitting: *submitting,
                    status: if *submitting {
                        "Preparing Plan execution…".to_owned()
                    } else {
                        "Confirmation required".to_owned()
                    },
                    cancel_label: "back",
                }),
            }
        }
        UiModalKind::Integration => {
            let prompt = state.integration_prompt.as_ref()?;
            let mut resolve = choice_item(
                "Resolve",
                if prompt.integration.resolver_available {
                    "run one isolated conflict resolver"
                } else {
                    "resolver attempt already used"
                },
            );
            resolve.enabled = prompt.integration.resolver_available;
            Some(ChoicePanelView {
                title: "Integrate subagent changes".to_owned(),
                prompt: format!(
                    "{} [{}] · {} changed files · {} bytes",
                    prompt.agent.id,
                    prompt.agent.profile,
                    prompt.integration.changed_paths.len(),
                    prompt.integration.patch_bytes
                ),
                items: vec![
                    choice_item("Apply", "apply the patch if it is clean"),
                    resolve,
                    choice_item("Keep", "retain the worktree and patch"),
                    choice_item("Discard", "remove the managed worktree and patch"),
                ],
                selected: prompt.selected,
                submitting: prompt.submitting,
                status: if prompt.submitting {
                    "Submitting integration action…".to_owned()
                } else {
                    "Choose an integration action".to_owned()
                },
                cancel_label: "close",
            })
        }
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct HistoryLine {
    pub line: Line<'static>,
    /// Plain prose can use terminal soft wrapping and naturally reflow.
    /// Alignment-sensitive Markdown is projected with this set to false.
    pub soft_wrap: bool,
}

#[derive(Debug, Clone, Default)]
pub struct HistoryProjection {
    pub lines: Vec<HistoryLine>,
}

/// Source-driven projection of append-only transcript state into native
/// scrollback. A bounded live tail keeps stable and mutable streaming output
/// contiguous above the composer while older overflow enters native history.
#[derive(Debug, Clone, Default)]
pub struct TranscriptProjector {
    next_item: usize,
    assistant: Option<AssistantProjection>,
    live_tail_lines: Vec<Line<'static>>,
    recent_history_lines: Vec<Line<'static>>,
    projection_width: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct PreparedTranscriptProjection {
    projected: TranscriptProjector,
    history_lines: Vec<Line<'static>>,
    width: usize,
}

impl PreparedTranscriptProjection {
    pub fn projected(&self) -> &TranscriptProjector {
        &self.projected
    }

    pub fn retain_live_tail(&mut self, state: &AppState, maximum_visual_height: u16) {
        let mutable_lines = self.projected.mutable_lines_with_width(state, self.width);
        let has_mutable_output = self.projected.has_mutable_table() || !mutable_lines.is_empty();
        let mutable_height = if self.projected.has_mutable_table() {
            0
        } else {
            visual_height(&mutable_lines, self.width).min(maximum_visual_height)
        };
        let maximum_visual_height = maximum_visual_height.saturating_sub(mutable_height);
        if maximum_visual_height == 0 {
            self.release_live_tail();
            return;
        }
        let mut retained_height: u16 = 0;
        let mut retained_start = self.projected.live_tail_lines.len();
        let mut trailing_blank = !has_mutable_output;
        for (index, line) in self.projected.live_tail_lines.iter().enumerate().rev() {
            let line_height = if trailing_blank && line_is_blank(line) {
                0
            } else {
                trailing_blank = false;
                visual_height(std::slice::from_ref(line), self.width).max(1)
            };
            if retained_height > 0
                && retained_height.saturating_add(line_height) > maximum_visual_height
            {
                break;
            }
            retained_height = retained_height.saturating_add(line_height);
            retained_start = index;
            if retained_height >= maximum_visual_height {
                break;
            }
        }
        if retained_start > 0 {
            self.history_lines
                .extend(self.projected.live_tail_lines.drain(..retained_start));
        }
    }

    pub fn release_live_tail(&mut self) {
        self.history_lines
            .append(&mut self.projected.live_tail_lines);
    }
}

#[derive(Debug, Clone)]
struct AssistantProjection {
    item_index: usize,
    thinking_offset: usize,
    text_offset: usize,
    thinking_started: bool,
    text_started: bool,
    markdown: MarkdownState,
}

impl AssistantProjection {
    fn new(item_index: usize) -> Self {
        Self {
            item_index,
            thinking_offset: 0,
            text_offset: 0,
            thinking_started: false,
            text_started: false,
            markdown: MarkdownState::default(),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct MarkdownState {
    fence: Option<char>,
    pending_row: Option<PendingMarkdownRow>,
    table: Option<MarkdownTable>,
}

#[derive(Debug, Clone)]
struct PendingMarkdownRow {
    prefix: String,
    prefix_style: Style,
    row: String,
}

#[derive(Debug, Clone)]
struct MarkdownTable {
    prefix: String,
    prefix_style: Style,
    headers: Vec<String>,
    alignments: Vec<TableAlignment>,
    rows: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TableAlignment {
    Left,
    Center,
    Right,
}

impl TranscriptProjector {
    pub fn next_item(&self) -> usize {
        self.next_item
    }

    pub fn has_mutable_table(&self) -> bool {
        self.assistant
            .as_ref()
            .is_some_and(|assistant| assistant.markdown.table.is_some())
    }

    pub fn has_live_tail(&self) -> bool {
        !self.live_tail_lines.is_empty()
    }

    pub fn prepare(&self, state: &AppState, width: usize) -> PreparedTranscriptProjection {
        let width = width.max(1);
        let mut projected = self.clone();
        let mut history_lines = Vec::new();
        if projected
            .projection_width
            .is_some_and(|projection_width| projection_width != width)
        {
            history_lines.append(&mut projected.live_tail_lines);
        }
        projected.projection_width = Some(width);
        let projected_lines = projected.project_ready(state, width);
        projected.live_tail_lines.extend(projected_lines);
        PreparedTranscriptProjection {
            projected,
            history_lines,
            width,
        }
    }

    pub fn commit(
        &mut self,
        terminal: &mut Terminal<impl Backend<Error = std::io::Error>>,
        prepared: PreparedTranscriptProjection,
    ) -> std::io::Result<()> {
        self.commit_with_backend(terminal, prepared)
    }

    pub fn flush(
        &mut self,
        terminal: &mut Terminal<impl Backend<Error = std::io::Error>>,
    ) -> std::io::Result<()> {
        self.flush_with_backend(terminal)
    }

    fn commit_with_backend<B: Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
        prepared: PreparedTranscriptProjection,
    ) -> Result<(), B::Error> {
        let PreparedTranscriptProjection {
            mut projected,
            history_lines,
            width,
        } = prepared;
        projected.remember_recent_history(&history_lines);
        if !history_lines.is_empty() {
            let height = visual_height(&history_lines, width).max(1);
            terminal.insert_before(height, move |buffer| {
                Paragraph::new(history_lines)
                    .style(Style::default().fg(TEXT))
                    .wrap(Wrap { trim: false })
                    .render(buffer.area, buffer);
            })?;
        }
        *self = projected;
        Ok(())
    }

    fn flush_with_backend<B: Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) -> Result<(), B::Error> {
        if self.live_tail_lines.is_empty() {
            return Ok(());
        }
        let width = terminal.size()?.width.max(1) as usize;
        let lines = std::mem::take(&mut self.live_tail_lines);
        let height = visual_height(&lines, width).max(1);
        terminal.insert_before(height, move |buffer| {
            Paragraph::new(lines)
                .style(Style::default().fg(TEXT))
                .wrap(Wrap { trim: false })
                .render(buffer.area, buffer);
        })
    }

    fn project_ready(&mut self, state: &AppState, width: usize) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        while let Some(item) = state.transcript.get(self.next_item) {
            let active_assistant =
                matches!(&self.assistant, Some(active) if active.item_index == self.next_item);

            if let TranscriptItem::Assistant(message) = item
                && (!message.complete || active_assistant)
            {
                if !active_assistant {
                    self.assistant = Some(AssistantProjection::new(self.next_item));
                }
                let projection = self
                    .assistant
                    .as_mut()
                    .expect("assistant projection was initialized");
                project_assistant(message, projection, width, &mut lines);
                if message.complete {
                    lines.push(Line::default());
                    self.assistant = None;
                    self.next_item += 1;
                    continue;
                }
                break;
            }

            if !is_complete(item) {
                break;
            }
            lines.extend(item_lines_with_width(item, width));
            self.next_item += 1;
        }
        lines
    }

    #[cfg(test)]
    fn active_lines(&self, state: &AppState) -> Vec<Line<'static>> {
        self.active_lines_with_width(state, 80)
    }

    fn active_lines_with_width(&self, state: &AppState, width: usize) -> Vec<Line<'static>> {
        let mut lines = self
            .live_tail_lines
            .iter()
            .cloned()
            .chain(self.mutable_lines_with_width(state, width))
            .collect::<Vec<_>>();
        while lines.last().is_some_and(line_is_blank) {
            lines.pop();
        }
        lines
    }

    fn mutable_lines_with_width(&self, state: &AppState, width: usize) -> Vec<Line<'static>> {
        let mut lines = state
            .transcript
            .iter()
            .enumerate()
            .skip(self.next_item)
            .flat_map(|(index, item)| {
                if let (TranscriptItem::Assistant(message), Some(projection)) =
                    (item, self.assistant.as_ref())
                    && projection.item_index == index
                {
                    return assistant_remainder_lines_with_width(message, projection, width);
                }
                if let TranscriptItem::Assistant(message) = item
                    && !message.complete
                {
                    return assistant_remainder_lines_with_width(
                        message,
                        &AssistantProjection::new(index),
                        width,
                    );
                }
                item_lines_with_width(item, width)
            })
            .collect::<Vec<_>>();
        while lines.last().is_some_and(line_is_blank) {
            lines.pop();
        }
        lines
    }

    fn remember_recent_history(&mut self, lines: &[Line<'static>]) {
        self.recent_history_lines.extend(lines.iter().cloned());
        let excess = self
            .recent_history_lines
            .len()
            .saturating_sub(RECENT_HISTORY_LINE_LIMIT);
        if excess > 0 {
            self.recent_history_lines.drain(..excess);
        }
    }
}

fn line_is_blank(line: &Line<'_>) -> bool {
    line.spans
        .iter()
        .all(|span| span.content.chars().all(char::is_whitespace))
}

/// Kept as a source-compatible name for callers built against the first inline
/// renderer. New code should use `TranscriptProjector`.
pub type TranscriptPresenter = TranscriptProjector;

pub fn uses_fullscreen_surface(state: &AppState) -> bool {
    matches!(
        state.active_modal_kind(),
        Some(
            UiModalKind::SessionBrowser
                | UiModalKind::TreeBrowser
                | UiModalKind::AgentPicker
                | UiModalKind::Transcript
                | UiModalKind::Auth
        )
    )
}

pub fn measure_layout_request(
    state: &AppState,
    presenter: &TranscriptPresenter,
    terminal_columns: u16,
    terminal_rows: u16,
) -> UiLayoutRequest {
    if uses_fullscreen_surface(state) {
        let footer_height = u16::from(terminal_rows > 1);
        return UiLayoutRequest {
            terminal_columns,
            terminal_rows,
            desired_height: terminal_rows.max(1),
            active_output_height: 0,
            requested_auxiliary_height: 0,
            composer_height: 0,
            footer_height,
            surface: LayoutSurface::Full,
        };
    }
    let inner_width = terminal_columns.saturating_sub(4).max(1) as usize;
    let maximum_composer_rows = if terminal_rows < 12 { 3 } else { 8 };
    let composer_rows = state
        .editor
        .composer_viewport(inner_width, maximum_composer_rows)
        .visible_rows;
    let composer_height = composer_rows.saturating_add(2);
    let footer_height = u16::from(terminal_rows >= 10);
    let requested_auxiliary_height = if let Some(panel) = short_choice_panel(state) {
        (panel.items.len() as u16).saturating_add(1).clamp(2, 12)
    } else {
        let candidates = state.command_candidates().len() as u16;
        if candidates == 0 {
            0
        } else {
            candidates.clamp(3, 12)
        }
    };
    let active_lines = presenter.active_lines_with_width(state, terminal_columns.max(1) as usize);
    let active_output_height = visual_height(&active_lines, terminal_columns.max(1) as usize);
    let essential = composer_height
        .saturating_add(footer_height)
        .saturating_add(requested_auxiliary_height);
    let preferred_output = active_output_height
        .min(12)
        .max(u16::from(active_output_height > 0));
    let desired_height = essential
        .saturating_add(preferred_output)
        .clamp(composer_height.min(terminal_rows), terminal_rows.max(1));
    UiLayoutRequest {
        terminal_columns,
        terminal_rows,
        desired_height,
        active_output_height,
        requested_auxiliary_height,
        composer_height,
        footer_height,
        surface: LayoutSurface::Main,
    }
}

pub fn render(
    frame: &mut Frame<'_>,
    state: &AppState,
    presenter: &TranscriptPresenter,
    metrics: UiLayoutMetrics,
) -> RenderOutcome {
    let mouse_capture =
        if state.active_modal_kind().is_some() || !state.command_candidates().is_empty() {
            MouseCaptureMode::Surface
        } else {
            MouseCaptureMode::Off
        };
    let mut outcome = RenderOutcome {
        desired_height: metrics.desired_height,
        mouse_capture,
        metrics,
        ..RenderOutcome::default()
    };
    match state.active_modal_kind() {
        Some(UiModalKind::SessionBrowser) => {
            render_session_browser(frame, state);
            outcome.hit_map = modal_list_hit_map(frame.area(), state);
            return outcome;
        }
        Some(UiModalKind::TreeBrowser) => {
            render_tree_browser(frame, state);
            outcome.hit_map = modal_list_hit_map(frame.area(), state);
            return outcome;
        }
        Some(UiModalKind::AgentPicker) => {
            render_agent_picker(frame, state);
            outcome.hit_map = modal_list_hit_map(frame.area(), state);
            return outcome;
        }
        Some(UiModalKind::Transcript) => {
            render_transcript_viewer(frame, state);
            outcome.hit_map = transcript_hit_map(frame.area(), state);
            return outcome;
        }
        Some(UiModalKind::Auth) => {
            render_auth(frame, state);
            outcome.hit_map = auth_choice_hit_map(frame.area(), state);
            return outcome;
        }
        _ => {}
    }

    let active_lines = presenter.active_lines_with_width(state, frame.area().width.max(1) as usize);
    let [output_area, auxiliary_area, input_area, footer_area] = Layout::vertical([
        Constraint::Length(metrics.output_height),
        Constraint::Length(metrics.auxiliary_height),
        Constraint::Length(metrics.composer_height),
        Constraint::Length(metrics.footer_height),
    ])
    .areas(frame.area());

    render_active_output(frame, active_lines, output_area);
    if let Some(panel) = short_choice_panel(state) {
        render_choice_panel(frame, auxiliary_area, &panel);
    } else {
        render_command_menu(frame, state, auxiliary_area);
    }
    render_input(frame, state, input_area);
    render_footer(frame, state, footer_area);
    outcome.hit_map = if let Some(panel) = short_choice_panel(state) {
        choice_hit_map(auxiliary_area, &panel)
    } else {
        command_hit_map(auxiliary_area, state)
    };
    outcome
}

fn command_hit_map(area: Rect, state: &AppState) -> UiHitMap {
    let candidates = state.command_candidates();
    let visible = area.height as usize;
    let selected = state
        .command_menu_selected()
        .min(candidates.len().saturating_sub(1));
    let start = centered_visible_start(candidates.len(), selected, visible);
    let mut hit_map = UiHitMap::default();
    for (row, index) in (start..candidates.len()).take(visible).enumerate() {
        hit_map.push(
            Rect::new(area.x, area.y.saturating_add(row as u16), area.width, 1),
            UiHitTarget::CommandCandidate(index),
        );
    }
    hit_map
}

fn choice_hit_map(area: Rect, panel: &ChoicePanelView) -> UiHitMap {
    let rows = area.height.saturating_sub(1) as usize;
    let selected = panel.selected.min(panel.items.len().saturating_sub(1));
    let start = centered_visible_start(panel.items.len(), selected, rows);
    let mut hit_map = UiHitMap::default();
    for (row, index) in (start..panel.items.len()).take(rows).enumerate() {
        hit_map.push(
            Rect::new(area.x, area.y.saturating_add(1 + row as u16), area.width, 1),
            UiHitTarget::ChoiceOption(index),
        );
    }
    hit_map
}

fn render_choice_panel(frame: &mut Frame<'_>, area: Rect, panel: &ChoicePanelView) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let [header_area, choices_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
    let header = Line::from(vec![
        Span::styled(
            format!("{}  ", panel.title),
            Style::default().fg(VIOLET).add_modifier(Modifier::BOLD),
        ),
        Span::styled(panel.prompt.clone(), Style::default().fg(TEXT)),
    ]);
    frame.render_widget(Paragraph::new(header), header_area);

    let visible = choices_area.height as usize;
    let selected = panel.selected.min(panel.items.len().saturating_sub(1));
    let start = centered_visible_start(panel.items.len(), selected, visible);
    let lines = panel
        .items
        .iter()
        .enumerate()
        .skip(start)
        .take(visible)
        .map(|(index, item)| {
            choice_line(
                choices_area.width as usize,
                index == selected,
                item.enabled,
                &format!("{}. {}", index + 1, item.label),
                &item.description,
            )
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), choices_area);
}

fn modal_list_hit_map(area: Rect, state: &AppState) -> UiHitMap {
    let [content_area, _, _] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .areas(area);
    let inner = Rect::new(
        content_area.x.saturating_add(1),
        content_area.y.saturating_add(1),
        content_area.width.saturating_sub(2),
        content_area.height.saturating_sub(2),
    );
    let (count, selected) = match state.active_modal_kind() {
        Some(UiModalKind::SessionBrowser) => state
            .session_browser
            .as_ref()
            .map(|browser| (browser.sessions.len(), browser.selected)),
        Some(UiModalKind::TreeBrowser) => state
            .tree_browser
            .as_ref()
            .filter(|browser| matches!(browser.phase, TreePhase::Browse))
            .map(|browser| (browser.items.len(), browser.selected)),
        Some(UiModalKind::AgentPicker) => state
            .agent_picker
            .as_ref()
            .map(|picker| (picker.profiles.len(), picker.selected)),
        _ => None,
    }
    .unwrap_or_default();
    let visible = inner.height as usize;
    let start = centered_visible_start(count, selected, visible);
    let mut hit_map = UiHitMap::default();
    for (row, index) in (start..count).take(visible).enumerate() {
        hit_map.push(
            Rect::new(inner.x, inner.y.saturating_add(row as u16), inner.width, 1),
            UiHitTarget::ListRow(index),
        );
    }
    hit_map
}

fn auth_choice_hit_map(area: Rect, state: &AppState) -> UiHitMap {
    let [content_area, _, _] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .areas(area);
    let mut hit_map = UiHitMap::default();
    match &state.auth_state {
        AuthState::Selecting {
            choices,
            selected,
            filter,
        } => {
            let matching = matching_auth_choice_indices(choices, filter.text());
            let start =
                centered_visible_start(matching.len(), *selected, content_area.height as usize);
            for (row, index) in (start..matching.len())
                .take(content_area.height as usize)
                .enumerate()
            {
                hit_map.push(
                    Rect::new(
                        content_area.x,
                        content_area.y.saturating_add(row as u16),
                        content_area.width,
                        1,
                    ),
                    UiHitTarget::ChoiceOption(index),
                );
            }
        }
        AuthState::Running(flow)
            if flow
                .prompt
                .as_ref()
                .is_some_and(|prompt| prompt.kind == AuthPromptKind::Select) =>
        {
            let prompt = flow.prompt.as_ref().expect("select prompt was checked");
            let options_y = content_area
                .y
                .saturating_add(5)
                .saturating_add(u16::from(flow.url.is_some()))
                .saturating_add(u16::from(flow.device_code.is_some()));
            let visible = content_area.bottom().saturating_sub(options_y) as usize;
            for index in 0..prompt.options.len().min(visible) {
                hit_map.push(
                    Rect::new(
                        content_area.x,
                        options_y.saturating_add(index as u16),
                        content_area.width,
                        1,
                    ),
                    UiHitTarget::ChoiceOption(index),
                );
            }
        }
        _ => {}
    }
    hit_map
}

fn render_agent_picker(frame: &mut Frame<'_>, state: &AppState) {
    let [content_area, input_area, footer_area] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let Some(picker) = state.agent_picker.as_ref() else {
        return;
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(CYAN))
        .title(Span::styled(" Select Subagent ", Style::default().fg(CYAN)));
    let inner = block.inner(content_area);
    frame.render_widget(block, content_area);
    let visible = inner.height as usize;
    let start = centered_visible_start(picker.profiles.len(), picker.selected, visible);
    let lines = if picker.profiles.is_empty() {
        vec![Line::from(Span::styled(
            "No available subagent profiles",
            Style::default().fg(MUTED),
        ))]
    } else {
        picker
            .profiles
            .iter()
            .enumerate()
            .skip(start)
            .take(visible)
            .map(|(index, profile)| {
                let model = profile.model.as_deref().unwrap_or("main model");
                let row = truncate_to_width(
                    &format!(
                        "{} · {} · {} · {}",
                        profile.name, model, profile.description, profile.source
                    ),
                    inner.width as usize,
                );
                full_width_choice_line(inner.width as usize, index == picker.selected, &row, "")
            })
            .collect()
    };
    frame.render_widget(Paragraph::new(lines), inner);
    render_status_input(
        frame,
        input_area,
        "Choose a profile; the command editor will wait for the task.",
        TEXT,
    );
    frame.render_widget(
        Paragraph::new(" ↑↓ · tab/shift-tab navigate  enter choose  esc close ")
            .style(Style::default().fg(MUTED)),
        footer_area,
    );
}

#[derive(Debug)]
struct TranscriptViewLine {
    item_index: usize,
    line: Line<'static>,
}

fn render_transcript_viewer(frame: &mut Frame<'_>, state: &AppState) {
    let Some(viewer) = state.transcript_viewer.as_ref() else {
        return;
    };
    let [content_area, footer_area] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(frame.area());
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER))
        .title(Span::styled(
            format!(" Transcript · {} ", viewer.mode.label()),
            Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(content_area);
    frame.render_widget(block, content_area);

    let view_lines = transcript_view_lines(state, viewer, inner.width as usize);
    let selected = viewer.selected_item;
    let lines = view_lines
        .iter()
        .map(|view_line| {
            if selected == Some(view_line.item_index) {
                view_line
                    .line
                    .clone()
                    .style(Style::default().bg(MENU_SELECTED))
            } else {
                view_line.line.clone()
            }
        })
        .collect::<Vec<_>>();
    let max_scroll =
        visual_height(&lines, inner.width as usize).saturating_sub(inner.height) as usize;
    let selected_line = selected.and_then(|selected| {
        view_lines
            .iter()
            .position(|line| line.item_index == selected)
    });
    let scroll = if viewer.scroll_to_selected {
        selected_line
            .unwrap_or(max_scroll)
            .saturating_sub(inner.height as usize / 2)
            .min(max_scroll)
    } else if viewer.follow_tail {
        max_scroll
    } else {
        max_scroll.saturating_sub(viewer.scroll_from_bottom.min(max_scroll))
    };
    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::default().fg(TEXT))
            .wrap(Wrap { trim: false })
            .scroll((scroll.min(u16::MAX as usize) as u16, 0)),
        inner,
    );
    let unseen = viewer.unseen_items(state.transcript.len());
    let footer = if viewer.search_active {
        format!(
            " /{}  {} match(es)  enter accept  esc cancel ",
            viewer.search_query,
            viewer.search_matches.len()
        )
    } else {
        format!(
            " ↑↓/pg scroll  g/G ends  / search  n/N match  enter expand  1-3 mode  esc close{} ",
            if unseen > 0 {
                format!("  · {unseen} new")
            } else {
                String::new()
            }
        )
    };
    frame.render_widget(
        Paragraph::new(truncate_to_width(&footer, footer_area.width as usize))
            .style(Style::default().fg(MUTED)),
        footer_area,
    );
}

fn transcript_hit_map(area: Rect, state: &AppState) -> UiHitMap {
    let Some(viewer) = state.transcript_viewer.as_ref() else {
        return UiHitMap::default();
    };
    let [content_area, _] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(area);
    let inner = Rect::new(
        content_area.x.saturating_add(1),
        content_area.y.saturating_add(1),
        content_area.width.saturating_sub(2),
        content_area.height.saturating_sub(2),
    );
    let view_lines = transcript_view_lines(state, viewer, inner.width.max(1) as usize);
    let max_scroll = view_lines.len().saturating_sub(inner.height as usize);
    let scroll = if viewer.follow_tail {
        max_scroll
    } else {
        max_scroll.saturating_sub(viewer.scroll_from_bottom.min(max_scroll))
    };
    let mut hit_map = UiHitMap::default();
    for (visible_row, view_line) in view_lines
        .iter()
        .skip(scroll)
        .take(inner.height as usize)
        .enumerate()
    {
        hit_map.push(
            Rect::new(
                inner.x,
                inner.y.saturating_add(visible_row as u16),
                inner.width,
                1,
            ),
            UiHitTarget::TranscriptItem(view_line.item_index),
        );
    }
    hit_map
}

fn transcript_view_lines(
    state: &AppState,
    viewer: &TranscriptViewerState,
    width: usize,
) -> Vec<TranscriptViewLine> {
    // Keep projection cost bounded for very large sessions. The window is
    // selected from a cheap item index; expensive Markdown/tool expansion is
    // performed only for the active neighborhood.
    const ITEM_WINDOW: usize = 256;
    let visible_indices = state
        .transcript
        .iter()
        .enumerate()
        .filter_map(|(index, item)| transcript_item_visible(item, viewer.mode).then_some(index))
        .collect::<Vec<_>>();
    let (start, end) = if viewer.scroll_to_selected {
        let center = viewer
            .selected_item
            .and_then(|selected| visible_indices.iter().position(|index| *index == selected))
            .unwrap_or(visible_indices.len().saturating_sub(1));
        let start = center.saturating_sub(ITEM_WINDOW / 2);
        (start, (start + ITEM_WINDOW).min(visible_indices.len()))
    } else if viewer.scroll_from_bottom == usize::MAX {
        (0, ITEM_WINDOW.min(visible_indices.len()))
    } else {
        // Scroll offsets are line-based while this first-stage index is
        // item-based. Once the tail window is exhausted, advance it
        // conservatively (roughly two rendered lines per item).
        let overflow = viewer.scroll_from_bottom.saturating_sub(ITEM_WINDOW);
        let end = visible_indices.len().saturating_sub(overflow / 2);
        (end.saturating_sub(ITEM_WINDOW), end)
    };
    visible_indices[start..end]
        .iter()
        .filter_map(|item_index| {
            state
                .transcript
                .get(*item_index)
                .map(|item| (*item_index, item))
        })
        .flat_map(|(item_index, item)| {
            let mut lines = transcript_view_item_lines(item, viewer, width);
            if lines.last().is_some_and(|line| line.spans.is_empty()) {
                lines.pop();
            }
            lines.push(Line::default());
            lines
                .into_iter()
                .map(move |line| TranscriptViewLine { item_index, line })
        })
        .collect()
}

fn transcript_item_visible(item: &TranscriptItem, mode: TranscriptViewMode) -> bool {
    if mode != TranscriptViewMode::Summary {
        return true;
    }
    match item {
        TranscriptItem::Tool(tool) => {
            !matches!(tool.status, ToolStatus::Succeeded)
                || !matches!(tool.name.as_str(), "read" | "grep" | "find" | "ls")
        }
        TranscriptItem::Context(_) | TranscriptItem::Resources(_) | TranscriptItem::Agents(_) => {
            false
        }
        _ => true,
    }
}

fn transcript_view_item_lines(
    item: &TranscriptItem,
    viewer: &TranscriptViewerState,
    width: usize,
) -> Vec<Line<'static>> {
    match item {
        TranscriptItem::Assistant(message) if viewer.mode != TranscriptViewMode::Verbose => {
            item_lines_with_width(
                &TranscriptItem::Assistant(AssistantMessage {
                    text: message.text.clone(),
                    thinking: String::new(),
                    complete: message.complete,
                }),
                width,
            )
        }
        TranscriptItem::Tool(tool) => {
            let default_expanded = viewer.mode == TranscriptViewMode::Verbose;
            let expanded = viewer
                .tool_expansion_overrides
                .get(&tool.id)
                .copied()
                .unwrap_or(default_expanded);
            transcript_tool_lines(tool, expanded)
        }
        _ => item_lines_with_width(item, width),
    }
}

fn transcript_tool_lines(tool: &crate::state::ToolExecution, expanded: bool) -> Vec<Line<'static>> {
    let (symbol, status, style) = tool_status_visual(tool.status);
    let fold = if expanded { "▾" } else { "▸" };
    let summary = tool_request_summary(&tool.name, &tool.args);
    let output_stat = (!tool.output.is_empty()).then(|| {
        format!(
            "{} lines · {}",
            line_count(&tool.output),
            human_bytes(tool.output.len())
        )
    });
    let details = [(!summary.is_empty()).then_some(summary), output_stat]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    let mut lines = vec![Line::from(vec![
        Span::styled(format!("{fold} {symbol} "), style),
        Span::styled(
            tool.name.clone(),
            Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" · {status}"), style),
        Span::styled(
            if details.is_empty() {
                String::new()
            } else {
                format!(" · {details}")
            },
            Style::default().fg(MUTED),
        ),
    ])];
    if expanded && !tool.output.is_empty() {
        push_prefixed(
            &mut lines,
            "  │ ",
            &tool.output,
            Style::default().fg(BORDER),
            if matches!(tool.status, ToolStatus::Failed | ToolStatus::Denied) {
                Style::default().fg(RED)
            } else {
                Style::default().fg(TEXT)
            },
        );
    } else if !expanded
        && matches!(tool.status, ToolStatus::Failed | ToolStatus::Denied)
        && !tool.output.is_empty()
    {
        let (preview, hidden) = bounded_output_preview(&tool.output, 12);
        push_prefixed(
            &mut lines,
            "  │ ",
            &preview,
            Style::default().fg(BORDER),
            Style::default().fg(RED),
        );
        if hidden > 0 {
            lines.push(Line::styled(
                format!("  … {hidden} more lines"),
                Style::default().fg(MUTED),
            ));
        }
    }
    lines
}

fn render_session_browser(frame: &mut Frame<'_>, state: &AppState) {
    let [content_area, input_area, footer_area] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let Some(browser) = state.session_browser.as_ref() else {
        return;
    };
    let scope = match browser.scope {
        SessionScope::Current => "current folder",
        SessionScope::All => "all projects",
    };
    let progress = browser.loaded.map_or_else(String::new, |(loaded, total)| {
        format!(" · loading {loaded}/{total}")
    });
    let bounded = if browser.truncated {
        format!(
            " · showing {}/{} · continue at end",
            browser.sessions.len(),
            browser.total
        )
    } else {
        String::new()
    };
    let title = format!(
        " Resume Session · {scope} · {} · {}{}{} ",
        browser.sort_mode.label(),
        if browser.named_only { "named" } else { "all" },
        progress,
        bounded
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(CYAN))
        .title(Span::styled(title, Style::default().fg(CYAN)));
    let inner = block.inner(content_area);
    frame.render_widget(block, content_area);

    let visible = inner.height as usize;
    let start = centered_visible_start(browser.sessions.len(), browser.selected, visible);
    let lines = if browser.sessions.is_empty() {
        vec![Line::from(Span::styled(
            if browser.loading {
                "Loading sessions…"
            } else {
                "No matching sessions"
            },
            Style::default().fg(MUTED),
        ))]
    } else {
        browser
            .sessions
            .iter()
            .enumerate()
            .skip(start)
            .take(visible)
            .map(|(index, session)| {
                let selected = index == browser.selected;
                let branch = if session.depth == 0 {
                    String::new()
                } else {
                    format!(
                        "{}{} ",
                        "   ".repeat(session.depth.saturating_sub(1)),
                        if session.is_last { "└─" } else { "├─" }
                    )
                };
                let current = if session.current { " [current]" } else { "" };
                let cwd = if browser.scope == SessionScope::All && !session.cwd.is_empty() {
                    format!(" · {}", session.cwd)
                } else {
                    String::new()
                };
                let path = if browser.show_path {
                    format!(" · {}", session.path)
                } else {
                    String::new()
                };
                let row = truncate_to_width(
                    &format!(
                        "{}{}{} · {} msgs{}{}",
                        branch,
                        session.label(),
                        current,
                        session.message_count,
                        cwd,
                        path
                    ),
                    inner.width as usize,
                );
                full_width_choice_line(inner.width as usize, selected, &row, "")
            })
            .collect()
    };
    frame.render_widget(Paragraph::new(lines), inner);

    if let Some(session) = browser.confirm_missing_cwd.as_ref() {
        render_status_input(
            frame,
            input_area,
            &format!(
                "Original cwd is unavailable. Resume {} in {}?  y/enter confirm · n/esc cancel",
                session.label(),
                browser.current_cwd
            ),
            ORANGE,
        );
    } else if browser.switching {
        render_status_input(frame, input_area, "Switching session…", ORANGE);
    } else {
        render_auth_editor(frame, input_area, " search ", &browser.query, false);
    }
    frame.render_widget(
        Paragraph::new(
            " ↑↓ move · ←→ page · tab scope · ctrl+s sort · ctrl+n named · ctrl+p path · enter resume · esc close ",
        )
        .style(Style::default().fg(MUTED)),
        footer_area,
    );
}

fn render_tree_browser(frame: &mut Frame<'_>, state: &AppState) {
    let [content_area, input_area, footer_area] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let Some(browser) = state.tree_browser.as_ref() else {
        return;
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(VIOLET))
        .title(Span::styled(
            format!(
                " Session Tree · {}{} ",
                browser.filter_mode.label(),
                if browser.loading { " · loading" } else { "" }
            ),
            Style::default().fg(VIOLET),
        ));
    let inner = block.inner(content_area);
    frame.render_widget(block, content_area);
    let visible = inner.height as usize;
    let start = centered_visible_start(browser.items.len(), browser.selected, visible);
    let lines = if browser.items.is_empty() {
        vec![Line::from(Span::styled(
            if browser.loading {
                "Loading tree…"
            } else {
                "No matching entries"
            },
            Style::default().fg(MUTED),
        ))]
    } else {
        let visible_items = browser
            .items
            .iter()
            .enumerate()
            .skip(start)
            .take(visible)
            .collect::<Vec<_>>();
        render_tree_rows(
            &visible_items,
            browser.selected,
            inner.width as usize,
            browser.show_label_timestamps,
        )
    };
    frame.render_widget(Paragraph::new(lines), inner);

    match &browser.phase {
        TreePhase::Browse => {
            render_auth_editor(frame, input_area, " search ", &browser.query, false);
        }
        TreePhase::EditLabel { editor, .. } => {
            render_auth_editor(frame, input_area, " label (empty removes) ", editor, false);
        }
        TreePhase::ChooseSummary { selected, .. } => {
            let choices = ["1 no summary", "2 summarize", "3 custom prompt"];
            render_status_input(
                frame,
                input_area,
                &choices
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        if index == *selected {
                            format!("[{value}]")
                        } else {
                            (*value).to_owned()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" · "),
                ORANGE,
            );
        }
        TreePhase::CustomSummary { editor, .. } => {
            render_auth_editor(frame, input_area, " summary instructions ", editor, false);
        }
        TreePhase::Navigating {
            summarizing,
            aborting,
            ..
        } => {
            let text = if *aborting {
                "Cancelling branch summary…"
            } else if *summarizing {
                "Summarizing abandoned branch…  esc cancel"
            } else {
                "Navigating session tree…"
            };
            render_status_input(frame, input_area, text, ORANGE);
        }
    }

    frame.render_widget(
        Paragraph::new(
            " ↑↓ move · ←→ page · ctrl+←/→ branch · ctrl+o filter · shift+l label · shift+t time · ctrl+x copy · enter navigate ",
        )
        .style(Style::default().fg(MUTED)),
        footer_area,
    );
}

const TREE_GUTTER_WIDTH: usize = 2;
const MIN_VISIBLE_TREE_CONTENT_WIDTH: usize = 4;
const MAX_VISIBLE_TREE_CONTENT_WIDTH: usize = 20;
const MIN_TREE_CONTEXT_WIDTH: usize = 2;
const MAX_TREE_CONTEXT_WIDTH: usize = 12;

struct TreeRenderRow {
    body: String,
    anchor_col: usize,
    body_width: usize,
    selected: bool,
}

fn render_tree_rows(
    items: &[(usize, &TreeItem)],
    selected: usize,
    width: usize,
    show_label_timestamps: bool,
) -> Vec<Line<'static>> {
    let rows = items
        .iter()
        .map(|(index, item)| {
            let mut prefix = String::new();
            let connector_position = item.visual_depth.saturating_sub(1);
            for level in 0..item.visual_depth {
                if item.show_connector && level == connector_position {
                    prefix.push(if item.is_last { '└' } else { '├' });
                    prefix.push(if item.folded {
                        '⊞'
                    } else if item.foldable {
                        '⊟'
                    } else {
                        '─'
                    });
                    prefix.push(' ');
                } else if item.gutter_positions.contains(&level) {
                    prefix.push_str("│  ");
                } else {
                    prefix.push_str("   ");
                }
            }
            if item.folded && !item.show_connector {
                prefix.push_str("⊞ ");
            }
            if item.is_active_path {
                prefix.push_str("• ");
            }
            let anchor_col = UnicodeWidthStr::width(prefix.as_str());
            let timestamp = if show_label_timestamps {
                item.label_timestamp
                    .as_deref()
                    .map(|value| format!(" [{value}]"))
                    .unwrap_or_default()
            } else {
                String::new()
            };
            let body = format!("{prefix}{}{timestamp}", item.preview);
            TreeRenderRow {
                body_width: UnicodeWidthStr::width(body.as_str()),
                body,
                anchor_col,
                selected: *index == selected,
            }
        })
        .collect::<Vec<_>>();
    let viewport_width = width.saturating_sub(TREE_GUTTER_WIDTH);
    let max_body_width = rows.iter().map(|row| row.body_width).max().unwrap_or(0);
    let max_horizontal_scroll = max_body_width.saturating_sub(viewport_width);
    let horizontal_scroll = rows.iter().find(|row| row.selected).map_or(0, |row| {
        let minimum_visible_content = MAX_VISIBLE_TREE_CONTENT_WIDTH
            .min(MIN_VISIBLE_TREE_CONTENT_WIDTH.max(viewport_width / 3));
        if row.anchor_col > viewport_width.saturating_sub(minimum_visible_content) {
            let anchor_context =
                MAX_TREE_CONTEXT_WIDTH.min(MIN_TREE_CONTEXT_WIDTH.max(viewport_width / 4));
            row.anchor_col
                .saturating_sub(anchor_context)
                .min(max_horizontal_scroll)
        } else {
            0
        }
    });

    rows.into_iter()
        .map(|row| {
            let marker = if row.selected { "› " } else { "  " };
            let body = if horizontal_scroll > 0 {
                slice_from_visual_column(&row.body, horizontal_scroll, viewport_width)
            } else {
                truncate_to_width(&row.body, viewport_width)
            };
            let padding = " ".repeat(
                width.saturating_sub(TREE_GUTTER_WIDTH + UnicodeWidthStr::width(body.as_str())),
            );
            let line = Line::from(vec![
                Span::styled(
                    marker,
                    Style::default()
                        .fg(if row.selected { CYAN } else { MUTED })
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(body, Style::default().fg(TEXT).add_modifier(Modifier::BOLD)),
                Span::raw(padding),
            ]);
            if row.selected {
                line.style(Style::default().bg(MENU_SELECTED))
            } else {
                line
            }
        })
        .collect()
}

fn slice_from_visual_column(value: &str, start: usize, max_width: usize) -> String {
    if start == 0 {
        return truncate_to_width(value, max_width);
    }
    if max_width == 0 {
        return String::new();
    }

    let mut column = 0;
    let mut byte_index = value.len();
    for (index, character) in value.char_indices() {
        let character_width = character.width().unwrap_or(0);
        if column >= start {
            byte_index = index;
            break;
        }
        column += character_width;
    }
    let content_width = max_width.saturating_sub(1);
    format!(
        "…{}",
        truncate_to_width(&value[byte_index..], content_width)
    )
}

fn render_status_input(frame: &mut Frame<'_>, area: Rect, text: &str, color: Color) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(color));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(
        Paragraph::new(text).style(Style::default().fg(color)),
        inner,
    );
}

fn render_auth(frame: &mut Frame<'_>, state: &AppState) {
    let [content_area, input_area, footer_area] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    render_auth_content(frame, state, content_area);
    render_auth_input(frame, state, input_area);
    render_auth_footer(frame, state, footer_area);
}

fn render_auth_content(frame: &mut Frame<'_>, state: &AppState, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    match &state.auth_state {
        AuthState::Inactive => {}
        AuthState::LoadingProviders => {
            frame.render_widget(
                Paragraph::new(vec![
                    Line::styled(
                        "Sign in to a provider",
                        Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
                    ),
                    Line::default(),
                    Line::styled(
                        "Loading authentication methods…",
                        Style::default().fg(MUTED),
                    ),
                ]),
                area,
            );
        }
        AuthState::Selecting {
            choices,
            selected,
            filter,
        } => {
            let matching = matching_auth_choice_indices(choices, filter.text());
            if matching.is_empty() {
                frame.render_widget(
                    Paragraph::new("No authentication methods match your search.")
                        .style(Style::default().fg(MUTED)),
                    area,
                );
                return;
            }
            let visible = area.height as usize;
            let start = centered_visible_start(matching.len(), *selected, visible);
            let lines = matching
                .iter()
                .enumerate()
                .skip(start)
                .take(visible)
                .map(|(match_index, choice_index)| {
                    let choice = &choices[*choice_index];
                    full_width_choice_line(
                        area.width as usize,
                        match_index == *selected,
                        &choice.provider_name,
                        &format!(
                            "{}{}",
                            choice.label,
                            if choice.configured {
                                " · configured"
                            } else {
                                ""
                            }
                        ),
                    )
                })
                .collect::<Vec<_>>();
            frame.render_widget(Paragraph::new(lines), area);
        }
        AuthState::Running(flow) => {
            let mut lines = vec![
                Line::from(vec![
                    Span::styled(
                        "Sign in  ",
                        Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        flow.provider_name.clone(),
                        Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::default(),
                Line::styled(flow.status.clone(), Style::default().fg(MUTED)),
            ];
            if let Some(url) = flow.url.as_ref() {
                lines.push(Line::styled(
                    url.clone(),
                    Style::default().fg(CYAN).add_modifier(Modifier::UNDERLINED),
                ));
            }
            if let Some(code) = flow.device_code.as_ref() {
                lines.push(Line::from(vec![
                    Span::styled("Code  ", Style::default().fg(MUTED)),
                    Span::styled(
                        code.clone(),
                        Style::default().fg(ORANGE).add_modifier(Modifier::BOLD),
                    ),
                ]));
            }
            if let Some(prompt) = flow.prompt.as_ref() {
                lines.push(Line::default());
                lines.push(Line::styled(
                    prompt.message.clone(),
                    Style::default().fg(TEXT),
                ));
                if prompt.kind == AuthPromptKind::Select {
                    for (index, option) in prompt.options.iter().enumerate() {
                        lines.push(full_width_choice_line(
                            area.width as usize,
                            index == prompt.selected,
                            &format!("{}. {}", index + 1, option.label),
                            option.description.as_deref().unwrap_or_default(),
                        ));
                    }
                }
            }
            frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
        }
    }
}

fn full_width_choice_line(width: usize, selected: bool, left: &str, right: &str) -> Line<'static> {
    choice_line(width, selected, true, left, right)
}

fn choice_line(
    width: usize,
    selected: bool,
    enabled: bool,
    left: &str,
    right: &str,
) -> Line<'static> {
    let marker = if selected { "› " } else { "  " };
    let marker_width = UnicodeWidthStr::width(marker);
    let left = truncate_to_width(left, width.saturating_sub(marker_width + 1));
    let left_width = UnicodeWidthStr::width(left.as_str());
    let right = truncate_to_width(right, width.saturating_sub(marker_width + left_width + 1));
    let right_width = UnicodeWidthStr::width(right.as_str());
    let padding = " ".repeat(width.saturating_sub(marker_width + left_width + right_width));
    let line = Line::from(vec![
        Span::styled(
            marker,
            Style::default()
                .fg(if selected && enabled { CYAN } else { MUTED })
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            left,
            Style::default()
                .fg(if enabled { TEXT } else { MUTED })
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(padding),
        Span::styled(
            right,
            Style::default().fg(if selected && enabled { TEXT } else { MUTED }),
        ),
    ]);
    if selected && enabled {
        line.style(Style::default().bg(MENU_SELECTED))
    } else {
        line
    }
}

fn render_auth_input(frame: &mut Frame<'_>, state: &AppState, area: Rect) {
    if area.width < 4 || area.height < 3 {
        return;
    }

    if let AuthState::Selecting { filter, .. } = &state.auth_state {
        render_auth_editor(frame, area, " search ", filter, false);
        return;
    }

    let prompt = match &state.auth_state {
        AuthState::Running(flow) => flow.prompt.as_ref(),
        _ => None,
    };
    if let Some(prompt) = prompt.filter(|prompt| prompt.kind != AuthPromptKind::Select) {
        let label = match prompt.kind {
            AuthPromptKind::Secret => " secret ",
            AuthPromptKind::ManualCode => " authorization code ",
            AuthPromptKind::Text => " input ",
            AuthPromptKind::Select => unreachable!("select prompts do not use the editor"),
        };
        render_auth_editor(
            frame,
            area,
            label,
            &prompt.editor,
            prompt.kind == AuthPromptKind::Secret,
        );
        return;
    }

    let label = if prompt.is_some() {
        " select "
    } else {
        " login "
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(CYAN))
        .title_bottom(
            Line::from(Span::styled(label, Style::default().fg(CYAN))).alignment(Alignment::Right),
        );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let message = match state.auth_state {
        AuthState::LoadingProviders => "Loading…",
        AuthState::Running(_) if prompt.is_some() => "Choose an option above",
        _ => "Waiting for authentication…",
    };
    frame.render_widget(
        Paragraph::new(message).style(Style::default().fg(MUTED)),
        inner,
    );
}

fn render_auth_editor(
    frame: &mut Frame<'_>,
    area: Rect,
    label: &'static str,
    editor: &EditorState,
    secret: bool,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(CYAN))
        .title_bottom(
            Line::from(Span::styled(label, Style::default().fg(CYAN))).alignment(Alignment::Right),
        );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let display = if secret {
        "•".repeat(editor.text().chars().count())
    } else {
        editor.text().to_owned()
    };
    let before_cursor = editor
        .text()
        .chars()
        .take(editor.cursor())
        .collect::<String>();
    let cursor_width = if secret {
        editor.cursor()
    } else {
        UnicodeWidthStr::width(before_cursor.as_str())
    };
    let prompt_width = 2;
    let horizontal_scroll =
        (prompt_width + cursor_width).saturating_sub((inner.width as usize).saturating_sub(1));
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("› ", Style::default().fg(CYAN)),
            Span::styled(display, Style::default().fg(TEXT)),
        ]))
        .scroll((0, horizontal_scroll as u16)),
        inner,
    );
    frame.set_cursor_position(Position::new(
        inner.x
            + prompt_width
                .saturating_add(cursor_width)
                .saturating_sub(horizontal_scroll)
                .min(inner.width.saturating_sub(1) as usize) as u16,
        inner.y,
    ));
}

fn render_auth_footer(frame: &mut Frame<'_>, state: &AppState, area: Rect) {
    let help = match &state.auth_state {
        AuthState::Selecting { .. } => {
            " type to filter  ↑↓ · tab/shift-tab · ctrl+n/p navigate  enter select  esc cancel "
        }
        AuthState::Running(flow)
            if flow
                .prompt
                .as_ref()
                .is_some_and(|prompt| prompt.kind == AuthPromptKind::Select) =>
        {
            " ↑↓ · tab/shift-tab · ctrl+n/p navigate  1-9 select  enter confirm  esc cancel "
        }
        AuthState::Running(flow) if flow.prompt.is_some() => {
            " enter continue  ctrl+u clear  esc cancel "
        }
        _ => " esc cancel ",
    };
    frame.render_widget(Paragraph::new(help).style(Style::default().fg(MUTED)), area);
}

pub fn render_terminal_overlays(state: &AppState, viewport: Rect) -> io::Result<()> {
    let Some(sequence) = auth_hyperlink_sequence(state, viewport) else {
        return Ok(());
    };
    let mut output = io::stdout().lock();
    output.write_all(sequence.as_bytes())?;
    output.flush()
}

fn auth_hyperlink_sequence(state: &AppState, viewport: Rect) -> Option<String> {
    let AuthState::Running(flow) = &state.auth_state else {
        return None;
    };
    let url = flow.url.as_deref().filter(|url| is_safe_web_url(url))?;
    let [content_area, _, _] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .areas(viewport);
    if content_area.width == 0 || content_area.height < 4 {
        return None;
    }
    let label = truncate_to_width(url, content_area.width as usize);
    Some(osc8_hyperlink(
        url,
        &label,
        Position::new(content_area.x, content_area.y.saturating_add(3)),
    ))
}

fn osc8_hyperlink(url: &str, label: &str, position: Position) -> String {
    format!(
        "\x1b7\x1b[{};{}H\x1b]8;;{}\x1b\\\x1b[38;2;91;196;214m\x1b[4m{}\x1b]8;;\x1b\\\x1b[0m\x1b8",
        position.y.saturating_add(1),
        position.x.saturating_add(1),
        url,
        label,
    )
}

fn render_active_output(frame: &mut Frame<'_>, lines: Vec<Line<'static>>, area: Rect) {
    render_bottom_aligned_lines(frame, lines, area);
}

pub fn render_recent_history_background(
    frame: &mut Frame<'_>,
    presenter: &TranscriptPresenter,
    area: Rect,
) {
    render_bottom_aligned_lines(frame, presenter.recent_history_lines.clone(), area);
}

fn render_bottom_aligned_lines(frame: &mut Frame<'_>, lines: Vec<Line<'static>>, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let content_height = visual_height(&lines, area.width as usize);
    let rendered_height = content_height.min(area.height);
    let content_area = Rect {
        y: area.bottom().saturating_sub(rendered_height),
        height: rendered_height,
        ..area
    };
    let scroll = content_height.saturating_sub(content_area.height);
    let paragraph = Paragraph::new(lines)
        .style(Style::default().fg(TEXT))
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph.scroll((scroll, 0)), content_area);
}

fn render_command_menu(frame: &mut Frame<'_>, state: &AppState, area: Rect) {
    if area.width < 4 || area.height == 0 {
        return;
    }

    let candidates = state.command_candidates();
    let visible = area.height as usize;
    let selected = state
        .command_menu_selected()
        .min(candidates.len().saturating_sub(1));
    let start = centered_visible_start(candidates.len(), selected, visible);
    let row_width = area.width as usize;
    let lines = candidates
        .iter()
        .enumerate()
        .skip(start)
        .take(visible)
        .map(|(index, command)| {
            let selected = index == state.command_menu_selected();
            let marker = if selected { "› " } else { "  " };
            let marker_width = UnicodeWidthStr::width(marker);
            let name = truncate_to_width(
                &format!("/{}", command.name),
                row_width.saturating_sub(marker_width),
            );
            let name_width = UnicodeWidthStr::width(name.as_str());
            let description = truncate_to_width(
                &command.description,
                row_width.saturating_sub(marker_width + name_width + 2),
            );
            let description_width = UnicodeWidthStr::width(description.as_str());
            let padding =
                " ".repeat(row_width.saturating_sub(marker_width + name_width + description_width));
            let line = Line::from(vec![
                Span::styled(
                    marker,
                    Style::default()
                        .fg(if selected { CYAN } else { MUTED })
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(name, Style::default().fg(TEXT).add_modifier(Modifier::BOLD)),
                Span::raw(padding),
                Span::styled(
                    description,
                    Style::default().fg(if selected { TEXT } else { MUTED }),
                ),
            ]);
            if selected {
                line.style(Style::default().bg(MENU_SELECTED))
            } else {
                line
            }
        })
        .collect::<Vec<_>>();

    frame.render_widget(Paragraph::new(lines), area);
}

fn truncate_to_width(value: &str, max_width: usize) -> String {
    if UnicodeWidthStr::width(value) <= max_width {
        return value.to_owned();
    }
    if max_width == 0 {
        return String::new();
    }

    let content_width = max_width.saturating_sub(1);
    let mut width = 0;
    let mut truncated = String::new();
    for character in value.chars() {
        let character_width = character.width().unwrap_or(0);
        if width + character_width > content_width {
            break;
        }
        truncated.push(character);
        width += character_width;
    }
    truncated.push('…');
    truncated
}

fn render_input(frame: &mut Frame<'_>, state: &AppState, area: Rect) {
    if area.width < 4 || area.height < 3 {
        return;
    }

    let accent = input_accent(state);
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(accent))
        .title_bottom(
            Line::from(Span::styled(
                format!(" {} ", state.run_state.label()),
                Style::default().fg(accent),
            ))
            .alignment(Alignment::Right),
        );
    if state.plan_mode_active || state.pending_plan_mode.is_some() {
        let label = match state.pending_plan_mode {
            Some(true) => " PLAN… ",
            Some(false) => " leaving PLAN… ",
            None => " PLAN ",
        };
        block = block.title(Line::from(Span::styled(
            label,
            Style::default().fg(VIOLET).add_modifier(Modifier::BOLD),
        )));
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width < 3 {
        return;
    }

    if let Some(flow) = state.question.as_ref()
        && flow.custom_answer
    {
        render_auth_editor(frame, area, " custom answer ", &flow.editor, false);
        return;
    }
    if let Some(panel) = short_choice_panel(state) {
        frame.render_widget(
            Paragraph::new(panel.status).style(Style::default().fg(if panel.submitting {
                ORANGE
            } else {
                MUTED
            })),
            inner,
        );
        return;
    }

    let [prompt_area, text_area] =
        Layout::horizontal([Constraint::Length(2), Constraint::Min(1)]).areas(inner);
    frame.render_widget(
        Paragraph::new(Span::styled(
            "› ",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        )),
        prompt_area,
    );

    let composer = state
        .editor
        .composer_viewport(text_area.width.max(1) as usize, text_area.height.max(1));
    frame.render_widget(
        Paragraph::new(state.editor.text())
            .style(Style::default().fg(TEXT))
            .wrap(Wrap { trim: false })
            .scroll((composer.first_visual_row.min(u16::MAX as usize) as u16, 0)),
        text_area,
    );

    let cursor_x = text_area.x
        + composer
            .cursor_visual_column
            .min(text_area.width.saturating_sub(1) as usize) as u16;
    let cursor_y = text_area.y
        + composer
            .cursor_visual_row
            .saturating_sub(composer.first_visual_row)
            .min(text_area.height.saturating_sub(1) as usize) as u16;
    frame.set_cursor_position(Position::new(cursor_x, cursor_y));
}

fn render_footer(frame: &mut Frame<'_>, state: &AppState, area: Rect) {
    let menu_visible = !state.command_candidates().is_empty();
    let help = if let Some(panel) = short_choice_panel(state) {
        if state
            .question
            .as_ref()
            .is_some_and(|question| question.custom_answer)
        {
            " enter answer  ctrl+u clear  esc back  ctrl+c interrupt ".to_owned()
        } else {
            format!(
                " ↑↓ · tab/shift-tab · ctrl+n/p navigate  1-{} select  enter confirm  esc {} ",
                panel.items.len().min(9),
                panel.cancel_label
            )
        }
    } else {
        match state.active_modal_kind() {
            _ if menu_visible => {
                " ↑↓ · tab/shift-tab · ctrl+n/p navigate  enter complete  esc close "
            }
            _ if state.can_abort() => {
                " enter steer  alt+enter follow-up  alt+up restore queue  esc interrupt "
            }
            _ => " enter send  shift+tab Plan  ctrl+o transcript  ctrl+u clear  ctrl+c quit ",
        }
        .to_owned()
    };
    let context = context_footer_label(&state.context);
    let goal = state
        .goal
        .as_ref()
        .and_then(|snapshot| snapshot.goal.as_ref())
        .map_or_else(String::new, |goal| format!(" · goal {}", goal.stage));
    let agents = if state.agents.active.is_empty() && state.agents.pending.is_empty() {
        String::new()
    } else {
        format!(
            " · agents {}+{}",
            state.agents.active.len(),
            state.agents.pending.len()
        )
    };
    let plan = match state.pending_plan_mode {
        Some(true) => "PLAN… · ",
        Some(false) => "leaving PLAN… · ",
        None if state.plan_mode_active => "PLAN · ",
        None => "",
    };
    let details_with_goal = format!(
        "{}{} · {}{}{} · {} ",
        plan,
        state.model_label(),
        context,
        goal,
        agents,
        state.run_state.label()
    );
    let details_with_context = format!(
        "{}{} · {} · {} ",
        plan,
        state.model_label(),
        context,
        state.run_state.label()
    );
    let details_without_context = format!(
        "{}{} · {} ",
        plan,
        state.model_label(),
        state.run_state.label()
    );
    let minimal_details = format!("{} ", state.run_state.label());
    let available = area.width as usize;
    let help_width = UnicodeWidthStr::width(help.as_str());
    let details = [
        details_with_goal,
        details_with_context,
        details_without_context,
        minimal_details.clone(),
    ]
    .into_iter()
    .find(|details| help_width + UnicodeWidthStr::width(details.as_str()) <= available)
    .unwrap_or(minimal_details);
    let detail_width = UnicodeWidthStr::width(details.as_str());
    let rendered_help = if help_width + detail_width <= available {
        help
    } else {
        truncate_to_width(&help, available.saturating_sub(detail_width))
    };
    let rendered_help_width = UnicodeWidthStr::width(rendered_help.as_str());
    let gap = available.saturating_sub(rendered_help_width + detail_width);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(rendered_help, Style::default().fg(MUTED)),
            Span::raw(" ".repeat(gap)),
            Span::styled(details, Style::default().fg(MUTED)),
        ])),
        area,
    );
}

fn context_footer_label(snapshot: &ContextSnapshot) -> String {
    match snapshot.usage_state {
        ContextUsageState::Recalculating => "ctx …".to_owned(),
        ContextUsageState::Actual => snapshot.actual_percent.map_or_else(
            || "ctx …".to_owned(),
            |percent| format!("ctx {:.0}%", percent.clamp(0.0, 999.0)),
        ),
        ContextUsageState::Estimated => match snapshot.context_window {
            Some(window) if window > 0 => format!(
                "ctx ~{:.0}%",
                (snapshot.estimated_next_request_tokens as f64 / window as f64 * 100.0)
                    .clamp(0.0, 999.0)
            ),
            _ => "ctx …".to_owned(),
        },
    }
}

fn project_assistant(
    message: &AssistantMessage,
    projection: &mut AssistantProjection,
    terminal_width: usize,
    lines: &mut Vec<Line<'static>>,
) {
    let content_width = terminal_width.saturating_sub(2).max(1);
    let thinking_finished = message.complete || !message.text.is_empty();
    project_field(
        &message.thinking,
        &mut projection.thinking_offset,
        &mut projection.thinking_started,
        "· ",
        Style::default().fg(VIOLET),
        Style::default().fg(MUTED).add_modifier(Modifier::ITALIC),
        content_width,
        thinking_finished,
        lines,
    );

    if projection.thinking_offset == message.thinking.len() {
        project_markdown_field(
            &message.text,
            &mut projection.text_offset,
            &mut projection.text_started,
            &mut projection.markdown,
            content_width,
            message.complete,
            lines,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn project_field(
    value: &str,
    offset: &mut usize,
    started: &mut bool,
    first_prefix: &str,
    prefix_style: Style,
    content_style: Style,
    content_width: usize,
    finish: bool,
    lines: &mut Vec<Line<'static>>,
) {
    while let Some(row) = next_stable_row(value, *offset, content_width, finish) {
        let prefix = if *started { "  " } else { first_prefix };
        lines.push(Line::from(vec![
            Span::styled(prefix.to_owned(), prefix_style),
            Span::styled(row.text, content_style),
        ]));
        *offset = row.next_offset;
        *started = true;
    }
}

fn project_markdown_field(
    value: &str,
    offset: &mut usize,
    started: &mut bool,
    markdown: &mut MarkdownState,
    content_width: usize,
    finish: bool,
    lines: &mut Vec<Line<'static>>,
) {
    while let Some(row) = next_stable_markdown_row(value, *offset, content_width, finish) {
        let prefix = if *started { "  " } else { "• " };
        push_markdown_block_row(
            lines,
            prefix,
            Style::default().fg(VIOLET),
            &row.text,
            markdown,
            content_width,
        );
        *offset = row.next_offset;
        *started = true;
    }
    if finish && *offset == value.len() {
        finish_markdown_blocks(lines, markdown, content_width);
    }
}

fn next_stable_markdown_row(
    value: &str,
    start: usize,
    max_width: usize,
    finish: bool,
) -> Option<StableRow> {
    let remaining = value.get(start..)?;
    if remaining.is_empty() {
        return None;
    }
    if let Some(relative_index) = remaining.find('\n') {
        return Some(StableRow {
            text: remaining[..relative_index].to_owned(),
            next_offset: start + relative_index + 1,
        });
    }
    if !remaining.contains('|') {
        return next_stable_row(value, start, max_width, finish);
    }
    finish.then(|| StableRow {
        text: remaining.to_owned(),
        next_offset: value.len(),
    })
}

fn assistant_remainder_lines_with_width(
    message: &AssistantMessage,
    projection: &AssistantProjection,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    push_remainder(
        &mut lines,
        &message.thinking,
        projection.thinking_offset,
        projection.thinking_started,
        "· ",
        Style::default().fg(VIOLET),
        Style::default().fg(MUTED).add_modifier(Modifier::ITALIC),
    );
    let mut markdown = projection.markdown.clone();
    if let Some(remainder) = message
        .text
        .get(projection.text_offset..)
        .filter(|remainder| !remainder.is_empty())
    {
        let prefix = if projection.text_started {
            "  "
        } else {
            "• "
        };
        let content_width = width.saturating_sub(2).max(1);
        if message.complete {
            push_markdown_with_state(
                &mut lines,
                prefix,
                remainder,
                Style::default().fg(VIOLET),
                &mut markdown,
                content_width,
                true,
            );
        } else {
            push_streaming_markdown_tail(
                &mut lines,
                prefix,
                remainder,
                Style::default().fg(VIOLET),
                &mut markdown,
                content_width,
            );
        }
    }
    if !message.complete {
        finish_markdown_blocks(&mut lines, &mut markdown, width.saturating_sub(2).max(1));
    }
    lines
}

fn push_streaming_markdown_tail(
    lines: &mut Vec<Line<'static>>,
    first_prefix: &str,
    content: &str,
    prefix_style: Style,
    state: &mut MarkdownState,
    width: usize,
) {
    let mut remaining = content;
    let mut prefix = first_prefix;
    while let Some(newline) = remaining.find('\n') {
        push_markdown_block_row(
            lines,
            prefix,
            prefix_style,
            &remaining[..newline],
            state,
            width,
        );
        remaining = &remaining[newline + 1..];
        prefix = "  ";
    }
    if remaining.is_empty() {
        return;
    }
    if let Some(table) = state.table.as_ref()
        && split_table_cells(remaining).is_none_or(|cells| cells.len() != table.headers.len())
    {
        return;
    }
    push_markdown_block_row(lines, prefix, prefix_style, remaining, state, width);
}

fn push_remainder(
    lines: &mut Vec<Line<'static>>,
    value: &str,
    offset: usize,
    started: bool,
    first_prefix: &str,
    prefix_style: Style,
    content_style: Style,
) {
    let Some(remainder) = value
        .get(offset..)
        .filter(|remainder| !remainder.is_empty())
    else {
        return;
    };
    push_prefixed(
        lines,
        if started { "  " } else { first_prefix },
        remainder,
        prefix_style,
        content_style,
    );
}

struct StableRow {
    text: String,
    next_offset: usize,
}

fn next_stable_row(value: &str, start: usize, max_width: usize, finish: bool) -> Option<StableRow> {
    let remaining = value.get(start..)?;
    if remaining.is_empty() {
        return None;
    }

    let max_width = max_width.max(1);
    let mut width = 0;
    for (relative_index, character) in remaining.char_indices() {
        let absolute_index = start + relative_index;
        if character == '\n' {
            return Some(StableRow {
                text: value[start..absolute_index].to_owned(),
                next_offset: absolute_index + character.len_utf8(),
            });
        }

        let character_width = character.width().unwrap_or(0);
        if width > 0 && width + character_width > max_width {
            return Some(StableRow {
                text: value[start..absolute_index].to_owned(),
                next_offset: absolute_index,
            });
        }

        width += character_width;
        if width >= max_width {
            let end = absolute_index + character.len_utf8();
            let next_offset = if value[end..].starts_with('\n') {
                end + 1
            } else {
                end
            };
            return Some(StableRow {
                text: value[start..end].to_owned(),
                next_offset,
            });
        }
    }

    finish.then(|| StableRow {
        text: remaining.to_owned(),
        next_offset: value.len(),
    })
}

#[cfg(test)]
fn item_lines(item: &TranscriptItem) -> Vec<Line<'static>> {
    item_lines_with_width(item, 80)
}

fn item_lines_with_width(item: &TranscriptItem, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    match item {
        TranscriptItem::User(message) => {
            let suffix = match message.status {
                UserMessageStatus::Pending => "  …",
                UserMessageStatus::Accepted => "",
                UserMessageStatus::Failed => "  failed",
            };
            push_prefixed(
                &mut lines,
                "› ",
                &format!("{}{}", message.text, suffix),
                Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
                Style::default().fg(TEXT),
            );
        }
        TranscriptItem::Assistant(message) => {
            if !message.thinking.is_empty() {
                push_prefixed(
                    &mut lines,
                    "· ",
                    &message.thinking,
                    Style::default().fg(VIOLET),
                    Style::default().fg(MUTED).add_modifier(Modifier::ITALIC),
                );
            }
            if !message.text.is_empty() {
                push_markdown_width(
                    &mut lines,
                    "• ",
                    &message.text,
                    Style::default().fg(VIOLET),
                    width,
                );
            }
        }
        TranscriptItem::Tool(tool) => {
            let (symbol, status, style) = tool_status_visual(tool.status);
            let read_only_collapsed = matches!(tool.name.as_str(), "read" | "grep" | "find" | "ls")
                && tool.status == ToolStatus::Succeeded
                && !tool.output.is_empty();
            let summary = tool_request_summary(&tool.name, &tool.args);
            let output_summary = read_only_collapsed.then(|| {
                format!(
                    "{} lines · {} · output collapsed",
                    line_count(&tool.output),
                    human_bytes(tool.output.len())
                )
            });
            let details = [(!summary.is_empty()).then_some(summary), output_summary]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{}{} ", if read_only_collapsed { "▸ " } else { "" }, symbol),
                    style,
                ),
                Span::styled(
                    tool.name.clone(),
                    Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" · {status}"), style),
                Span::styled(
                    if details.is_empty() {
                        String::new()
                    } else {
                        format!(" · {details}")
                    },
                    Style::default().fg(MUTED),
                ),
            ]));
            if !read_only_collapsed {
                push_tool_output(&mut lines, tool);
            }
        }
        TranscriptItem::Plan(plan) => {
            lines.push(Line::from(vec![
                Span::styled(
                    "◆ ",
                    Style::default().fg(VIOLET).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        "{}  r{} · {}",
                        plan.title,
                        plan.revision,
                        plan.status.label()
                    ),
                    Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
                ),
            ]));
            push_markdown_width(
                &mut lines,
                "  ",
                &plan.summary,
                Style::default().fg(MUTED),
                width,
            );
            push_markdown_width(
                &mut lines,
                "  ",
                &plan.body_markdown,
                Style::default().fg(MUTED),
                width,
            );
            if !plan.assumptions.is_empty() {
                push_markdown_width(
                    &mut lines,
                    "  ",
                    &format!(
                        "### Assumptions\n{}",
                        plan.assumptions
                            .iter()
                            .map(|item| format!("- {item}"))
                            .collect::<Vec<_>>()
                            .join("\n")
                    ),
                    Style::default().fg(MUTED),
                    width,
                );
            }
            if !plan.test_plan.is_empty() {
                push_markdown_width(
                    &mut lines,
                    "  ",
                    &format!(
                        "### Test plan\n{}",
                        plan.test_plan
                            .iter()
                            .map(|item| format!("- {item}"))
                            .collect::<Vec<_>>()
                            .join("\n")
                    ),
                    Style::default().fg(MUTED),
                    width,
                );
            }
        }
        TranscriptItem::Context(snapshot) => push_context_snapshot(&mut lines, snapshot),
        TranscriptItem::Resources(snapshot) => {
            lines.push(Line::from(vec![
                Span::styled(
                    "◇ resources ",
                    Style::default().fg(VIOLET).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        "r{} · {} · context {} · skills {} · prompts {} · extensions {}",
                        snapshot.revision,
                        if snapshot.trusted {
                            "trusted project"
                        } else {
                            "global only"
                        },
                        snapshot.context_files.len(),
                        snapshot.skills.len(),
                        snapshot.prompts.len(),
                        snapshot.extensions.len()
                    ),
                    Style::default().fg(TEXT),
                ),
            ]));
            for path in &snapshot.context_files {
                push_prefixed(
                    &mut lines,
                    "  context  ",
                    path,
                    Style::default().fg(CYAN),
                    Style::default().fg(MUTED),
                );
            }
            for skill in &snapshot.skills {
                push_prefixed(
                    &mut lines,
                    "  skill    ",
                    &format!("{} · {}", skill.name, skill.path),
                    Style::default().fg(CYAN),
                    Style::default().fg(MUTED),
                );
            }
            for prompt in &snapshot.prompts {
                push_prefixed(
                    &mut lines,
                    "  prompt   ",
                    &format!("{} · {}", prompt.name, prompt.path),
                    Style::default().fg(CYAN),
                    Style::default().fg(MUTED),
                );
            }
            for extension in &snapshot.extensions {
                push_prefixed(
                    &mut lines,
                    "  extension  ",
                    extension,
                    Style::default().fg(CYAN),
                    Style::default().fg(MUTED),
                );
            }
            for diagnostic in &snapshot.diagnostics {
                push_prefixed(
                    &mut lines,
                    "  ! ",
                    &diagnostic.message,
                    Style::default().fg(ORANGE),
                    Style::default().fg(MUTED),
                );
            }
        }
        TranscriptItem::Goal(snapshot) => {
            if let Some(goal) = snapshot.goal.as_ref() {
                lines.push(Line::from(vec![
                    Span::styled(
                        "◆ goal ",
                        Style::default().fg(TEAL).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("{} · {} · r{}", goal.id, goal.stage, goal.revision),
                        Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
                    ),
                ]));
                push_prefixed(
                    &mut lines,
                    "  ",
                    &goal.objective,
                    Style::default().fg(TEAL),
                    Style::default().fg(TEXT),
                );
                if let Some(spec) = goal.spec.as_ref() {
                    push_prefixed(
                        &mut lines,
                        "  spec  ",
                        &format!("r{} · {}", spec.revision, spec.summary),
                        Style::default().fg(TEAL),
                        Style::default().fg(MUTED),
                    );
                }
                for task in &goal.tasks {
                    push_prefixed(
                        &mut lines,
                        "  task  ",
                        &format!(
                            "{} · {} · {} [{}]",
                            task.id, task.status, task.title, task.profile
                        ),
                        Style::default().fg(TEAL),
                        Style::default().fg(MUTED),
                    );
                }
                if let Some(error) = goal.last_error.as_ref() {
                    push_prefixed(
                        &mut lines,
                        "  ! ",
                        error,
                        Style::default().fg(RED),
                        Style::default().fg(RED),
                    );
                }
                push_prefixed(
                    &mut lines,
                    "  state  ",
                    &snapshot.state_path,
                    Style::default().fg(MUTED),
                    Style::default().fg(MUTED),
                );
            } else {
                lines.push(Line::styled(
                    "◇ no structured Goal in this session",
                    Style::default().fg(MUTED),
                ));
            }
        }
        TranscriptItem::Goals(snapshot) => {
            lines.push(Line::from(vec![
                Span::styled(
                    "◇ goals ",
                    Style::default().fg(VIOLET).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{} sidecars", snapshot.goals.len()),
                    Style::default().fg(TEXT),
                ),
            ]));
            for goal in &snapshot.goals {
                push_prefixed(
                    &mut lines,
                    "  ",
                    &format!(
                        "{} · {} · r{} · {} · session {}",
                        goal.id, goal.stage, goal.revision, goal.objective, goal.session_id
                    ),
                    Style::default().fg(CYAN),
                    Style::default().fg(MUTED),
                );
            }
            push_prefixed(
                &mut lines,
                "  state  ",
                &snapshot.state_directory,
                Style::default().fg(MUTED),
                Style::default().fg(MUTED),
            );
        }
        TranscriptItem::Agents(snapshot) => {
            lines.push(Line::from(vec![
                Span::styled(
                    "◇ agents ",
                    Style::default().fg(VIOLET).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        "{} profiles · {} active · {} pending integration · max {}",
                        snapshot.profiles.len(),
                        snapshot.active.len(),
                        snapshot.pending.len(),
                        snapshot.max_parallel
                    ),
                    Style::default().fg(TEXT),
                ),
            ]));
            for profile in &snapshot.profiles {
                push_prefixed(
                    &mut lines,
                    "  ",
                    &format!(
                        "{} · {} · {} · turns {} · parallel {} · isolation {}/{} · {}",
                        profile.name,
                        profile.model.as_deref().unwrap_or("primary model"),
                        profile.permission,
                        profile.max_turns,
                        profile.max_parallel,
                        profile.isolation.mode,
                        profile.isolation.integration,
                        if profile.disabled {
                            "disabled".to_owned()
                        } else {
                            format!("tools {}", profile.tools.join(","))
                        },
                    ),
                    Style::default().fg(CYAN),
                    Style::default().fg(MUTED),
                );
                push_prefixed(
                    &mut lines,
                    "    ",
                    &format!(
                        "{} · {}{}",
                        profile.description,
                        profile.source,
                        profile
                            .unavailable_reason
                            .as_deref()
                            .map_or_else(String::new, |reason| format!(" · unavailable: {reason}"))
                    ),
                    Style::default().fg(MUTED),
                    Style::default().fg(MUTED),
                );
            }
            for agent in &snapshot.active {
                push_prefixed(
                    &mut lines,
                    "  running  ",
                    &format!(
                        "{} [{}] {} · {}/{} turns · {}",
                        agent.id,
                        agent.profile,
                        agent.lifecycle,
                        agent.turns,
                        agent.max_turns,
                        agent.model
                    ),
                    Style::default().fg(ORANGE),
                    Style::default().fg(TEXT),
                );
                push_prefixed(
                    &mut lines,
                    "    task  ",
                    &agent.task,
                    Style::default().fg(MUTED),
                    Style::default().fg(TEXT),
                );
            }
            for agent in &snapshot.pending {
                push_prefixed(
                    &mut lines,
                    "  pending  ",
                    &format!(
                        "{} [{}] {} · {}",
                        agent.id, agent.profile, agent.integration_status, agent.task
                    ),
                    Style::default().fg(ORANGE),
                    Style::default().fg(TEXT),
                );
            }
            for diagnostic in &snapshot.diagnostics {
                push_prefixed(
                    &mut lines,
                    "  config  ",
                    &format!(
                        "{}{}",
                        diagnostic.message,
                        diagnostic
                            .path
                            .as_deref()
                            .map_or_else(String::new, |path| format!(" · {path}"))
                    ),
                    Style::default().fg(RED),
                    Style::default().fg(MUTED),
                );
            }
        }
        TranscriptItem::Subagent(item) => {
            let color = match item.event.as_str() {
                "completed" => GREEN,
                "failed" | "limit_reached" => RED,
                "cancelled" => ORANGE,
                _ => VIOLET,
            };
            lines.push(Line::from(vec![
                Span::styled(
                    "◇ subagent ",
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        "{} [{}] {} · {} · {}/{} turns · {}/{}",
                        item.agent.id,
                        item.agent.profile,
                        item.event,
                        item.agent.model,
                        item.agent.turns,
                        item.agent.max_turns,
                        item.agent.isolation_backend,
                        item.agent.integration_status,
                    ),
                    Style::default().fg(TEXT),
                ),
            ]));
            push_prefixed(
                &mut lines,
                "  task  ",
                &item.agent.task,
                Style::default().fg(MUTED),
                Style::default().fg(TEXT),
            );
            if let Some(summary) = item
                .result
                .as_ref()
                .and_then(|result| result.get("summary"))
                .and_then(serde_json::Value::as_str)
            {
                push_prefixed(
                    &mut lines,
                    "  result  ",
                    summary,
                    Style::default().fg(MUTED),
                    Style::default().fg(TEXT),
                );
            }
            if let Some(error) = item.error.as_deref() {
                push_prefixed(
                    &mut lines,
                    "  error  ",
                    error,
                    Style::default().fg(RED),
                    Style::default().fg(RED),
                );
            }
        }
        TranscriptItem::Compaction(record) => {
            let files = record.file_count();
            let verb = if record.reason == "restored" {
                "restored compaction"
            } else {
                "compacted"
            };
            let separator = match (record.estimated_tokens_after, record.saved_percent) {
                (Some(after), Some(percent)) => format!(
                    "── {verb} · {} → {} · saved {:.0}% · files {} ──",
                    human_tokens(record.tokens_before),
                    human_tokens(after),
                    percent,
                    files
                ),
                _ => format!(
                    "── {verb} · before {} · files {} ──",
                    human_tokens(record.tokens_before),
                    files
                ),
            };
            lines.push(Line::styled(
                separator,
                Style::default().fg(VIOLET).add_modifier(Modifier::BOLD),
            ));
        }
        TranscriptItem::BranchSummary(summary) => {
            lines.push(Line::styled(
                "◇ branch summary",
                Style::default().fg(VIOLET).add_modifier(Modifier::BOLD),
            ));
            push_markdown_width(&mut lines, "  ", summary, Style::default().fg(MUTED), width);
        }
        TranscriptItem::SessionBoundary { action, label, cwd } => {
            lines.push(Line::styled(
                format!("── {action} · {label} · {cwd} ──"),
                Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
            ));
        }
        TranscriptItem::Notice(message) => push_prefixed(
            &mut lines,
            "› ",
            message,
            Style::default().fg(ORANGE),
            Style::default().fg(MUTED),
        ),
        TranscriptItem::Error(message) => push_prefixed(
            &mut lines,
            "× ",
            message,
            Style::default().fg(RED).add_modifier(Modifier::BOLD),
            Style::default().fg(RED),
        ),
    }
    lines.push(Line::default());
    lines
}

fn push_context_snapshot(lines: &mut Vec<Line<'static>>, snapshot: &ContextSnapshot) {
    let actual = match snapshot.usage_state {
        ContextUsageState::Actual => match (
            snapshot.actual_tokens,
            snapshot.actual_percent,
            snapshot.context_window,
        ) {
            (Some(tokens), Some(percent), Some(window)) => format!(
                "last request {} / {} ({percent:.0}%)",
                human_tokens(tokens),
                human_tokens(window)
            ),
            (Some(tokens), _, _) => format!("last request {}", human_tokens(tokens)),
            _ => "last request unavailable".to_owned(),
        },
        ContextUsageState::Estimated => "no completed request measurement yet".to_owned(),
        ContextUsageState::Recalculating => {
            "recalculating after compaction; waiting for the next response".to_owned()
        }
    };
    lines.push(Line::from(vec![
        Span::styled(
            "◇ Context  ",
            Style::default().fg(VIOLET).add_modifier(Modifier::BOLD),
        ),
        Span::styled(actual, Style::default().fg(TEXT)),
    ]));

    let next_percent = snapshot
        .context_window
        .filter(|window| *window > 0)
        .map(|window| snapshot.estimated_next_request_tokens as f64 / window as f64 * 100.0);
    let next = next_percent.map_or_else(
        || {
            format!(
                "unfiltered ~{} · next request ~{}",
                human_tokens(snapshot.estimated_unfiltered_tokens),
                human_tokens(snapshot.estimated_next_request_tokens)
            )
        },
        |percent| {
            format!(
                "unfiltered ~{} · next request ~{} ({percent:.0}%)",
                human_tokens(snapshot.estimated_unfiltered_tokens),
                human_tokens(snapshot.estimated_next_request_tokens)
            )
        },
    );
    lines.push(Line::styled(
        format!("  {next}"),
        Style::default().fg(MUTED),
    ));

    let category = |wanted| {
        snapshot
            .categories
            .iter()
            .find(|estimate| estimate.category == wanted)
            .map_or(0, |estimate| estimate.estimated_tokens)
    };
    lines.push(Line::styled(
        format!(
            "  user ~{} · assistant ~{} · tool results ~{} · other ~{}",
            human_tokens(category(ContextCategory::User)),
            human_tokens(category(ContextCategory::Assistant)),
            human_tokens(category(ContextCategory::ToolResult)),
            human_tokens(category(ContextCategory::Other))
        ),
        Style::default().fg(MUTED),
    ));

    if let Some(overhead) = snapshot.estimated_system_tool_other_tokens {
        lines.push(Line::styled(
            format!(
                "  aligned system/tool-schema/other overhead ~{}",
                human_tokens(overhead)
            ),
            Style::default().fg(MUTED),
        ));
    }

    let policy = if snapshot.policy.enabled { "on" } else { "off" };
    lines.push(Line::styled(
        format!(
            "  pruning {policy} · this request ~{} · still eligible ~{} · cumulative avoided ~{}",
            human_tokens(snapshot.estimated_pruned_this_request_tokens),
            human_tokens(snapshot.estimated_currently_prunable_tokens),
            human_tokens(snapshot.estimated_cumulative_avoided_tokens)
        ),
        Style::default().fg(MUTED),
    ));

    let pruning = snapshot
        .pruning
        .iter()
        .filter(|estimate| estimate.count > 0)
        .map(|estimate| {
            let reason = match estimate.reason {
                PruneReason::HardLimit => "hard",
                PruneReason::HistoryBudget => "history",
                PruneReason::Superseded => "superseded",
            };
            format!(
                "{reason} {} / ~{}",
                estimate.count,
                human_tokens(estimate.estimated_tokens_saved)
            )
        })
        .collect::<Vec<_>>();
    if !pruning.is_empty() {
        lines.push(Line::styled(
            format!("  {}", pruning.join(" · ")),
            Style::default().fg(MUTED),
        ));
    }

    if !snapshot.top_consumers.is_empty() {
        lines.push(Line::styled(
            "  Top consumers",
            Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
        ));
        for consumer in snapshot.top_consumers.iter().take(5) {
            lines.push(Line::styled(
                format!(
                    "    {} · ~{}",
                    truncate_to_width(&consumer.label, 72),
                    human_tokens(consumer.estimated_tokens)
                ),
                Style::default().fg(MUTED),
            ));
        }
    }

    if snapshot.compaction_count > 0 {
        lines.push(Line::styled(
            format!(
                "  compactions {} · epoch {}",
                snapshot.compaction_count, snapshot.epoch
            ),
            Style::default().fg(MUTED),
        ));
        if let Some(record) = snapshot.recent_compactions.last() {
            let usage = record.estimated_tokens_after.map_or_else(
                || format!("before {}", human_tokens(record.tokens_before)),
                |after| {
                    format!(
                        "{} → {}",
                        human_tokens(record.tokens_before),
                        human_tokens(after)
                    )
                },
            );
            lines.push(Line::styled(
                format!(
                    "    latest {} · {} · files {}",
                    record.reason,
                    usage,
                    record.file_count()
                ),
                Style::default().fg(MUTED),
            ));
        }
    }

    let suggestion = if !snapshot.policy.enabled {
        "Context pruning is disabled by NABLA_CONTEXT_PRUNING."
    } else if snapshot.usage_state == ContextUsageState::Recalculating {
        "Usage will become exact after the next model response."
    } else if next_percent.is_some_and(|percent| percent >= 80.0) {
        "Consider /compact with a focus when the current detail is no longer needed."
    } else if snapshot.estimated_currently_prunable_tokens
        >= snapshot.policy.minimum_batch_savings_tokens
    {
        "The next model request can form another sticky pruning batch."
    } else {
        "Recent evidence is protected; older pruning decisions remain cache-stable."
    };
    lines.push(Line::styled(
        format!("  Suggestion: {suggestion}"),
        Style::default().fg(CYAN),
    ));
}

fn push_prefixed(
    lines: &mut Vec<Line<'static>>,
    prefix: &str,
    content: &str,
    prefix_style: Style,
    content_style: Style,
) {
    let mut content_lines = content.lines();
    let first = content_lines.next().unwrap_or_default();
    lines.push(Line::from(vec![
        Span::styled(prefix.to_owned(), prefix_style),
        Span::styled(first.to_owned(), content_style),
    ]));
    for line in content_lines {
        lines.push(Line::styled(format!("  {line}"), content_style));
    }
}

fn push_markdown_width(
    lines: &mut Vec<Line<'static>>,
    first_prefix: &str,
    content: &str,
    prefix_style: Style,
    width: usize,
) {
    push_markdown_with_state(
        lines,
        first_prefix,
        content,
        prefix_style,
        &mut MarkdownState::default(),
        width.saturating_sub(2).max(1),
        true,
    );
}

fn push_markdown_with_state(
    lines: &mut Vec<Line<'static>>,
    first_prefix: &str,
    content: &str,
    prefix_style: Style,
    state: &mut MarkdownState,
    width: usize,
    finish: bool,
) {
    let mut rows = content.split('\n');
    if let Some(first) = rows.next() {
        push_markdown_block_row(lines, first_prefix, prefix_style, first, state, width);
    }
    for row in rows {
        push_markdown_block_row(lines, "  ", prefix_style, row, state, width);
    }
    if finish {
        finish_markdown_blocks(lines, state, width);
    }
}

fn push_markdown_block_row(
    lines: &mut Vec<Line<'static>>,
    prefix: &str,
    prefix_style: Style,
    row: &str,
    state: &mut MarkdownState,
    width: usize,
) {
    if state.fence.is_some() || fence_marker(row.trim_start()).is_some() {
        finish_markdown_blocks(lines, state, width);
        lines.push(markdown_row(prefix, prefix_style, row, state));
        return;
    }

    if let Some(table) = state.table.as_mut() {
        if let Some(cells) = split_table_cells(row)
            && cells.len() == table.headers.len()
        {
            table.rows.push(cells);
            return;
        }
        if let Some(table) = state.table.take() {
            render_markdown_table(lines, table, width);
        }
    }

    if let Some(pending) = state.pending_row.take() {
        if let Some(headers) = split_table_cells(&pending.row)
            && let Some(alignments) = parse_table_delimiter(row)
            && headers.len() == alignments.len()
        {
            state.table = Some(MarkdownTable {
                prefix: pending.prefix,
                prefix_style: pending.prefix_style,
                headers,
                alignments,
                rows: Vec::new(),
            });
            return;
        }
        lines.push(markdown_row(
            &pending.prefix,
            pending.prefix_style,
            &pending.row,
            state,
        ));
    }

    if is_potential_table_header(row) {
        state.pending_row = Some(PendingMarkdownRow {
            prefix: prefix.to_owned(),
            prefix_style,
            row: row.to_owned(),
        });
    } else {
        lines.push(markdown_row(prefix, prefix_style, row, state));
    }
}

fn finish_markdown_blocks(lines: &mut Vec<Line<'static>>, state: &mut MarkdownState, width: usize) {
    if let Some(table) = state.table.take() {
        render_markdown_table(lines, table, width);
    }
    if let Some(pending) = state.pending_row.take() {
        lines.push(markdown_row(
            &pending.prefix,
            pending.prefix_style,
            &pending.row,
            state,
        ));
    }
}

fn is_potential_table_header(row: &str) -> bool {
    split_table_cells(row).is_some_and(|cells| cells.len() >= 2)
}

fn split_table_cells(row: &str) -> Option<Vec<String>> {
    let trimmed = row.trim();
    if !trimmed.contains('|') {
        return None;
    }

    let mut cells = vec![String::new()];
    let mut offset = 0;
    let mut code_delimiter = None;
    while offset < trimmed.len() {
        let remaining = &trimmed[offset..];
        let character = remaining.chars().next()?;
        if character == '`' {
            let delimiter_len = remaining.bytes().take_while(|byte| *byte == b'`').count();
            cells.last_mut()?.push_str(&remaining[..delimiter_len]);
            match code_delimiter {
                Some(open_len) if open_len == delimiter_len => code_delimiter = None,
                None => code_delimiter = Some(delimiter_len),
                _ => {}
            }
            offset += delimiter_len;
            continue;
        }
        if character == '\\' && code_delimiter.is_none() {
            let slash_len = character.len_utf8();
            let after_slash = &remaining[slash_len..];
            if let Some(escaped) = after_slash.chars().next() {
                if escaped != '|' {
                    cells.last_mut()?.push('\\');
                }
                cells.last_mut()?.push(escaped);
                offset += slash_len + escaped.len_utf8();
                continue;
            }
        }
        if character == '|' && code_delimiter.is_none() {
            cells.push(String::new());
        } else {
            cells.last_mut()?.push(character);
        }
        offset += character.len_utf8();
    }

    if trimmed.starts_with('|') && cells.first().is_some_and(|cell| cell.trim().is_empty()) {
        cells.remove(0);
    }
    if trimmed.ends_with('|') && cells.last().is_some_and(|cell| cell.trim().is_empty()) {
        cells.pop();
    }
    let cells = cells
        .into_iter()
        .map(|cell| cell.trim().to_owned())
        .collect::<Vec<_>>();
    (cells.len() >= 2).then_some(cells)
}

fn parse_table_delimiter(row: &str) -> Option<Vec<TableAlignment>> {
    let cells = split_table_cells(row)?;
    cells
        .into_iter()
        .map(|cell| {
            let trimmed = cell.trim();
            let left = trimmed.starts_with(':');
            let right = trimmed.ends_with(':');
            let dashes = trimmed.trim_matches(':');
            if dashes.len() < 3 || !dashes.chars().all(|character| character == '-') {
                return None;
            }
            Some(match (left, right) {
                (true, true) => TableAlignment::Center,
                (false, true) => TableAlignment::Right,
                _ => TableAlignment::Left,
            })
        })
        .collect()
}

fn render_markdown_table(lines: &mut Vec<Line<'static>>, table: MarkdownTable, width: usize) {
    let column_count = table.headers.len();
    if column_count == 0 {
        return;
    }
    let prefix_width = UnicodeWidthStr::width(table.prefix.as_str());
    let border_width = column_count.saturating_mul(3).saturating_add(1);
    let available_cells = width.saturating_sub(prefix_width + border_width);
    if available_cells < column_count.saturating_mul(3) {
        render_stacked_table(lines, table);
        return;
    }

    let mut widths = (0..column_count)
        .map(|index| {
            std::iter::once(table.headers[index].as_str())
                .chain(
                    table
                        .rows
                        .iter()
                        .filter_map(|row| row.get(index).map(String::as_str)),
                )
                .map(markdown_display_width)
                .max()
                .unwrap_or(3)
                .clamp(3, 32)
        })
        .collect::<Vec<_>>();
    while widths.iter().sum::<usize>() > available_cells {
        let Some((index, _)) = widths
            .iter()
            .enumerate()
            .filter(|(_, width)| **width > 3)
            .max_by_key(|(_, width)| **width)
        else {
            break;
        };
        widths[index] -= 1;
    }

    let mut first = true;
    push_table_border(lines, &table, &widths, &mut first, ('┌', '┬', '┐', '─'));
    push_table_row(lines, &table, &table.headers, &widths, &mut first, true);
    push_table_border(lines, &table, &widths, &mut first, ('├', '┼', '┤', '─'));
    for row in &table.rows {
        push_table_row(lines, &table, row, &widths, &mut first, false);
    }
    push_table_border(lines, &table, &widths, &mut first, ('└', '┴', '┘', '─'));
}

fn push_table_border(
    lines: &mut Vec<Line<'static>>,
    table: &MarkdownTable,
    widths: &[usize],
    first: &mut bool,
    glyphs: (char, char, char, char),
) {
    let (left, joint, right, fill) = glyphs;
    let mut border = String::new();
    border.push(left);
    for (index, width) in widths.iter().enumerate() {
        border.push_str(&fill.to_string().repeat(width.saturating_add(2)));
        border.push(if index + 1 == widths.len() {
            right
        } else {
            joint
        });
    }
    lines.push(Line::from(vec![
        Span::styled(
            if *first {
                table.prefix.clone()
            } else {
                "  ".to_owned()
            },
            table.prefix_style,
        ),
        Span::styled(border, Style::default().fg(BORDER)),
    ]));
    *first = false;
}

fn push_table_row(
    lines: &mut Vec<Line<'static>>,
    table: &MarkdownTable,
    cells: &[String],
    widths: &[usize],
    first: &mut bool,
    header: bool,
) {
    let base_style = if header {
        Style::default().fg(CYAN).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(TEXT)
    };
    let wrapped = cells
        .iter()
        .zip(widths)
        .map(|(cell, width)| wrap_styled_cell(inline_markdown(cell, base_style), *width))
        .collect::<Vec<_>>();
    let row_height = wrapped.iter().map(Vec::len).max().unwrap_or(1);
    for visual_row in 0..row_height {
        let mut spans = vec![
            Span::styled(
                if *first {
                    table.prefix.clone()
                } else {
                    "  ".to_owned()
                },
                table.prefix_style,
            ),
            Span::styled("│ ", Style::default().fg(BORDER)),
        ];
        for (index, width) in widths.iter().enumerate() {
            let value = wrapped[index].get(visual_row).cloned().unwrap_or_default();
            let content_width = spans_display_width(&value);
            let padding = width.saturating_sub(content_width);
            let (left, right) = alignment_padding(padding, table.alignments[index]);
            spans.push(Span::styled(" ".repeat(left), base_style));
            spans.extend(value);
            spans.push(Span::styled(" ".repeat(right), base_style));
            spans.push(Span::styled(" │ ", Style::default().fg(BORDER)));
        }
        lines.push(Line::from(spans));
        *first = false;
    }
}

fn markdown_display_width(value: &str) -> usize {
    spans_display_width(&inline_markdown(value, Style::default()))
}

fn spans_display_width(spans: &[Span<'_>]) -> usize {
    spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum()
}

fn wrap_styled_cell(spans: Vec<Span<'static>>, width: usize) -> Vec<Vec<Span<'static>>> {
    if spans.is_empty() {
        return vec![Vec::new()];
    }
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut row_width = 0;
    for span in spans {
        for character in span.content.chars() {
            let character_width = character.width().unwrap_or(0);
            if row_width > 0 && row_width + character_width > width {
                rows.push(std::mem::take(&mut row));
                row_width = 0;
            }
            row.push(Span::styled(character.to_string(), span.style));
            row_width += character_width;
        }
    }
    if !row.is_empty() {
        rows.push(row);
    }
    if rows.is_empty() {
        rows.push(Vec::new());
    }
    rows
}

fn alignment_padding(padding: usize, alignment: TableAlignment) -> (usize, usize) {
    match alignment {
        TableAlignment::Left => (0, padding),
        TableAlignment::Center => (padding / 2, padding - padding / 2),
        TableAlignment::Right => (padding, 0),
    }
}

fn render_stacked_table(lines: &mut Vec<Line<'static>>, table: MarkdownTable) {
    let rows = if table.rows.is_empty() {
        vec![vec![String::new(); table.headers.len()]]
    } else {
        table.rows.clone()
    };
    let mut first = true;
    for (row_index, row) in rows.iter().enumerate() {
        if row_index > 0 {
            lines.push(Line::default());
        }
        for (index, header) in table.headers.iter().enumerate() {
            let mut spans = vec![Span::styled(
                if first {
                    table.prefix.clone()
                } else {
                    "  ".to_owned()
                },
                table.prefix_style,
            )];
            spans.extend(inline_markdown(
                header,
                Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(
                ": ",
                Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
            ));
            spans.extend(inline_markdown(
                row.get(index).map_or("", String::as_str),
                Style::default().fg(TEXT),
            ));
            lines.push(Line::from(spans));
            first = false;
        }
    }
}

fn markdown_row(
    prefix: &str,
    prefix_style: Style,
    row: &str,
    state: &mut MarkdownState,
) -> Line<'static> {
    let trimmed = row.trim_start();
    let indent = &row[..row.len().saturating_sub(trimmed.len())];
    if let Some(marker) = fence_marker(trimmed) {
        let language = trimmed[3..].trim();
        let closing = state.fence == Some(marker);
        state.fence = if closing { None } else { Some(marker) };
        return Line::from(vec![
            Span::styled(prefix.to_owned(), prefix_style),
            Span::styled(
                if closing {
                    format!("{indent}└─")
                } else if language.is_empty() {
                    format!("{indent}┌─ code")
                } else {
                    format!("{indent}┌─ {language}")
                },
                Style::default().fg(VIOLET).add_modifier(Modifier::BOLD),
            ),
        ])
        .style(Style::default().bg(THEME.surface0));
    }

    if state.fence.is_some() {
        return Line::from(vec![
            Span::styled(prefix.to_owned(), prefix_style),
            Span::styled(
                format!("{indent}│ "),
                Style::default().fg(VIOLET).bg(THEME.surface0),
            ),
            Span::styled(
                trimmed.to_owned(),
                Style::default().fg(TEXT).bg(THEME.surface0),
            ),
        ])
        .style(Style::default().bg(THEME.surface0));
    }

    let mut spans = vec![Span::styled(prefix.to_owned(), prefix_style)];
    if trimmed.is_empty() {
        return Line::from(spans);
    }

    if let Some((level, content)) = heading(trimmed) {
        let marker = match level {
            1 => "◆ ",
            2 => "◇ ",
            _ => "▸ ",
        };
        spans.push(Span::styled(
            format!("{indent}{marker}"),
            Style::default().fg(VIOLET).add_modifier(Modifier::BOLD),
        ));
        spans.extend(inline_markdown(
            content,
            Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
        ));
        return Line::from(spans);
    }

    if is_horizontal_rule(trimmed) {
        spans.push(Span::styled(
            format!("{indent}{}", "─".repeat(32)),
            Style::default().fg(BORDER),
        ));
        return Line::from(spans);
    }

    if let Some(content) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("+ "))
    {
        spans.push(Span::styled(
            format!("{indent}• "),
            Style::default().fg(VIOLET).add_modifier(Modifier::BOLD),
        ));
        spans.extend(inline_markdown(content, Style::default().fg(TEXT)));
        return Line::from(spans);
    }

    if let Some((number, content)) = ordered_list_item(trimmed) {
        spans.push(Span::styled(
            format!("{indent}{number}. "),
            Style::default().fg(VIOLET).add_modifier(Modifier::BOLD),
        ));
        spans.extend(inline_markdown(content, Style::default().fg(TEXT)));
        return Line::from(spans);
    }

    if let Some(content) = trimmed.strip_prefix("> ") {
        spans.push(Span::styled(
            format!("{indent}│ "),
            Style::default().fg(VIOLET),
        ));
        spans.extend(inline_markdown(
            content,
            Style::default().fg(MUTED).add_modifier(Modifier::ITALIC),
        ));
        return Line::from(spans);
    }

    spans.push(Span::raw(indent.to_owned()));
    spans.extend(inline_markdown(trimmed, Style::default().fg(TEXT)));
    Line::from(spans)
}

fn fence_marker(value: &str) -> Option<char> {
    if value.starts_with("```") {
        Some('`')
    } else if value.starts_with("~~~") {
        Some('~')
    } else {
        None
    }
}

fn heading(value: &str) -> Option<(usize, &str)> {
    let level = value
        .chars()
        .take_while(|character| *character == '#')
        .count();
    if !(1..=6).contains(&level) {
        return None;
    }
    value
        .get(level..)?
        .strip_prefix(' ')
        .map(|body| (level, body))
}

fn ordered_list_item(value: &str) -> Option<(&str, &str)> {
    let digits = value
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .count();
    if digits == 0 {
        return None;
    }
    let suffix = value.get(digits..)?;
    suffix
        .strip_prefix(". ")
        .map(|content| (&value[..digits], content))
}

fn is_horizontal_rule(value: &str) -> bool {
    let compact = value
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    compact.len() >= 3
        && compact.chars().next().is_some_and(|first| {
            matches!(first, '-' | '*' | '_') && compact.chars().all(|character| character == first)
        })
}

fn inline_markdown(value: &str, base: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut offset = 0;
    while offset < value.len() {
        let remaining = &value[offset..];
        if let Some(after_escape) = remaining.strip_prefix('\\')
            && let Some(character) = after_escape.chars().next()
        {
            spans.push(Span::styled(character.to_string(), base));
            offset += 1 + character.len_utf8();
            continue;
        }
        if remaining.starts_with('`')
            && let Some((content, consumed)) = parse_code_span(remaining)
        {
            spans.push(Span::styled(content, base.fg(ORANGE)));
            offset += consumed;
            continue;
        }
        let mut parsed_bold = false;
        for marker in ["**", "__"] {
            if let Some(after_open) = remaining.strip_prefix(marker)
                && let Some(end) = after_open.find(marker)
            {
                spans.push(Span::styled(
                    after_open[..end].to_owned(),
                    base.add_modifier(Modifier::BOLD),
                ));
                offset += end + marker.len() * 2;
                parsed_bold = true;
                break;
            }
        }
        if parsed_bold {
            continue;
        }
        if let Some(after_label) = remaining.strip_prefix('[')
            && let Some(label_end) = after_label.find("](")
        {
            let after_url_open = &after_label[label_end + 2..];
            if let Some(url_end) = after_url_open.find(')') {
                let link_style = base.fg(CYAN).add_modifier(Modifier::UNDERLINED);
                spans.push(Span::styled(
                    after_label[..label_end].to_owned(),
                    link_style,
                ));
                spans.push(Span::styled(" (", base));
                spans.push(Span::styled(
                    after_url_open[..url_end].to_owned(),
                    link_style,
                ));
                spans.push(Span::styled(")", base));
                offset += label_end + url_end + 4;
                continue;
            }
        }
        if (remaining.starts_with('*') || remaining.starts_with('_'))
            && let Some(marker) = remaining.chars().next()
            && let Some(end) = remaining[marker.len_utf8()..].find(marker)
        {
            let body_start = marker.len_utf8();
            spans.push(Span::styled(
                remaining[body_start..body_start + end].to_owned(),
                base.add_modifier(Modifier::ITALIC),
            ));
            offset += body_start + end + marker.len_utf8();
            continue;
        }

        let plain_end = remaining
            .char_indices()
            .skip(1)
            .find_map(|(index, character)| {
                matches!(character, '\\' | '`' | '*' | '_' | '[').then_some(index)
            })
            .unwrap_or(remaining.len());
        spans.push(Span::styled(remaining[..plain_end].to_owned(), base));
        offset += plain_end;
    }
    spans
}

fn parse_code_span(value: &str) -> Option<(String, usize)> {
    let delimiter_len = value.bytes().take_while(|byte| *byte == b'`').count();
    if delimiter_len == 0 {
        return None;
    }
    let mut offset = delimiter_len;
    while offset < value.len() {
        let remaining = &value[offset..];
        let character = remaining.chars().next()?;
        if character != '`' {
            offset += character.len_utf8();
            continue;
        }
        let run_len = remaining.bytes().take_while(|byte| *byte == b'`').count();
        if run_len == delimiter_len {
            let raw = value[delimiter_len..offset].replace('\n', " ");
            let content = if raw.starts_with(' ')
                && raw.ends_with(' ')
                && raw.chars().any(|character| character != ' ')
            {
                raw[1..raw.len().saturating_sub(1)].to_owned()
            } else {
                raw
            };
            return Some((content, offset + run_len));
        }
        offset += run_len;
    }
    None
}

fn tool_request_summary(name: &str, args: &serde_json::Value) -> String {
    if args.is_null() || args.as_object().is_some_and(serde_json::Map::is_empty) {
        return String::new();
    }

    match name {
        "ask_user" => {
            let count = args["questions"].as_array().map_or(0, Vec::len);
            format!("{count} clarification question(s)")
        }
        "submit_plan" => args["title"]
            .as_str()
            .unwrap_or("submitted plan")
            .to_owned(),
        "read" => {
            let path = path_arg(args);
            let offset = args["offset"]
                .as_u64()
                .or_else(|| args["line"].as_u64())
                .or_else(|| args["start_line"].as_u64());
            let limit = args["limit"].as_u64();
            match (offset, limit) {
                (Some(offset), Some(limit)) => {
                    format!("{path} · lines {offset}–{}", offset.saturating_add(limit))
                }
                (Some(offset), None) => format!("{path} · from line {offset}"),
                _ => path,
            }
        }
        "grep" => {
            let pattern = args["pattern"]
                .as_str()
                .or_else(|| args["query"].as_str())
                .unwrap_or_default();
            format!("“{pattern}” in {}", path_arg(args))
        }
        "find" => {
            let pattern = args["pattern"]
                .as_str()
                .or_else(|| args["glob"].as_str())
                .unwrap_or_default();
            format!("{pattern} in {}", path_arg(args))
        }
        "ls" => path_arg(args),
        "bash" => {
            let command = args["command"].as_str().unwrap_or_default();
            let first = command.lines().next().unwrap_or_default();
            if command.lines().count() > 1 {
                format!("{first}  …")
            } else {
                first.to_owned()
            }
        }
        "edit" => {
            let path = args["path"].as_str().unwrap_or("<unknown path>");
            let edits = args["edits"].as_array();
            let edit_count = edits.map_or(1, Vec::len);
            let (removed, added) = if let Some(edits) = edits {
                edits.iter().fold((0, 0), |(removed, added), edit| {
                    (
                        removed + line_count(edit["oldText"].as_str().unwrap_or_default()),
                        added + line_count(edit["newText"].as_str().unwrap_or_default()),
                    )
                })
            } else {
                (
                    line_count(args["oldText"].as_str().unwrap_or_default()),
                    line_count(args["newText"].as_str().unwrap_or_default()),
                )
            };
            format!("{path} · {edit_count} edit(s) · -{removed}/+{added} lines")
        }
        "write" => {
            let path = args["path"].as_str().unwrap_or("<unknown path>");
            let content = args["content"].as_str().unwrap_or_default();
            format!(
                "{path} · {} lines · {}",
                line_count(content),
                human_bytes(content.len())
            )
        }
        _ => truncate_to_width(&serde_json::to_string(args).unwrap_or_default(), 240),
    }
}

fn push_tool_output(lines: &mut Vec<Line<'static>>, tool: &crate::state::ToolExecution) {
    if tool.output.is_empty() {
        return;
    }

    let max_lines = match tool.status {
        ToolStatus::Failed | ToolStatus::Denied => 12,
        _ => 8,
    };
    let (preview, hidden) = bounded_output_preview(&tool.output, max_lines);
    push_prefixed(
        lines,
        "  └ ",
        &preview,
        Style::default().fg(MUTED),
        if matches!(tool.status, ToolStatus::Failed | ToolStatus::Denied) {
            Style::default().fg(RED)
        } else {
            Style::default().fg(TEXT)
        },
    );
    if hidden > 0 {
        lines.push(Line::styled(
            format!("    … {hidden} lines collapsed"),
            Style::default().fg(MUTED).add_modifier(Modifier::ITALIC),
        ));
    }
}

fn tool_status_visual(status: ToolStatus) -> (&'static str, &'static str, Style) {
    match status {
        ToolStatus::WaitingApproval => ("?", "approval required", Style::default().fg(ORANGE)),
        ToolStatus::Running => ("↳", "running", Style::default().fg(ORANGE)),
        ToolStatus::Succeeded => ("✓", "done", Style::default().fg(GREEN)),
        ToolStatus::Failed => ("✕", "failed", Style::default().fg(RED)),
        ToolStatus::Denied => ("✕", "denied", Style::default().fg(RED)),
    }
}

fn bounded_output_preview(output: &str, max_lines: usize) -> (String, usize) {
    let rows = output.lines().collect::<Vec<_>>();
    if rows.len() <= max_lines {
        return (output.to_owned(), 0);
    }

    let tail = 2.min(max_lines.saturating_sub(1));
    let head = max_lines.saturating_sub(tail);
    let mut preview = rows[..head].to_vec();
    preview.extend_from_slice(&rows[rows.len() - tail..]);
    (preview.join("\n"), rows.len().saturating_sub(max_lines))
}

fn path_arg(args: &serde_json::Value) -> String {
    args["path"]
        .as_str()
        .or_else(|| args["file"].as_str())
        .or_else(|| args["directory"].as_str())
        .unwrap_or(".")
        .to_owned()
}

fn line_count(value: &str) -> usize {
    if value.is_empty() {
        0
    } else {
        value.lines().count()
    }
}

fn human_bytes(bytes: usize) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    if bytes < 1024 * 1024 {
        return format!("{:.1} KiB", bytes as f64 / 1024.0);
    }
    format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
}

fn human_tokens(tokens: u64) -> String {
    if tokens < 1_000 {
        return tokens.to_string();
    }
    if tokens < 1_000_000 {
        let value = tokens as f64 / 1_000.0;
        return if tokens < 10_000 {
            format!("{value:.1}k")
        } else {
            format!("{value:.0}k")
        };
    }
    format!("{:.1}m", tokens as f64 / 1_000_000.0)
}

fn is_complete(item: &TranscriptItem) -> bool {
    match item {
        TranscriptItem::User(message) => message.status != UserMessageStatus::Pending,
        TranscriptItem::Assistant(message) => message.complete,
        TranscriptItem::Tool(tool) => !matches!(
            tool.status,
            ToolStatus::WaitingApproval | ToolStatus::Running
        ),
        TranscriptItem::Plan(_)
        | TranscriptItem::Context(_)
        | TranscriptItem::Resources(_)
        | TranscriptItem::Goal(_)
        | TranscriptItem::Goals(_)
        | TranscriptItem::Agents(_)
        | TranscriptItem::Subagent(_)
        | TranscriptItem::Compaction(_)
        | TranscriptItem::BranchSummary(_)
        | TranscriptItem::SessionBoundary { .. }
        | TranscriptItem::Notice(_)
        | TranscriptItem::Error(_) => true,
    }
}

fn visual_height(lines: &[Line<'_>], width: usize) -> u16 {
    lines
        .iter()
        .map(|line| {
            let line_width = line
                .spans
                .iter()
                .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
                .sum::<usize>()
                .max(1);
            line_width.div_ceil(width.max(1)).max(1)
        })
        .sum::<usize>()
        .min(u16::MAX as usize) as u16
}

fn input_accent(state: &AppState) -> Color {
    if state.connection_state == ConnectionState::Disconnected {
        return RED;
    }
    match state.run_state {
        RunState::Idle if state.plan_mode_active => THEME.user,
        RunState::Idle => THEME.primary,
        RunState::Submitting
        | RunState::Running
        | RunState::Compacting
        | RunState::Authenticating
        | RunState::SwitchingSession
        | RunState::NavigatingTree
        | RunState::SummarizingBranch
        | RunState::Aborting => ORANGE,
        RunState::AuthRequired | RunState::Error => RED,
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};
    use serde_json::json;

    use super::*;
    use crate::{
        app::App,
        command::DiscoveredCommand,
        event::{AppEvent, RuntimeEvent},
        rpc::PiState,
        state::{
            ApprovalState, AssistantMessage, AuthChoice, AuthFlowState, AuthPromptState,
            CompactionRecord, ContextCategory, ContextCategoryEstimate, ContextConsumer,
            ContextUsageState, PlanArtifact, PlanQuestion, PlanReviewState, PlanStatus,
            QuestionFlowState, QuestionOption, SessionBrowserState, SessionScope, SessionSortMode,
            SessionSummary, ToolExecution, TreeBrowserState, TreeFilterMode, TreeItem, TreePhase,
            UserMessage,
        },
    };

    fn session() -> PiState {
        PiState {
            model: Some(json!({"provider": "test", "name": "fake"})),
            thinking_level: "off".to_owned(),
            is_streaming: false,
            is_compacting: false,
            steering_mode: "one-at-a-time".to_owned(),
            follow_up_mode: "one-at-a-time".to_owned(),
            session_file: None,
            session_id: "session-1".to_owned(),
            session_name: None,
            auto_compaction_enabled: true,
            message_count: 2,
            pending_message_count: 0,
        }
    }

    fn rendered(state: &AppState, presenter: &TranscriptPresenter) -> (String, bool) {
        rendered_width(state, presenter, 80)
    }

    fn rendered_width(
        state: &AppState,
        presenter: &TranscriptPresenter,
        width: u16,
    ) -> (String, bool) {
        rendered_size(state, presenter, width, inline_viewport_height(24))
    }

    fn rendered_size(
        state: &AppState,
        presenter: &TranscriptPresenter,
        width: u16,
        height: u16,
    ) -> (String, bool) {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        let metrics =
            measure_layout_request(state, presenter, width, height).resolve_layout(height);
        terminal
            .draw(|frame| {
                let _ = render(frame, state, presenter, metrics);
            })
            .unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        (text, terminal.backend().cursor_visible())
    }

    #[test]
    fn adaptive_inline_viewport_uses_two_thirds_with_bounds() {
        assert_eq!(inline_viewport_height(0), 0);
        assert_eq!(inline_viewport_height(10), 10);
        assert_eq!(inline_viewport_height(24), 16);
        assert_eq!(inline_viewport_height(30), 20);
        assert_eq!(inline_viewport_height(48), 32);
        assert_eq!(inline_viewport_height(100), 32);
    }

    #[test]
    fn main_view_layout_keeps_the_composer_at_the_bottom() {
        assert_eq!(
            main_view_layout(16, 0, 0, 3, 1),
            MainViewLayout {
                output_height: 12,
                auxiliary_height: 0,
            }
        );
        assert_eq!(
            main_view_layout(16, 12, 8, 3, 1),
            MainViewLayout {
                output_height: 4,
                auxiliary_height: 8,
            }
        );
        assert_eq!(
            main_view_layout(8, 4, 8, 3, 1),
            MainViewLayout {
                output_height: 1,
                auxiliary_height: 3,
            }
        );
    }

    #[test]
    fn measured_layout_grows_multiline_composer_and_degrades_footer_first() {
        let mut state = AppState::new(session());
        state.editor.insert_text("one\ntwo\nthree\nfour");
        let presenter = TranscriptPresenter::default();

        let roomy = measure_layout_request(&state, &presenter, 80, 24).resolve_layout(24);
        assert_eq!(roomy.composer_height, 6);
        assert_eq!(roomy.footer_height, 1);

        let short = measure_layout_request(&state, &presenter, 80, 8).resolve_layout(8);
        assert_eq!(short.composer_height, 5);
        assert_eq!(short.footer_height, 0);
        assert!(short.desired_height <= 8);
    }

    #[test]
    fn footer_visibility_uses_physical_terminal_height_not_inline_frame_height() {
        let state = AppState::new(session());
        let presenter = TranscriptPresenter::default();
        let metrics = measure_layout_request(&state, &presenter, 80, 24).resolve_layout(4);
        assert_eq!(metrics.footer_height, 1);
        assert_eq!(metrics.desired_height, 4);

        let backend = TestBackend::new(80, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let _ = render(frame, &state, &presenter, metrics);
            })
            .unwrap();
        let footer = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .skip(80 * 3)
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(footer.contains("idle"));
    }

    fn layout_request(
        desired_height: u16,
        terminal_rows: u16,
        surface: LayoutSurface,
    ) -> UiLayoutRequest {
        UiLayoutRequest {
            terminal_columns: 80,
            terminal_rows,
            desired_height,
            active_output_height: desired_height.saturating_sub(4),
            requested_auxiliary_height: 0,
            composer_height: 3,
            footer_height: 1,
            surface,
        }
    }

    #[test]
    fn main_viewport_uses_each_requested_height_without_a_busy_floor() {
        let heights = [6, 10, 5].map(|height| {
            layout_request(height, 24, LayoutSurface::Main)
                .resolve_layout(height)
                .desired_height
        });
        assert_eq!(heights, [6, 10, 5]);
    }

    #[test]
    fn full_and_inline_surfaces_resolve_independently() {
        let inline = layout_request(6, 24, LayoutSurface::Main).resolve_layout(6);
        let full = layout_request(24, 24, LayoutSurface::Full).resolve_layout(24);
        assert_eq!(inline.desired_height, 6);
        assert_eq!(full.desired_height, 24);
    }

    #[test]
    fn short_choices_stay_inline_while_full_surfaces_use_the_alternate_screen() {
        let mut state = AppState::new(session());
        state.question = Some(QuestionFlowState {
            request_id: "question-1".to_owned(),
            questions: vec![PlanQuestion {
                id: "scope".to_owned(),
                prompt: "Which scope?".to_owned(),
                options: vec![QuestionOption {
                    id: "small".to_owned(),
                    label: "Small".to_owned(),
                    description: None,
                }],
            }],
            current: 0,
            selected: 0,
            custom_answer: false,
            editor: EditorState::default(),
            answers: Vec::new(),
            replying: false,
        });
        assert!(!uses_fullscreen_surface(&state));

        state.question = None;
        state.auth_state = AuthState::LoadingProviders;
        assert!(uses_fullscreen_surface(&state));
    }

    #[test]
    fn transcript_viewer_projects_only_a_bounded_large_session_window() {
        let mut state = AppState::new(session());
        state
            .transcript
            .extend((0..10_000).map(|index| TranscriptItem::Notice(format!("history {index}"))));
        let viewer = TranscriptViewerState::new(TranscriptViewMode::Normal, &state.transcript);
        let lines = transcript_view_lines(&state, &viewer, 80);

        assert!(lines.len() <= 512);
        assert!(
            line_text(lines.into_iter().map(|line| line.line).collect())
                .join("\n")
                .contains("history 9999")
        );
    }

    #[test]
    fn context_footer_distinguishes_actual_estimated_and_recalculating_usage() {
        let mut state = AppState::new(session());
        state.context.context_window = Some(100_000);
        state.context.usage_state = ContextUsageState::Actual;
        state.context.actual_tokens = Some(47_000);
        state.context.actual_percent = Some(47.0);
        let (actual, _) = rendered_width(&state, &TranscriptPresenter::default(), 180);
        assert!(actual.contains("ctx 47%"));

        state.context.usage_state = ContextUsageState::Estimated;
        state.context.actual_tokens = None;
        state.context.actual_percent = None;
        state.context.estimated_next_request_tokens = 47_000;
        let (estimated, _) = rendered_width(&state, &TranscriptPresenter::default(), 180);
        assert!(estimated.contains("ctx ~47%"));

        state.context.usage_state = ContextUsageState::Recalculating;
        let (unknown, _) = rendered_width(&state, &TranscriptPresenter::default(), 180);
        assert!(unknown.contains("ctx …"));
    }

    #[test]
    fn narrow_footer_hides_context_before_the_running_state() {
        let mut state = AppState::new(session());
        state.run_state = RunState::Compacting;
        state.context.usage_state = ContextUsageState::Actual;
        state.context.actual_percent = Some(92.0);

        let (text, _) = rendered_width(&state, &TranscriptPresenter::default(), 42);

        assert!(text.contains("compacting"));
        assert!(!text.contains("ctx 92%"));
    }

    #[test]
    fn session_browser_renders_scope_threading_and_missing_cwd_confirmation() {
        let mut state = AppState::new(session());
        state.session_browser = Some(SessionBrowserState {
            browser_id: Some("browser-1".to_owned()),
            current_cwd: "/workspace/current".to_owned(),
            scope: SessionScope::All,
            sort_mode: SessionSortMode::Threaded,
            named_only: true,
            show_path: true,
            query: EditorState::default(),
            sessions: vec![SessionSummary {
                path: "/sessions/old.jsonl".to_owned(),
                id: "old".to_owned(),
                cwd: "/workspace/old".to_owned(),
                cwd_available: false,
                name: Some("Old investigation".to_owned()),
                parent_session_path: None,
                created_at: "2026-01-01T00:00:00.000Z".to_owned(),
                modified_at: "2026-01-02T00:00:00.000Z".to_owned(),
                message_count: 12,
                first_message: "diagnose parser".to_owned(),
                depth: 0,
                is_last: true,
                current: false,
            }],
            total: 1,
            next_offset: None,
            truncated: false,
            selected: 0,
            loading: false,
            loaded: None,
            generation: 0,
            switching: false,
            confirm_missing_cwd: None,
        });
        let missing_session = state
            .session_browser
            .as_ref()
            .and_then(|browser| browser.sessions.first().cloned());
        state
            .session_browser
            .as_mut()
            .expect("session browser")
            .confirm_missing_cwd = missing_session;

        let (text, _) = rendered_width(&state, &TranscriptPresenter::default(), 180);

        assert!(text.contains("Resume Session · all projects · threaded · named"));
        assert!(text.contains("Old investigation · 12 msgs · /workspace/old"));
        assert!(text.contains("/sessions/old.jsonl"));
        assert!(text.contains("Original cwd is unavailable"));
        assert!(text.contains("tab scope"));
    }

    #[test]
    fn adaptive_session_browser_shows_a_large_centered_candidate_window() {
        let mut state = AppState::new(session());
        state.session_browser = Some(SessionBrowserState {
            browser_id: Some("browser-1".to_owned()),
            current_cwd: "/workspace/current".to_owned(),
            scope: SessionScope::Current,
            sort_mode: SessionSortMode::Recent,
            named_only: false,
            show_path: false,
            query: EditorState::default(),
            sessions: (0..30)
                .map(|index| SessionSummary {
                    path: format!("/sessions/{index:02}.jsonl"),
                    id: format!("session-{index:02}"),
                    cwd: "/workspace/current".to_owned(),
                    cwd_available: true,
                    name: Some(format!("Candidate {index:02}")),
                    parent_session_path: None,
                    created_at: "2026-01-01T00:00:00.000Z".to_owned(),
                    modified_at: "2026-01-02T00:00:00.000Z".to_owned(),
                    message_count: 2,
                    first_message: format!("candidate {index:02}"),
                    depth: 0,
                    is_last: index == 29,
                    current: false,
                })
                .collect(),
            total: 30,
            next_offset: None,
            truncated: false,
            selected: 15,
            loading: false,
            loaded: None,
            generation: 0,
            switching: false,
            confirm_missing_cwd: None,
        });
        let height = inline_viewport_height(45);

        let (text, _) = rendered_size(&state, &TranscriptPresenter::default(), 120, height);

        assert_eq!(height, 30);
        assert!(text.contains("Candidate 04"));
        assert!(text.contains("Candidate 27"));
        assert!(text.contains("search"));
        assert!(text.contains("tab scope"));
    }

    #[test]
    fn adaptive_tree_browser_shows_a_large_centered_candidate_window() {
        let mut state = AppState::new(session());
        state.tree_browser = Some(TreeBrowserState {
            items: (0..30)
                .map(|index| TreeItem {
                    entry_id: format!("entry-{index:02}"),
                    parent_id: (index > 0).then(|| format!("entry-{:02}", index - 1)),
                    kind: "message".to_owned(),
                    role: Some("user".to_owned()),
                    preview: format!("user: Tree candidate {index:02}"),
                    label: None,
                    label_timestamp: None,
                    visual_depth: 0,
                    show_connector: false,
                    gutter_positions: Vec::new(),
                    is_last: index == 29,
                    is_active_path: true,
                    is_leaf: index == 29,
                    foldable: false,
                    folded: false,
                })
                .collect(),
            leaf_id: Some("entry-29".to_owned()),
            selected: 15,
            selected_entry_id: Some("entry-15".to_owned()),
            filter_mode: TreeFilterMode::Default,
            query: EditorState::default(),
            folded_entry_ids: Default::default(),
            show_label_timestamps: false,
            phase: TreePhase::Browse,
            loading: false,
            generation: 0,
        });
        let height = inline_viewport_height(45);

        let (text, _) = rendered_size(&state, &TranscriptPresenter::default(), 120, height);

        assert_eq!(height, 30);
        assert!(text.contains("Tree candidate 04"));
        assert!(text.contains("Tree candidate 27"));
        assert!(text.contains("search"));
        assert!(text.contains("ctrl+o filter"));
    }

    #[test]
    fn tree_browser_renders_active_path_labels_and_summary_choices() {
        let mut state = AppState::new(session());
        state.tree_browser = Some(TreeBrowserState {
            items: vec![
                TreeItem {
                    entry_id: "root".to_owned(),
                    parent_id: None,
                    kind: "message".to_owned(),
                    role: Some("user".to_owned()),
                    preview: "[checkpoint] user: implement tree".to_owned(),
                    label: Some("checkpoint".to_owned()),
                    label_timestamp: Some("2026-01-01T10:00:00.000Z".to_owned()),
                    visual_depth: 0,
                    show_connector: false,
                    gutter_positions: Vec::new(),
                    is_last: true,
                    is_active_path: true,
                    is_leaf: false,
                    foldable: true,
                    folded: false,
                },
                TreeItem {
                    entry_id: "leaf".to_owned(),
                    parent_id: Some("root".to_owned()),
                    kind: "message".to_owned(),
                    role: Some("assistant".to_owned()),
                    preview: "assistant: implemented".to_owned(),
                    label: None,
                    label_timestamp: None,
                    visual_depth: 1,
                    show_connector: true,
                    gutter_positions: Vec::new(),
                    is_last: true,
                    is_active_path: true,
                    is_leaf: true,
                    foldable: false,
                    folded: false,
                },
            ],
            leaf_id: Some("leaf".to_owned()),
            selected: 0,
            selected_entry_id: Some("root".to_owned()),
            filter_mode: TreeFilterMode::LabeledOnly,
            query: EditorState::default(),
            folded_entry_ids: Default::default(),
            show_label_timestamps: true,
            phase: TreePhase::ChooseSummary {
                entry_id: "root".to_owned(),
                selected: 1,
            },
            loading: false,
            generation: 0,
        });

        let (text, _) = rendered_width(&state, &TranscriptPresenter::default(), 180);

        assert!(text.contains("Session Tree · labeled-only"));
        assert!(text.contains("[checkpoint] user: implement tree"));
        assert!(text.contains("[2026-01-01T10:00:00.000Z]"));
        assert!(text.contains("[2 summarize]"));
        assert!(text.contains("ctrl+x copy"));
    }

    #[test]
    fn deep_tree_rows_pan_horizontally_to_keep_selected_content_visible() {
        let item = TreeItem {
            entry_id: "deep".to_owned(),
            parent_id: Some("parent".to_owned()),
            kind: "message".to_owned(),
            role: Some("toolResult".to_owned()),
            preview: "toolResult: 中文 evidence remains visible".to_owned(),
            label: None,
            label_timestamp: None,
            visual_depth: 40,
            show_connector: true,
            gutter_positions: (0..39).collect(),
            is_last: true,
            is_active_path: true,
            is_leaf: true,
            foldable: false,
            folded: false,
        };

        let rows = render_tree_rows(&[(0, &item)], 0, 72, false);
        let text = line_text(rows).join("\n");

        assert!(text.starts_with("› "));
        assert!(text.contains("toolResult: 中文 evidence"));
        assert!(UnicodeWidthStr::width(text.as_str()) <= 72);
    }

    #[test]
    fn restored_session_boundaries_and_branch_summaries_render_as_local_history() {
        assert_eq!(
            line_text(item_lines(&TranscriptItem::SessionBoundary {
                action: "resumed".to_owned(),
                label: "old work".to_owned(),
                cwd: "/workspace/old".to_owned(),
            })),
            vec!["── resumed · old work · /workspace/old ──", ""]
        );
        let summary = line_text(item_lines(&TranscriptItem::BranchSummary(
            "Preserve the target branch.".to_owned(),
        )));
        assert_eq!(summary[0], "◇ branch summary");
        assert!(
            summary
                .iter()
                .any(|line| line.contains("Preserve the target branch."))
        );
    }

    #[test]
    fn context_transcript_renders_categories_top_consumers_and_guidance() {
        let mut state = AppState::new(session());
        let mut snapshot = state.context.clone();
        snapshot.context_window = Some(200_000);
        snapshot.estimated_unfiltered_tokens = 110_000;
        snapshot.estimated_next_request_tokens = 72_000;
        snapshot.estimated_pruned_this_request_tokens = 38_000;
        snapshot.estimated_cumulative_avoided_tokens = 120_000;
        snapshot.categories = vec![
            ContextCategoryEstimate {
                category: ContextCategory::User,
                message_count: 2,
                estimated_tokens: 3_000,
            },
            ContextCategoryEstimate {
                category: ContextCategory::Assistant,
                message_count: 4,
                estimated_tokens: 9_000,
            },
            ContextCategoryEstimate {
                category: ContextCategory::ToolResult,
                message_count: 8,
                estimated_tokens: 90_000,
            },
            ContextCategoryEstimate {
                category: ContextCategory::Other,
                message_count: 1,
                estimated_tokens: 8_000,
            },
        ];
        snapshot.top_consumers = vec![ContextConsumer {
            category: ContextCategory::ToolResult,
            label: "read result · src/lib.rs".to_owned(),
            estimated_tokens: 48_000,
            tool_call_id: Some("call-1".to_owned()),
        }];
        state.transcript.push(TranscriptItem::Context(snapshot));

        let lines = item_lines(&state.transcript[0])
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.into_owned())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(lines.contains("tool results ~90k"));
        assert!(lines.contains("Top consumers"));
        assert!(lines.contains("read result · src/lib.rs"));
        assert!(lines.contains("Suggestion:"));
    }

    #[test]
    fn compaction_separator_uses_before_after_or_before_only_formats() {
        let complete = TranscriptItem::Compaction(CompactionRecord {
            reason: "manual".to_owned(),
            first_kept_entry_id: "entry-1".to_owned(),
            tokens_before: 82_000,
            estimated_tokens_after: Some(31_000),
            tokens_saved: Some(51_000),
            saved_percent: Some(62.0),
            file_count: 14,
            read_file_count: 10,
            modified_file_count: 5,
        });
        let before_only = TranscriptItem::Compaction(CompactionRecord {
            estimated_tokens_after: None,
            tokens_saved: None,
            saved_percent: None,
            ..match &complete {
                TranscriptItem::Compaction(record) => record.clone(),
                _ => unreachable!(),
            }
        });

        let render_item = |item: &TranscriptItem| {
            item_lines(item)
                .into_iter()
                .flat_map(|line| line.spans.into_iter())
                .map(|span| span.content.into_owned())
                .collect::<String>()
        };
        assert!(render_item(&complete).contains("82k → 31k · saved 62% · files 14"));
        assert!(render_item(&before_only).contains("before 82k · files 14"));
    }

    #[test]
    fn renders_only_active_output_and_bordered_input() {
        let mut state = AppState::new(session());
        state.transcript.extend([
            TranscriptItem::User(UserMessage {
                text: "fix the parser".to_owned(),
                status: UserMessageStatus::Accepted,
            }),
            TranscriptItem::Assistant(AssistantMessage {
                text: "I am checking it now.".to_owned(),
                thinking: String::new(),
                complete: false,
            }),
            TranscriptItem::Tool(ToolExecution {
                id: "tool-1".to_owned(),
                name: "read".to_owned(),
                args: serde_json::Value::Null,
                output: String::new(),
                status: ToolStatus::Running,
            }),
        ]);
        state.editor.insert_text("run the tests");
        let presenter = TranscriptPresenter {
            next_item: 1,
            assistant: None,
            live_tail_lines: Vec::new(),
            recent_history_lines: Vec::new(),
            projection_width: None,
        };

        let (text, cursor_visible) = rendered(&state, &presenter);

        assert!(!text.contains("Nabla"));
        assert!(!text.contains("fix the parser"));
        assert!(text.contains("I am checking it now."));
        assert!(text.contains("read · running"));
        assert!(text.contains('╭'));
        assert!(text.contains('╯'));
        assert!(text.contains("run the tests"));
        assert!(cursor_visible);
    }

    #[test]
    fn renders_structured_question_choices_and_custom_answer_row() {
        let mut state = AppState::new(session());
        state.run_state = RunState::Running;
        state.question = Some(QuestionFlowState {
            request_id: "question-1".to_owned(),
            questions: vec![PlanQuestion {
                id: "scope".to_owned(),
                prompt: "Which implementation scope?".to_owned(),
                options: vec![
                    QuestionOption {
                        id: "minimal".to_owned(),
                        label: "Minimal".to_owned(),
                        description: Some("smallest safe change".to_owned()),
                    },
                    QuestionOption {
                        id: "complete".to_owned(),
                        label: "Complete".to_owned(),
                        description: Some("full workflow".to_owned()),
                    },
                ],
            }],
            current: 0,
            selected: 1,
            custom_answer: false,
            editor: EditorState::default(),
            answers: Vec::new(),
            replying: false,
        });

        let (text, cursor_visible) = rendered(&state, &TranscriptPresenter::default());

        assert!(text.contains("Which implementation scope?"));
        assert!(text.contains("Complete"));
        assert!(text.contains("full workflow"));
        assert!(text.contains("Other…"));
        assert!(!cursor_visible);
    }

    #[test]
    fn renders_three_way_plan_review_and_plan_transcript() {
        let mut state = AppState::new(session());
        let artifact = PlanArtifact {
            schema_version: 2,
            id: "plan-1".to_owned(),
            revision: 4,
            status: PlanStatus::Submitted,
            title: "Structured Plan".to_owned(),
            summary: "Persist and review plans.".to_owned(),
            body_markdown: "Implement both execution paths.".to_owned(),
            assumptions: vec!["Current context is the default".to_owned()],
            test_plan: vec!["Run cargo test".to_owned()],
            source_session_id: "session-1".to_owned(),
            created_at: "2026-01-01T00:00:00.000Z".to_owned(),
            updated_at: "2026-01-01T00:00:01.000Z".to_owned(),
            last_execution_error: None,
        };
        state.transcript.push(TranscriptItem::Plan(artifact));
        state.plan_review = Some(PlanReviewState::Menu { selected: 0 });

        let (text, cursor_visible) = rendered(&state, &TranscriptPresenter::default());
        let plan_lines = item_lines(&state.transcript[0])
            .iter()
            .map(ToString::to_string)
            .collect::<String>();

        assert!(text.contains("Execute · current context"));
        assert!(text.contains("Execute · fresh context"));
        assert!(text.contains("Keep discussing"));
        assert!(plan_lines.contains("Structured Plan"));
        assert!(!cursor_visible);
    }

    #[test]
    fn renders_plan_mode_and_locks_the_input_during_approval() {
        let mut state = AppState::new(session());
        state.plan_mode_active = true;
        state.run_state = RunState::Running;
        state.transcript.push(TranscriptItem::Tool(ToolExecution {
            id: "call-1".to_owned(),
            name: "bash".to_owned(),
            args: json!({"command": "cargo test"}),
            output: String::new(),
            status: ToolStatus::WaitingApproval,
        }));
        state.approval = Some(ApprovalState {
            approval_id: "approval-1".to_owned(),
            tool_call_id: "call-1".to_owned(),
            tool_name: "bash".to_owned(),
            input: json!({"command": "cargo test"}),
            agent_id: None,
            agent_profile: None,
            model: None,
            goal_id: None,
            reason: None,
            risk: None,
            selected: 0,
            replying: false,
        });

        let (text, cursor_visible) = rendered(&state, &TranscriptPresenter::default());

        assert!(text.contains("PLAN"));
        assert!(text.contains("cargo test"));
        assert!(text.contains("Command approval"));
        assert!(text.contains("Allow bash?"));
        assert!(text.contains("Allow once"));
        assert!(!cursor_visible);
    }

    #[test]
    fn collapses_successful_read_output_without_losing_request_context() {
        let output = (1..=40)
            .map(|line| format!("secret source line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let item = TranscriptItem::Tool(ToolExecution {
            id: "call-1".to_owned(),
            name: "read".to_owned(),
            args: json!({"path": "src/lib.rs", "offset": 10, "limit": 40}),
            output,
            status: ToolStatus::Succeeded,
        });

        let rendered = item_lines(&item)
            .iter()
            .map(ToString::to_string)
            .collect::<String>();

        assert!(rendered.contains("src/lib.rs"));
        assert!(rendered.contains("40 lines"));
        assert!(rendered.contains("output collapsed"));
        assert!(!rendered.contains("secret source line"));
    }

    #[test]
    fn keeps_failed_read_diagnostics_and_bounds_long_tool_output() {
        let item = TranscriptItem::Tool(ToolExecution {
            id: "call-1".to_owned(),
            name: "read".to_owned(),
            args: json!({"path": "missing.rs"}),
            output: (1..=30)
                .map(|line| format!("diagnostic {line}"))
                .collect::<Vec<_>>()
                .join("\n"),
            status: ToolStatus::Failed,
        });

        let rendered = item_lines(&item)
            .iter()
            .map(ToString::to_string)
            .collect::<String>();

        assert!(rendered.contains("diagnostic 1"));
        assert!(rendered.contains("diagnostic 30"));
        assert!(rendered.contains("18 lines collapsed"));
        assert!(!rendered.contains("diagnostic 15"));
    }

    #[test]
    fn edit_and_write_summaries_do_not_dump_file_contents() {
        let edit = tool_request_summary(
            "edit",
            &json!({
                "path": "src/lib.rs",
                "oldText": "private old body",
                "newText": "private new body"
            }),
        );
        let write = tool_request_summary(
            "write",
            &json!({
                "path": "src/new.rs",
                "content": "private generated body\nsecond line"
            }),
        );

        assert!(edit.contains("src/lib.rs"));
        assert!(edit.contains("-1/+1 lines"));
        assert!(!edit.contains("private"));
        assert!(write.contains("src/new.rs"));
        assert!(write.contains("2 lines"));
        assert!(!write.contains("private"));
    }

    #[test]
    fn renders_markdown_structure_and_inline_emphasis() {
        let item = TranscriptItem::Assistant(AssistantMessage {
            text: [
                "# Result",
                "- **important** and `cargo test`",
                "> quoted detail",
                "[documentation](https://example.com/docs)",
                "```rust",
                "fn main() {}",
                "```",
            ]
            .join("\n"),
            thinking: String::new(),
            complete: true,
        });

        let lines = item_lines(&item);
        let rendered = lines.iter().map(ToString::to_string).collect::<String>();
        let spans = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .collect::<Vec<_>>();

        assert!(rendered.contains("◆ Result"));
        assert!(rendered.contains("• important and cargo test"));
        assert!(rendered.contains("│ quoted detail"));
        assert!(rendered.contains("documentation (https://example.com/docs)"));
        assert!(rendered.contains("┌─ rust"));
        assert!(rendered.contains("│ fn main() {}"));
        assert!(rendered.contains("└─"));
        assert!(!rendered.contains("**"));
        assert!(!rendered.contains("```"));
        assert!(spans.iter().any(|span| {
            span.content == "important" && span.style.add_modifier.contains(Modifier::BOLD)
        }));
        assert!(
            spans
                .iter()
                .any(|span| span.content == "cargo test" && span.style.fg == Some(ORANGE))
        );
        assert!(spans.iter().any(|span| {
            span.content == "https://example.com/docs"
                && span.style.add_modifier.contains(Modifier::UNDERLINED)
        }));
    }

    #[test]
    fn renders_gfm_tables_with_alignment_unicode_and_escaped_pipes() {
        let item = TranscriptItem::Assistant(AssistantMessage {
            text: [
                "| 名称 | 状态 | 数量 |",
                "| :--- | :---: | ---: |",
                "| 编译器 | `a|b` | 12 |",
                "| 标记 | ``a ` b`` | 7 |",
                r"| 转义 | left \| right | 3 |",
            ]
            .join("\n"),
            thinking: String::new(),
            complete: true,
        });

        let lines = item_lines_with_width(&item, 72);
        let rendered = lines.iter().map(ToString::to_string).collect::<String>();

        assert!(rendered.contains('┌'));
        assert!(rendered.contains("名称"));
        assert!(rendered.contains("编译器"));
        assert!(rendered.contains("a|b"));
        assert!(rendered.contains("a ` b"));
        assert!(rendered.contains("left | right"));
        assert!(!rendered.contains(":---"));
        assert!(!rendered.contains("`a|b`"));
        assert!(lines.iter().flat_map(|line| &line.spans).any(|span| {
            (span.content == "a" || span.content == "|") && span.style.fg == Some(ORANGE)
        }));
    }

    #[test]
    fn markdown_tables_fit_common_terminal_widths_without_restoring_markup() {
        let item = TranscriptItem::Assistant(AssistantMessage {
            text: [
                "| Command | Result | Explanation |",
                "| :--- | :---: | ---: |",
                "| `cargo test` | **passed** | 中文与 English mixed content |",
            ]
            .join("\n"),
            thinking: String::new(),
            complete: true,
        });

        for width in [40, 80, 120] {
            let lines = item_lines_with_width(&item, width);
            assert!(
                lines
                    .iter()
                    .all(|line| spans_display_width(&line.spans) <= width),
                "table exceeded terminal width {width}"
            );
            let rendered = line_text(lines).join("\n");
            assert!(rendered.contains("cargo test"));
            assert!(rendered.contains("passed"));
            assert!(!rendered.contains('`'));
            assert!(!rendered.contains("**"));
        }
    }

    #[test]
    fn narrow_markdown_tables_fall_back_to_stacked_records() {
        let item = TranscriptItem::Assistant(AssistantMessage {
            text: [
                "| Name | Status | Detail |",
                "| --- | --- | --- |",
                "| `build` | **failed** | missing dependency |",
            ]
            .join("\n"),
            thinking: String::new(),
            complete: true,
        });

        let rendered = item_lines_with_width(&item, 16)
            .iter()
            .map(ToString::to_string)
            .collect::<String>();

        assert!(rendered.contains("Name: build"));
        assert!(rendered.contains("Status: failed"));
        assert!(rendered.contains("Detail: missing dependency"));
        assert!(!rendered.contains('┌'));
        assert!(!rendered.contains('`'));
        assert!(!rendered.contains("**"));
    }

    #[test]
    fn streaming_markdown_renders_mutable_tables_before_the_block_is_stable() {
        let mut state = AppState::new(session());
        state
            .transcript
            .push(TranscriptItem::Assistant(AssistantMessage {
                text: "| Name | Value |\n| --- | ---: |\n| alpha | 1 |\n".to_owned(),
                thinking: String::new(),
                complete: false,
            }));
        let mut presenter = TranscriptPresenter::default();

        assert!(presenter.project_ready(&state, 60).is_empty());
        assert!(presenter.has_mutable_table());
        let active = line_text(presenter.active_lines_with_width(&state, 60)).join("\n");
        assert!(active.contains("Name"));
        assert!(active.contains("alpha"));
        assert!(!active.contains("---:"));

        let TranscriptItem::Assistant(message) = &mut state.transcript[0] else {
            panic!("expected assistant");
        };
        message.text.push_str("| beta | ``a|b`` |");

        assert!(presenter.project_ready(&state, 60).is_empty());
        let active = line_text(presenter.active_lines_with_width(&state, 60)).join("\n");
        assert!(active.contains("beta"));
        assert!(active.contains("a|b"));
        assert!(!active.contains('`'));

        let TranscriptItem::Assistant(message) = &mut state.transcript[0] else {
            panic!("expected assistant");
        };
        message
            .text
            .truncate(message.text.len() - "| beta | ``a|b`` |".len());
        message.text.push_str("| incomplete");
        let active = line_text(presenter.active_lines_with_width(&state, 60)).join("\n");
        assert!(active.contains("alpha"));
        assert!(!active.contains("incomplete"));

        let TranscriptItem::Assistant(message) = &mut state.transcript[0] else {
            panic!("expected assistant");
        };
        message
            .text
            .truncate(message.text.len() - "| incomplete".len());
        message.text.push_str("| beta | ``a|b`` |");

        let TranscriptItem::Assistant(message) = &mut state.transcript[0] else {
            panic!("expected assistant");
        };
        message.text.push_str("\n\nfollowing");

        let rendered = line_text(presenter.project_ready(&state, 60)).join("\n");
        assert!(rendered.contains("Name"));
        assert!(rendered.contains("alpha"));
        assert!(rendered.contains("beta"));
        assert!(!rendered.contains("---:"));
        assert!(presenter.project_ready(&state, 60).is_empty());
        assert!(!presenter.has_mutable_table());
        assert_eq!(
            line_text(presenter.active_lines_with_width(&state, 60)),
            vec!["  following"]
        );
    }

    #[test]
    fn closed_mutable_table_joins_the_live_tail_without_duplication() {
        use ratatui::{TerminalOptions, Viewport};

        let mut state = AppState::new(session());
        state.run_state = RunState::Running;
        state
            .transcript
            .push(TranscriptItem::Assistant(AssistantMessage {
                text: "| Name | Value |\n| --- | ---: |\n| alpha | `1` |\n".to_owned(),
                thinking: String::new(),
                complete: false,
            }));
        let backend = TestBackend::new(60, 16);
        let mut terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Inline(8),
            },
        )
        .unwrap();
        let mut presenter = TranscriptPresenter::default();

        let mut prepared = presenter.prepare(&state, 60);
        prepared.retain_live_tail(&state, LIVE_TRANSCRIPT_TAIL_HEIGHT);
        assert!(prepared.history_lines.is_empty());
        presenter
            .commit_with_backend(&mut terminal, prepared)
            .unwrap();
        assert!(presenter.has_mutable_table());
        assert!(
            line_text(presenter.active_lines_with_width(&state, 60))
                .join("\n")
                .contains("alpha")
        );

        let TranscriptItem::Assistant(message) = &mut state.transcript[0] else {
            panic!("expected assistant");
        };
        message.text.push_str("\nfollowing");
        let mut prepared = presenter.prepare(&state, 60);
        prepared.retain_live_tail(&state, LIVE_TRANSCRIPT_TAIL_HEIGHT);
        assert!(prepared.history_lines.is_empty());
        presenter
            .commit_with_backend(&mut terminal, prepared)
            .unwrap();
        assert!(!presenter.has_mutable_table());
        let live = line_text(presenter.active_lines_with_width(&state, 60)).join("\n");
        assert_eq!(live.matches("Name").count(), 1);
        assert_eq!(live.matches("alpha").count(), 1);
        assert!(live.contains("following"));
        assert!(!live.contains('`'));
    }

    #[test]
    fn streaming_markdown_preserves_fenced_code_state() {
        let mut state = AppState::new(session());
        state
            .transcript
            .push(TranscriptItem::Assistant(AssistantMessage {
                text: "```rust\nlet value = 1;\n".to_owned(),
                thinking: String::new(),
                complete: false,
            }));
        let mut presenter = TranscriptPresenter::default();

        assert_eq!(
            line_text(presenter.project_ready(&state, 80)),
            vec!["• ┌─ rust", "  │ let value = 1;"]
        );
        if let TranscriptItem::Assistant(message) = &mut state.transcript[0] {
            message.text.push_str("```");
        } else {
            panic!("expected assistant");
        }

        assert_eq!(line_text(presenter.active_lines(&state)), vec!["  └─"]);
        if let TranscriptItem::Assistant(message) = &mut state.transcript[0] {
            message.complete = true;
        }
        assert_eq!(
            line_text(presenter.project_ready(&state, 80)),
            vec!["  └─", ""]
        );
    }

    #[test]
    fn renders_command_menu_above_composer_without_a_counter() {
        let mut state = AppState::with_commands(
            session(),
            vec![DiscoveredCommand {
                name: "fix-tests".to_owned(),
                description: "Fix failing tests".to_owned(),
                source: "prompt".to_owned(),
            }],
        );
        state.editor.insert_text("/fi");

        let (text, _) = rendered(&state, &TranscriptPresenter::default());
        let rows = text
            .chars()
            .collect::<Vec<_>>()
            .chunks(80)
            .map(|row| row.iter().collect::<String>())
            .collect::<Vec<_>>();
        let input_row = rows
            .iter()
            .position(|row| row.contains("│› /fi"))
            .expect("composer row");
        let command_row_index = rows
            .iter()
            .position(|row| row.contains("/fix-tests"))
            .expect("command candidate row");
        let command_row = &rows[command_row_index];

        assert!(text.contains("/fix-tests"));
        assert!(text.contains("ctrl+n/p navigate"));
        assert!(text.contains("Fix failing tests"));
        assert!(text.contains("› /fix-tests"));
        assert!(command_row_index < input_row);
        assert!(command_row.ends_with("Fix failing tests"));
        assert!(!text.contains("1/1"));
    }

    #[test]
    fn command_menu_right_aligns_descriptions_and_fills_the_selected_row() {
        let mut state = AppState::with_commands(
            session(),
            vec![DiscoveredCommand {
                name: "fix-tests".to_owned(),
                description: "Fix failing tests".to_owned(),
                source: "prompt".to_owned(),
            }],
        );
        state.editor.insert_text("/fi");

        for width in [40, 80, 120] {
            let backend = TestBackend::new(width, 1);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal
                .draw(|frame| render_command_menu(frame, &state, frame.area()))
                .unwrap();
            let row = terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(row.starts_with("› /fix-tests"));
            assert!(row.ends_with("Fix failing tests"));
            assert!(!row.contains("1/1"));
            assert!(
                terminal
                    .backend()
                    .buffer()
                    .content()
                    .iter()
                    .all(|cell| cell.bg == MENU_SELECTED)
            );
        }
    }

    #[test]
    fn narrow_command_menu_preserves_the_command_name_before_description() {
        let mut state = AppState::with_commands(
            session(),
            vec![DiscoveredCommand {
                name: "fix-tests".to_owned(),
                description: "Fix failing tests".to_owned(),
                source: "prompt".to_owned(),
            }],
        );
        state.editor.insert_text("/fi");
        let backend = TestBackend::new(12, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_command_menu(frame, &state, frame.area()))
            .unwrap();
        let row = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert_eq!(row, "› /fix-tests");
        assert!(!row.contains("Fix failing tests"));
    }

    #[test]
    fn command_menu_preserves_transcript_and_keeps_latest_content_above_composer() {
        let mut state = AppState::new(session());
        state
            .transcript
            .push(TranscriptItem::Assistant(AssistantMessage {
                text: (1..=12)
                    .map(|line| format!("answer line {line}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
                thinking: String::new(),
                complete: false,
            }));
        state.editor.insert_text("/");

        let (text, _) = rendered_size(&state, &TranscriptPresenter::default(), 80, 16);
        let rows = text
            .chars()
            .collect::<Vec<_>>()
            .chunks(80)
            .map(|row| row.iter().collect::<String>())
            .collect::<Vec<_>>();
        let latest = rows
            .iter()
            .position(|row| row.contains("answer line 12"))
            .expect("latest answer");
        let composer = rows
            .iter()
            .position(|row| row.contains("│› /"))
            .expect("composer");
        let candidate = rows
            .iter()
            .position(|row| row.contains("› /login"))
            .expect("candidate");

        assert!(latest < candidate);
        assert!(candidate < composer);
    }

    #[test]
    fn command_menu_scrolls_to_keep_the_selected_candidate_visible() {
        let mut state = AppState::new(session());
        state.editor.insert_text("/");

        let (initial, _) = rendered(&state, &TranscriptPresenter::default());
        assert!(initial.contains("› /login"));
        assert!(!initial.contains("/tree"));

        for _ in 0..16 {
            state.select_next_command();
        }
        let (scrolled, _) = rendered(&state, &TranscriptPresenter::default());

        assert!(scrolled.contains("› /tree"));
        assert!(!scrolled.contains("/login"));
        assert_eq!(state.command_candidates().len(), 17);
    }

    #[test]
    fn agent_picker_and_subagent_result_render_profile_details() {
        let mut state = AppState::new(session());
        state.agents = serde_json::from_value(serde_json::json!({
            "maxParallel": 3,
            "profiles": [{
                "name": "reviewer",
                "description": "Review changes independently",
                "source": "/tmp/reviewer.md",
                "model": "test/model",
                "thinkingLevel": "high",
                "skills": [],
                "tools": ["read"],
                "permission": "read:allow",
                "maxParallel": 1,
                "maxTurns": 12,
                "disabled": false,
                "unavailableReason": null
            }],
            "active": [],
            "diagnostics": []
        }))
        .unwrap();
        state.agent_picker = Some(crate::state::AgentPickerState::new(&state.agents));

        let (picker, _) = rendered(&state, &TranscriptPresenter::default());
        assert!(picker.contains("Select Subagent"));
        assert!(picker.contains("reviewer"));
        assert!(picker.contains("Review changes independently"));

        state.agent_picker = None;
        state.transcript.push(TranscriptItem::Subagent(
            serde_json::from_value(serde_json::json!({
                "event": "completed",
                "agent": {
                    "id": "agent-1",
                    "profile": "reviewer",
                    "task": "Review the diff",
                    "lifecycle": "running",
                    "startedAt": "2026-01-01T00:00:00Z",
                    "turns": 3,
                    "maxTurns": 12,
                    "model": "test/model",
                    "originSessionId": "session-1"
                },
                "result": {"summary": "No regressions found"},
                "error": null
            }))
            .unwrap(),
        ));
        let (result, _) = rendered(&state, &TranscriptPresenter::default());
        assert!(result.contains("agent-1 [reviewer] completed"));
        assert!(result.contains("No regressions found"));
    }

    #[test]
    fn transcript_viewer_renders_modes_and_per_tool_expansion() {
        let mut state = AppState::new(session());
        state.transcript.extend([
            TranscriptItem::Assistant(AssistantMessage {
                text: "Final answer".to_owned(),
                thinking: "private reasoning".to_owned(),
                complete: true,
            }),
            TranscriptItem::Tool(ToolExecution {
                id: "read-1".to_owned(),
                name: "read".to_owned(),
                args: json!({"path": "src/lib.rs"}),
                output: "source body".to_owned(),
                status: ToolStatus::Succeeded,
            }),
            TranscriptItem::Tool(ToolExecution {
                id: "write-1".to_owned(),
                name: "write".to_owned(),
                args: json!({"path": "src/new.rs", "content": "private"}),
                output: "wrote file".to_owned(),
                status: ToolStatus::Succeeded,
            }),
        ]);
        state.transcript_viewer = Some(TranscriptViewerState::new(
            TranscriptViewMode::Normal,
            &state.transcript,
        ));

        let (normal, _) = rendered_width(&state, &TranscriptPresenter::default(), 120);
        assert!(normal.contains("Transcript · Normal"));
        assert!(normal.contains("▸ ✓ read"));
        assert!(normal.contains("Final answer"));
        assert!(!normal.contains("source body"));
        assert!(!normal.contains("private reasoning"));

        state
            .transcript_viewer
            .as_mut()
            .expect("viewer")
            .tool_expansion_overrides
            .insert("read-1".to_owned(), true);
        let (expanded, _) = rendered_width(&state, &TranscriptPresenter::default(), 120);
        assert!(expanded.contains("▾ ✓ read"));
        assert!(expanded.contains("source body"));

        let viewer = state.transcript_viewer.as_mut().expect("viewer");
        viewer.mode = TranscriptViewMode::Summary;
        viewer.tool_expansion_overrides.clear();
        let (summary, _) = rendered_width(&state, &TranscriptPresenter::default(), 120);
        assert!(summary.contains("Transcript · Summary"));
        assert!(!summary.contains("▸ ✓ read"));
        assert!(summary.contains("▸ ✓ write"));
    }

    #[test]
    fn worktree_integration_prompt_shows_safe_actions() {
        let mut state = AppState::new(session());
        state.integration_prompt = Some(crate::state::IntegrationPromptState {
            agent: serde_json::from_value(serde_json::json!({
                "id": "agent-2",
                "profile": "worker",
                "task": "Implement",
                "lifecycle": "awaiting_integration",
                "startedAt": "2026-01-01T00:00:00Z",
                "turns": 4,
                "maxTurns": 32,
                "model": "test/model",
                "originSessionId": "session-1",
                "isolationBackend": "worktree",
                "integrationStatus": "conflicted"
            }))
            .unwrap(),
            integration: serde_json::from_value(serde_json::json!({
                "backend": "worktree",
                "status": "conflicted",
                "changedPaths": ["src/app.rs", "src/ui.rs"],
                "patchBytes": 2048
            }))
            .unwrap(),
            selected: 1,
            submitting: false,
        });

        let (text, _) = rendered(&state, &TranscriptPresenter::default());
        assert!(text.contains("agent-2 [worker]"));
        assert!(text.contains("2 changed files"));
        assert!(text.contains("Apply"));
        assert!(text.contains("Resolve"));
        assert!(text.contains("Keep"));
        assert!(text.contains("Discard"));
    }

    #[test]
    fn authentication_secret_input_is_masked_and_not_rendered_verbatim() {
        let mut state = AppState::new(session());
        let mut editor = crate::state::EditorState::default();
        editor.insert_text("sk-secret");
        state.run_state = RunState::Authenticating;
        state.auth_state = AuthState::Running(Box::new(AuthFlowState {
            id: "flow-1".to_owned(),
            provider_name: "Test Provider".to_owned(),
            status: "Input required".to_owned(),
            url: None,
            device_code: None,
            prompt: Some(AuthPromptState {
                id: "prompt-1".to_owned(),
                kind: AuthPromptKind::Secret,
                message: "Enter API key".to_owned(),
                placeholder: None,
                options: Vec::new(),
                selected: 0,
                editor,
            }),
        }));

        let (text, _) = rendered(&state, &TranscriptPresenter::default());

        assert!(text.contains("Test Provider"));
        assert!(text.contains("•••••••••"));
        assert!(!text.contains("sk-secret"));
    }

    #[test]
    fn authentication_provider_search_filters_the_full_width_menu() {
        let mut state = AppState::new(session());
        let mut filter = EditorState::default();
        filter.insert_text("github device");
        state.run_state = RunState::Authenticating;
        state.auth_state = AuthState::Selecting {
            choices: vec![
                AuthChoice {
                    provider_id: "openai-codex".to_owned(),
                    provider_name: "OpenAI Codex".to_owned(),
                    auth_type: "oauth".to_owned(),
                    label: "ChatGPT Plus/Pro".to_owned(),
                    configured: false,
                },
                AuthChoice {
                    provider_id: "github-copilot".to_owned(),
                    provider_name: "GitHub Copilot".to_owned(),
                    auth_type: "oauth".to_owned(),
                    label: "Device login".to_owned(),
                    configured: false,
                },
            ],
            selected: 0,
            filter,
        };

        let (text, cursor_visible) = rendered(&state, &TranscriptPresenter::default());

        assert!(text.contains("GitHub Copilot"));
        assert!(text.contains("github device"));
        assert!(text.contains("type to filter"));
        assert!(!text.contains("OpenAI Codex"));
        assert!(cursor_visible);
    }

    #[test]
    fn oauth_url_is_visible_and_is_the_terminal_osc8_hyperlink_text() {
        let mut state = AppState::new(session());
        let url = "https://auth.openai.com/oauth/authorize?state=test";
        state.run_state = RunState::Authenticating;
        state.auth_state = AuthState::Running(Box::new(AuthFlowState {
            id: "flow-1".to_owned(),
            provider_name: "OpenAI Codex".to_owned(),
            status: "Continue in your browser".to_owned(),
            url: Some(url.to_owned()),
            device_code: None,
            prompt: None,
        }));

        let (text, _) = rendered(&state, &TranscriptPresenter::default());
        let sequence = auth_hyperlink_sequence(&state, Rect::new(0, 5, 80, 12))
            .expect("safe auth URL should produce a terminal overlay");

        assert!(text.contains(url));
        assert!(sequence.contains(&format!("\x1b]8;;{url}\x1b\\")));
        assert!(sequence.contains(url));
        assert!(sequence.contains("\x1b]8;;\x1b\\"));

        if let AuthState::Running(flow) = &mut state.auth_state {
            flow.url = Some("javascript:alert(1)".to_owned());
        }
        assert!(auth_hyperlink_sequence(&state, Rect::new(0, 5, 80, 12)).is_none());
    }

    #[test]
    fn completed_items_commit_immediately_to_native_scrollback() {
        use ratatui::{TerminalOptions, Viewport};

        let mut state = AppState::new(session());
        state.transcript.extend([
            TranscriptItem::User(UserMessage {
                text: "fix parser".to_owned(),
                status: UserMessageStatus::Accepted,
            }),
            TranscriptItem::Assistant(AssistantMessage {
                text: "fixed".to_owned(),
                thinking: String::new(),
                complete: true,
            }),
        ]);
        let viewport_height = inline_viewport_height(24);
        let backend = TestBackend::new(40, viewport_height);
        let mut terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Inline(viewport_height),
            },
        )
        .unwrap();
        let mut presenter = TranscriptPresenter::default();

        let mut prepared = presenter.prepare(&state, 40);
        prepared.release_live_tail();
        assert_eq!(
            presenter.next_item(),
            0,
            "preparing a frame must not consume transcript state"
        );
        assert_eq!(prepared.projected().next_item(), 2);
        presenter
            .commit_with_backend(&mut terminal, prepared)
            .unwrap();

        assert_eq!(presenter.next_item(), 2);
        let scrollback_text = terminal
            .backend()
            .scrollback()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(scrollback_text.contains("› fix parser"));
        assert!(scrollback_text.contains("• fixed"));
        assert!(presenter.live_tail_lines.is_empty());

        let metrics =
            measure_layout_request(&state, &presenter, 40, 24).resolve_layout(viewport_height);
        terminal
            .draw(|frame| {
                let _ = render(frame, &state, &presenter, metrics);
            })
            .unwrap();
        let viewport_text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(!viewport_text.contains("› fix parser"));
        assert!(!viewport_text.contains("• fixed"));

        presenter.flush_with_backend(&mut terminal).unwrap();
        assert!(presenter.live_tail_lines.is_empty());
    }

    #[test]
    fn recent_native_history_can_restore_rows_covered_by_the_command_menu() {
        use ratatui::{TerminalOptions, Viewport};

        let mut state = AppState::new(session());
        state
            .transcript
            .push(TranscriptItem::Assistant(AssistantMessage {
                text: "restored line one\nrestored line two".to_owned(),
                thinking: String::new(),
                complete: true,
            }));
        let backend = TestBackend::new(40, 8);
        let mut terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Inline(4),
            },
        )
        .unwrap();
        let mut presenter = TranscriptPresenter::default();
        let mut prepared = presenter.prepare(&state, 40);
        prepared.release_live_tail();
        assert!(presenter.recent_history_lines.is_empty());
        presenter
            .commit_with_backend(&mut terminal, prepared)
            .unwrap();
        assert!(!presenter.recent_history_lines.is_empty());

        let backend = TestBackend::new(40, 3);
        let mut restoration = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Fixed(Rect::new(0, 0, 40, 3)),
            },
        )
        .unwrap();
        restoration
            .draw(|frame| {
                render_recent_history_background(frame, &presenter, frame.area());
            })
            .unwrap();
        let text = restoration
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("restored line one"));
        assert!(text.contains("restored line two"));
    }

    #[test]
    fn streaming_rows_form_one_contiguous_block_above_the_composer() {
        let mut state = AppState::new(session());
        state.run_state = RunState::Running;
        state
            .transcript
            .push(TranscriptItem::Assistant(AssistantMessage {
                text: "one\ntwo\npartial".to_owned(),
                thinking: String::new(),
                complete: false,
            }));
        let mut presenter = TranscriptPresenter::default();
        let mut prepared = presenter.prepare(&state, 40);
        prepared.retain_live_tail(&state, LIVE_TRANSCRIPT_TAIL_HEIGHT);
        let request = measure_layout_request(&state, prepared.projected(), 40, 10);
        let metrics = request.resolve_layout(request.desired_height());
        let backend = TestBackend::new(40, metrics.desired_height);
        let mut terminal = Terminal::new(backend).unwrap();
        presenter
            .commit_with_backend(&mut terminal, prepared)
            .unwrap();
        terminal
            .draw(|frame| {
                let _ = render(frame, &state, &presenter, metrics);
            })
            .unwrap();
        let rows = terminal
            .backend()
            .buffer()
            .content()
            .chunks(40)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<Vec<_>>();
        let one = rows.iter().position(|row| row.contains("one")).unwrap();
        let two = rows.iter().position(|row| row.contains("two")).unwrap();
        let partial = rows.iter().position(|row| row.contains("partial")).unwrap();
        let composer = rows.iter().position(|row| row.contains('╭')).unwrap();

        assert_eq!(two, one + 1, "rendered rows: {rows:?}");
        assert_eq!(partial, two + 1, "rendered rows: {rows:?}");
        assert_eq!(composer, partial + 1, "rendered rows: {rows:?}");
    }

    #[test]
    fn tool_and_assistant_output_share_the_same_live_window() {
        let mut state = AppState::new(session());
        state.run_state = RunState::Running;
        state.transcript.extend([
            TranscriptItem::Assistant(AssistantMessage {
                text: "analysis one\nanalysis two".to_owned(),
                thinking: String::new(),
                complete: true,
            }),
            TranscriptItem::Tool(ToolExecution {
                id: "tool-1".to_owned(),
                name: "read".to_owned(),
                args: serde_json::Value::Null,
                output: String::new(),
                status: ToolStatus::Running,
            }),
        ]);
        let mut presenter = TranscriptPresenter::default();
        let mut prepared = presenter.prepare(&state, 40);
        prepared.retain_live_tail(&state, LIVE_TRANSCRIPT_TAIL_HEIGHT);
        let request = measure_layout_request(&state, prepared.projected(), 40, 12);
        let metrics = request.resolve_layout(request.desired_height());
        let backend = TestBackend::new(40, metrics.desired_height);
        let mut terminal = Terminal::new(backend).unwrap();
        presenter
            .commit_with_backend(&mut terminal, prepared)
            .unwrap();
        terminal
            .draw(|frame| {
                let _ = render(frame, &state, &presenter, metrics);
            })
            .unwrap();
        let rows = terminal
            .backend()
            .buffer()
            .content()
            .chunks(40)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<Vec<_>>();
        let tool = rows
            .iter()
            .position(|row| row.contains("read · running"))
            .unwrap();
        let composer = rows.iter().position(|row| row.contains('╭')).unwrap();

        assert_eq!(composer, tool + 1, "rendered rows: {rows:?}");
    }

    #[test]
    fn live_tail_commits_only_visual_overflow_and_keeps_the_remainder_visible() {
        let mut state = AppState::new(session());
        state.run_state = RunState::Running;
        let stable = (0..15)
            .map(|index| format!("line {index:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        state
            .transcript
            .push(TranscriptItem::Assistant(AssistantMessage {
                text: format!("{stable}\npartial"),
                thinking: String::new(),
                complete: false,
            }));
        let presenter = TranscriptPresenter::default();
        let mut prepared = presenter.prepare(&state, 40);
        prepared.retain_live_tail(&state, LIVE_TRANSCRIPT_TAIL_HEIGHT);

        let history = line_text(prepared.history_lines.to_vec());
        let active = line_text(prepared.projected().active_lines_with_width(&state, 40));
        assert_eq!(
            history,
            vec!["• line 00", "  line 01", "  line 02", "  line 03"]
        );
        assert_eq!(active.len(), LIVE_TRANSCRIPT_TAIL_HEIGHT as usize);
        assert_eq!(active.first().map(String::as_str), Some("  line 04"));
        assert_eq!(active.last().map(String::as_str), Some("  partial"));
        assert!(active.iter().all(|line| !line.is_empty()));
    }

    #[test]
    fn hidden_trailing_separator_does_not_reduce_the_busy_live_tail() {
        let mut state = AppState::new(session());
        state.run_state = RunState::Running;
        state
            .transcript
            .push(TranscriptItem::Assistant(AssistantMessage {
                text: (0..15)
                    .map(|index| format!("line {index:02}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
                thinking: String::new(),
                complete: true,
            }));
        let presenter = TranscriptPresenter::default();
        let mut prepared = presenter.prepare(&state, 40);
        prepared.retain_live_tail(&state, LIVE_TRANSCRIPT_TAIL_HEIGHT);

        let active = line_text(prepared.projected().active_lines_with_width(&state, 40));
        assert_eq!(active.len(), LIVE_TRANSCRIPT_TAIL_HEIGHT as usize);
        assert_eq!(active.first().map(String::as_str), Some("  line 03"));
        assert_eq!(active.last().map(String::as_str), Some("  line 14"));
    }

    #[test]
    fn idle_release_moves_the_complete_live_window_to_history_once() {
        use ratatui::{TerminalOptions, Viewport};

        let mut state = AppState::new(session());
        state.run_state = RunState::Running;
        state
            .transcript
            .push(TranscriptItem::Assistant(AssistantMessage {
                text: "one\ntwo\npartial".to_owned(),
                thinking: String::new(),
                complete: false,
            }));
        let backend = TestBackend::new(40, 8);
        let mut terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Inline(8),
            },
        )
        .unwrap();
        let mut presenter = TranscriptPresenter::default();
        let mut prepared = presenter.prepare(&state, 40);
        prepared.retain_live_tail(&state, LIVE_TRANSCRIPT_TAIL_HEIGHT);
        presenter
            .commit_with_backend(&mut terminal, prepared)
            .unwrap();

        let TranscriptItem::Assistant(message) = &mut state.transcript[0] else {
            panic!("expected assistant");
        };
        message.complete = true;
        state.run_state = RunState::Idle;
        let mut prepared = presenter.prepare(&state, 40);
        prepared.release_live_tail();
        let history = line_text(prepared.history_lines.to_vec()).join("\n");
        assert_eq!(history.matches("one").count(), 1);
        assert_eq!(history.matches("two").count(), 1);
        assert_eq!(history.matches("partial").count(), 1);
        assert!(
            prepared
                .projected()
                .active_lines_with_width(&state, 40)
                .is_empty()
        );
        presenter
            .commit_with_backend(&mut terminal, prepared)
            .unwrap();
        assert!(!presenter.has_live_tail());
        assert!(presenter.prepare(&state, 40).history_lines.is_empty());
    }

    #[test]
    fn width_change_flushes_the_old_live_tail_without_reprojecting_it() {
        let mut state = AppState::new(session());
        state.run_state = RunState::Running;
        state
            .transcript
            .push(TranscriptItem::Assistant(AssistantMessage {
                text: "one\ntwo\npartial".to_owned(),
                thinking: String::new(),
                complete: false,
            }));
        let mut presenter = TranscriptPresenter::default();
        let mut first = presenter.prepare(&state, 40);
        first.retain_live_tail(&state, LIVE_TRANSCRIPT_TAIL_HEIGHT);
        presenter = first.projected().clone();

        let mut resized = presenter.prepare(&state, 20);
        resized.retain_live_tail(&state, LIVE_TRANSCRIPT_TAIL_HEIGHT);
        assert_eq!(
            line_text(resized.history_lines.to_vec()),
            vec!["• one", "  two"]
        );
        assert_eq!(
            line_text(resized.projected().active_lines_with_width(&state, 20),),
            vec!["  partial"]
        );
    }

    #[test]
    fn uncommitted_history_remains_visible_in_a_fixed_fallback_viewport() {
        use ratatui::{TerminalOptions, Viewport};

        let mut state = AppState::new(session());
        state.run_state = RunState::Running;
        state.transcript.extend([
            TranscriptItem::User(UserMessage {
                text: "inspect evidence".to_owned(),
                status: UserMessageStatus::Accepted,
            }),
            TranscriptItem::Assistant(AssistantMessage {
                text: "first stable line\nsecond stable line\npartial".to_owned(),
                thinking: String::new(),
                complete: false,
            }),
        ]);
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Fixed(Rect::new(0, 0, 40, 10)),
            },
        )
        .unwrap();
        let presenter = TranscriptPresenter::default();
        let request = measure_layout_request(&state, &presenter, 40, 10);
        let metrics = request.resolve_layout(10);

        terminal
            .draw(|frame| {
                let _ = render(frame, &state, &presenter, metrics);
            })
            .unwrap();

        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("first stable line"));
        assert!(text.contains("second stable line"));
        assert!(text.contains("partial"));
        assert_eq!(
            presenter.next_item(),
            0,
            "fallback rendering must not consume native history"
        );
    }

    #[test]
    fn large_completed_transcript_does_not_retain_a_fixed_tail() {
        use ratatui::{TerminalOptions, Viewport};

        let mut state = AppState::new(session());
        state
            .transcript
            .extend((0..40).map(|index| TranscriptItem::Notice(format!("notice {index:02}"))));
        let backend = TestBackend::new(40, 16);
        let mut terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Inline(16),
            },
        )
        .unwrap();
        let mut presenter = TranscriptPresenter::default();

        let mut prepared = presenter.prepare(&state, 40);
        prepared.release_live_tail();
        presenter
            .commit_with_backend(&mut terminal, prepared)
            .unwrap();

        let scrollback_text = terminal
            .backend()
            .scrollback()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(scrollback_text.contains("notice 00"));
        assert!(scrollback_text.contains("notice 39"));
        assert!(presenter.live_tail_lines.is_empty());
    }

    #[test]
    fn disconnect_maps_to_visible_error_state() {
        let mut app = App::new(session());
        app.update(AppEvent::Runtime(RuntimeEvent::PiDisconnected));

        let (text, _) = rendered(app.state(), &TranscriptPresenter::default());

        assert!(text.contains("Pi process disconnected"));
        assert!(text.contains("error"));
    }

    #[test]
    fn identifies_only_stable_items_for_native_scrollback() {
        let user = TranscriptItem::User(UserMessage {
            text: "task".to_owned(),
            status: UserMessageStatus::Accepted,
        });
        let assistant = TranscriptItem::Assistant(AssistantMessage {
            text: "working".to_owned(),
            thinking: String::new(),
            complete: false,
        });
        let tool = TranscriptItem::Tool(ToolExecution {
            id: "tool-1".to_owned(),
            name: "read".to_owned(),
            args: serde_json::Value::Null,
            output: String::new(),
            status: ToolStatus::Succeeded,
        });

        assert!(is_complete(&user));
        assert!(!is_complete(&assistant));
        assert!(is_complete(&tool));
    }

    #[test]
    fn streaming_projection_flushes_newlines_without_duplicates() {
        let mut state = AppState::new(session());
        state
            .transcript
            .push(TranscriptItem::Assistant(AssistantMessage {
                text: "first".to_owned(),
                thinking: String::new(),
                complete: false,
            }));
        let mut presenter = TranscriptPresenter::default();

        assert!(presenter.project_ready(&state, 80).is_empty());
        let TranscriptItem::Assistant(message) = &mut state.transcript[0] else {
            panic!("expected assistant");
        };
        message.text.push_str(" line\npartial");

        let first_projection = line_text(presenter.project_ready(&state, 80));
        assert_eq!(first_projection, vec!["• first line"]);
        assert!(presenter.project_ready(&state, 80).is_empty());

        let active = line_text(presenter.active_lines(&state));
        assert_eq!(active, vec!["  partial"]);

        let TranscriptItem::Assistant(message) = &mut state.transcript[0] else {
            panic!("expected assistant");
        };
        message.complete = true;
        let final_projection = line_text(presenter.project_ready(&state, 80));
        assert_eq!(final_projection, vec!["  partial", ""]);
        assert_eq!(presenter.next_item(), 1);
    }

    #[test]
    fn streaming_projection_tracks_unicode_on_byte_boundaries() {
        let mut state = AppState::new(session());
        state
            .transcript
            .push(TranscriptItem::Assistant(AssistantMessage {
                text: "你好\n尾".to_owned(),
                thinking: String::new(),
                complete: false,
            }));
        let mut presenter = TranscriptPresenter::default();

        assert_eq!(
            line_text(presenter.project_ready(&state, 80)),
            vec!["• 你好"]
        );
        assert_eq!(line_text(presenter.active_lines(&state)), vec!["  尾"]);

        let TranscriptItem::Assistant(message) = &mut state.transcript[0] else {
            panic!("expected assistant");
        };
        message.complete = true;
        assert_eq!(
            line_text(presenter.project_ready(&state, 80)),
            vec!["  尾", ""]
        );
    }

    #[test]
    fn streaming_projection_flushes_rows_at_terminal_width() {
        let mut state = AppState::new(session());
        state
            .transcript
            .push(TranscriptItem::Assistant(AssistantMessage {
                text: "abcdef".to_owned(),
                thinking: String::new(),
                complete: false,
            }));
        let mut presenter = TranscriptPresenter::default();

        assert_eq!(
            line_text(presenter.project_ready(&state, 6)),
            vec!["• abcd"]
        );
        assert_eq!(line_text(presenter.active_lines(&state)), vec!["  ef"]);
    }

    #[test]
    fn text_start_flushes_pending_thinking_before_answer() {
        let mut state = AppState::new(session());
        state
            .transcript
            .push(TranscriptItem::Assistant(AssistantMessage {
                text: String::new(),
                thinking: "checking".to_owned(),
                complete: false,
            }));
        let mut presenter = TranscriptPresenter::default();

        assert!(presenter.project_ready(&state, 80).is_empty());
        let TranscriptItem::Assistant(message) = &mut state.transcript[0] else {
            panic!("expected assistant");
        };
        message.text = "answer".to_owned();

        assert_eq!(
            line_text(presenter.project_ready(&state, 80)),
            vec!["· checking"]
        );
        assert_eq!(line_text(presenter.active_lines(&state)), vec!["• answer"]);
    }

    #[test]
    fn tool_lifecycle_commits_one_final_row() {
        let mut state = AppState::new(session());
        state.transcript.push(TranscriptItem::Tool(ToolExecution {
            id: "tool-1".to_owned(),
            name: "read".to_owned(),
            args: serde_json::Value::Null,
            output: String::new(),
            status: ToolStatus::Running,
        }));
        let mut presenter = TranscriptPresenter::default();

        assert!(presenter.project_ready(&state, 80).is_empty());
        assert_eq!(
            line_text(presenter.active_lines(&state)),
            vec!["↳ read · running"]
        );

        let TranscriptItem::Tool(tool) = &mut state.transcript[0] else {
            panic!("expected tool");
        };
        tool.status = ToolStatus::Succeeded;
        assert_eq!(
            line_text(presenter.project_ready(&state, 80)),
            vec!["✓ read · done", ""]
        );
        assert!(presenter.project_ready(&state, 80).is_empty());
    }

    fn line_text(lines: Vec<Line<'static>>) -> Vec<String> {
        lines
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.into_owned())
                    .collect()
            })
            .collect()
    }
}
