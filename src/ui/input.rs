use crossterm::event::{Event, KeyEvent};

use super::{
    store::{FocusTarget, ModalId, OverlayId},
    types::TerminalSize,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutedInput {
    Prompt(KeyEvent),
    Overlay(OverlayId, KeyEvent),
    Transcript(KeyEvent),
    Modal(ModalId, KeyEvent),
    Paste { focus: FocusTarget, text: String },
    Resize(TerminalSize),
    FocusChanged(bool),
}

#[derive(Debug, Clone, Copy, Default)]
pub struct InputRouter;

impl InputRouter {
    pub fn route(self, event: Event, focus: FocusTarget) -> Option<RoutedInput> {
        match event {
            Event::Key(key) => Some(match focus {
                FocusTarget::Prompt => RoutedInput::Prompt(key),
                FocusTarget::Overlay(id) => RoutedInput::Overlay(id, key),
                FocusTarget::Transcript => RoutedInput::Transcript(key),
                FocusTarget::Modal(id) => RoutedInput::Modal(id, key),
            }),
            Event::Paste(text) => Some(RoutedInput::Paste { focus, text }),
            // Nabla's alternate views and inline panels are deliberately
            // keyboard-only; mouse events never enter modal state.
            Event::Mouse(_) => None,
            Event::Resize(width, height) => {
                Some(RoutedInput::Resize(TerminalSize::new(width, height)))
            }
            Event::FocusGained => Some(RoutedInput::FocusChanged(true)),
            Event::FocusLost => Some(RoutedInput::FocusChanged(false)),
        }
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };

    use super::*;

    #[test]
    fn keys_are_dispatched_only_to_the_focused_controller() {
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(
            InputRouter.route(Event::Key(key), FocusTarget::Prompt),
            Some(RoutedInput::Prompt(key))
        );
        assert_eq!(
            InputRouter.route(Event::Key(key), FocusTarget::Overlay(4)),
            Some(RoutedInput::Overlay(4, key))
        );
        assert_eq!(
            InputRouter.route(Event::Key(key), FocusTarget::Modal(9)),
            Some(RoutedInput::Modal(9, key))
        );
    }

    #[test]
    fn mouse_events_are_not_routed_into_panels_or_alternate_views() {
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 3,
            row: 2,
            modifiers: KeyModifiers::NONE,
        };
        assert_eq!(
            InputRouter.route(Event::Mouse(mouse), FocusTarget::Modal(9)),
            None
        );
    }
}
