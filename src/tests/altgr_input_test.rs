use crossterm::event::KeyModifiers;

/// The AltGr character-acceptance predicate from `src/tui/app/input.rs`.
/// Extracted here as a pure function so we can test it without a full App.
///
/// The actual guard is:
///   `!event.modifiers.contains(CONTROL) || event.modifiers.contains(ALT)`
///
/// Meaning: accept if CONTROL is NOT pressed, OR if ALT is also pressed
/// (CTRL+ALT = AltGr on Windows).
fn accepts_char_input(modifiers: KeyModifiers) -> bool {
    !modifiers.contains(KeyModifiers::CONTROL) || modifiers.contains(KeyModifiers::ALT)
}

#[test]
fn normal_typing_accepted() {
    assert!(accepts_char_input(KeyModifiers::empty()));
}

#[test]
fn shift_accepted() {
    assert!(accepts_char_input(KeyModifiers::SHIFT));
}

#[test]
fn ctrl_alone_rejected() {
    assert!(!accepts_char_input(KeyModifiers::CONTROL));
}

#[test]
fn ctrl_shift_rejected() {
    assert!(!accepts_char_input(
        KeyModifiers::CONTROL | KeyModifiers::SHIFT
    ));
}

#[test]
fn altgr_ctrl_alt_accepted() {
    // AltGr on Windows = CTRL+ALT
    assert!(accepts_char_input(
        KeyModifiers::CONTROL | KeyModifiers::ALT
    ));
}

#[test]
fn alt_alone_accepted() {
    assert!(accepts_char_input(KeyModifiers::ALT));
}

#[test]
fn super_key_accepted() {
    assert!(accepts_char_input(KeyModifiers::SUPER));
}

#[test]
fn ctrl_alt_shift_accepted() {
    // AltGr + Shift on some layouts
    assert!(accepts_char_input(
        KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT
    ));
}
