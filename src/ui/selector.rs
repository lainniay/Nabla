use super::store::EditorCore;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::selection::{centered_visible_start, next_wrapped, previous_wrapped};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionNavigation {
    Previous,
    Next,
}

pub fn selection_navigation(key: KeyEvent) -> Option<SelectionNavigation> {
    if matches!(key.code, KeyCode::Up | KeyCode::BackTab)
        || (key.code == KeyCode::Tab && key.modifiers.contains(KeyModifiers::SHIFT))
        || (key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('p' | 'P')))
    {
        return Some(SelectionNavigation::Previous);
    }
    if key.code == KeyCode::Down
        || (key.code == KeyCode::Tab && !key.modifiers.contains(KeyModifiers::SHIFT))
        || (key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('n' | 'N')))
    {
        return Some(SelectionNavigation::Next);
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterMode {
    None,
    Prefix,
    Fuzzy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectorAction {
    Accept,
    Preview,
    Deny,
    Custom(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectorResult {
    Cancelled,
    Action(SelectorAction),
    Noop,
}

pub trait SelectorPolicy<T> {
    fn actions(&self, item: &T) -> Vec<SelectorAction>;
    fn default_action(&self) -> Option<SelectorAction>;
    fn on_escape(&self) -> SelectorResult;
    fn filter_mode(&self) -> FilterMode;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectorModel<T> {
    pub items: Vec<T>,
    pub selected: usize,
    pub filter: EditorCore,
    pub loading: bool,
    pub error: Option<String>,
}

impl<T> SelectorModel<T> {
    pub fn selected(&self) -> Option<&T> {
        self.items.get(self.selected)
    }

    pub fn select_previous(&mut self) {
        self.selected = previous_wrapped(self.selected, self.items.len());
    }

    pub fn select_next(&mut self) {
        self.selected = next_wrapped(self.selected, self.items.len());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualList {
    pub total: usize,
    pub selected: usize,
    pub visible_rows: usize,
}

impl VirtualList {
    pub fn visible_range(self) -> std::ops::Range<usize> {
        if self.total == 0 || self.visible_rows == 0 {
            return 0..0;
        }
        let start = centered_visible_start(self.total, self.selected, self.visible_rows);
        start..start.saturating_add(self.visible_rows).min(self.total)
    }
}

/// Approval semantics intentionally differ from completion semantics: Enter
/// has no implicit accept action until the policy explicitly supplies one.
#[derive(Debug, Clone, Copy)]
pub struct ApprovalPolicy {
    pub high_risk: bool,
}

impl<T> SelectorPolicy<T> for ApprovalPolicy {
    fn actions(&self, _item: &T) -> Vec<SelectorAction> {
        vec![SelectorAction::Accept, SelectorAction::Deny]
    }

    fn default_action(&self) -> Option<SelectorAction> {
        (!self.high_risk).then_some(SelectorAction::Accept)
    }

    fn on_escape(&self) -> SelectorResult {
        SelectorResult::Action(SelectorAction::Deny)
    }

    fn filter_mode(&self) -> FilterMode {
        FilterMode::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_list_is_bounded_for_empty_and_large_collections() {
        assert_eq!(
            VirtualList {
                total: 0,
                selected: 0,
                visible_rows: 8,
            }
            .visible_range(),
            0..0
        );
        assert_eq!(
            VirtualList {
                total: 1000,
                selected: 500,
                visible_rows: 9,
            }
            .visible_range(),
            496..505
        );
    }

    #[test]
    fn high_risk_approval_never_inherits_completion_default_accept() {
        let policy = ApprovalPolicy { high_risk: true };
        assert_eq!(
            <ApprovalPolicy as SelectorPolicy<()>>::default_action(&policy),
            None
        );
        assert_eq!(
            <ApprovalPolicy as SelectorPolicy<()>>::on_escape(&policy),
            SelectorResult::Action(SelectorAction::Deny)
        );
    }

    #[test]
    fn keyboard_navigation_normalizes_tab_shift_tab_and_ctrl_np() {
        for key in [
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL),
        ] {
            assert_eq!(selection_navigation(key), Some(SelectionNavigation::Next));
        }
        for key in [
            KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
            KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT),
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL),
        ] {
            assert_eq!(
                selection_navigation(key),
                Some(SelectionNavigation::Previous)
            );
        }
    }
}
