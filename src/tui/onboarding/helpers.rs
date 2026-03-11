use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Returns true if the event is a "clear entire field" gesture
/// (Ctrl+Backspace or Alt+Backspace).
pub(super) fn is_clear_field(event: &KeyEvent) -> bool {
    event.code == KeyCode::Backspace
        && (event.modifiers.contains(KeyModifiers::CONTROL)
            || event.modifiers.contains(KeyModifiers::ALT))
}
