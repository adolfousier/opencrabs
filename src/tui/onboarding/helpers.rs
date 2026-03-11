use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Returns true if the event is a "clear entire field" gesture
/// (Ctrl+Backspace or Alt+Backspace).
pub(crate) fn is_clear_field(event: &KeyEvent) -> bool {
    event.code == KeyCode::Backspace
        && (event.modifiers.contains(KeyModifiers::CONTROL)
            || event.modifiers.contains(KeyModifiers::ALT))
}

/// Handle a key event for a text input field with cursor tracking.
/// Returns `true` if the event was consumed.
pub(crate) fn handle_text_input(
    event: &KeyEvent,
    buf: &mut String,
    cursor: &mut usize,
    existing_sentinel: bool,
    char_filter: Option<fn(char) -> bool>,
) -> bool {
    match event.code {
        KeyCode::Char(c) => {
            if let Some(filter) = char_filter
                && !filter(c)
            {
                return true;
            }
            if existing_sentinel {
                buf.clear();
                *cursor = 0;
            }
            buf.insert(*cursor, c);
            *cursor += c.len_utf8();
            true
        }
        KeyCode::Backspace if is_clear_field(event) => {
            buf.clear();
            *cursor = 0;
            true
        }
        KeyCode::Backspace => {
            if existing_sentinel {
                buf.clear();
                *cursor = 0;
            } else if *cursor > 0 {
                // Find the previous char boundary
                let prev = buf[..*cursor]
                    .char_indices()
                    .last()
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                buf.remove(prev);
                *cursor = prev;
            }
            true
        }
        KeyCode::Delete => {
            if existing_sentinel {
                buf.clear();
                *cursor = 0;
            } else if *cursor < buf.len() {
                buf.remove(*cursor);
            }
            true
        }
        KeyCode::Left if event.modifiers.is_empty() => {
            if *cursor > 0 {
                let prev = buf[..*cursor]
                    .char_indices()
                    .last()
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                *cursor = prev;
            }
            true
        }
        KeyCode::Right if event.modifiers.is_empty() => {
            if *cursor < buf.len() {
                let next = buf[*cursor..]
                    .char_indices()
                    .nth(1)
                    .map(|(i, _)| *cursor + i)
                    .unwrap_or(buf.len());
                *cursor = next;
            }
            true
        }
        KeyCode::Left
            if event.modifiers.contains(KeyModifiers::CONTROL)
                || event.modifiers.contains(KeyModifiers::ALT) =>
        {
            // Word jump left
            *cursor = word_boundary_left(buf, *cursor);
            true
        }
        KeyCode::Right
            if event.modifiers.contains(KeyModifiers::CONTROL)
                || event.modifiers.contains(KeyModifiers::ALT) =>
        {
            // Word jump right
            *cursor = word_boundary_right(buf, *cursor);
            true
        }
        KeyCode::Home => {
            *cursor = 0;
            true
        }
        KeyCode::End => {
            *cursor = buf.len();
            true
        }
        _ => false,
    }
}

/// Handle paste for a text input field — inserts at cursor position.
pub(crate) fn handle_text_paste(
    text: &str,
    buf: &mut String,
    cursor: &mut usize,
    existing_sentinel: bool,
    char_filter: Option<fn(char) -> bool>,
) {
    if existing_sentinel {
        buf.clear();
        *cursor = 0;
    }
    let filtered: String = if let Some(filter) = char_filter {
        text.chars().filter(|c| filter(*c)).collect()
    } else {
        text.to_string()
    };
    buf.insert_str(*cursor, &filtered);
    *cursor += filtered.len();
}

fn word_boundary_left(s: &str, pos: usize) -> usize {
    let bytes = s.as_bytes();
    if pos == 0 {
        return 0;
    }
    let mut i = pos - 1;
    // Skip whitespace/punctuation
    while i > 0 && !bytes[i].is_ascii_alphanumeric() {
        i -= 1;
    }
    // Skip word chars
    while i > 0 && bytes[i - 1].is_ascii_alphanumeric() {
        i -= 1;
    }
    i
}

fn word_boundary_right(s: &str, pos: usize) -> usize {
    let len = s.len();
    let bytes = s.as_bytes();
    if pos >= len {
        return len;
    }
    let mut i = pos;
    // Skip current word chars
    while i < len && bytes[i].is_ascii_alphanumeric() {
        i += 1;
    }
    // Skip whitespace/punctuation
    while i < len && !bytes[i].is_ascii_alphanumeric() {
        i += 1;
    }
    i
}
