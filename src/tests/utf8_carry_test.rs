//! Tests for `Utf8Carry`, the streaming carry buffer that reassembles
//! multi-byte UTF-8 characters split across chunk boundaries.
//!
//! Extracted from an inline `#[cfg(test)]` block that landed with PR #1608;
//! project policy (CONTRIBUTING.md) requires all tests under `src/tests/`.

use crate::brain::provider::custom_openai_compatible::Utf8Carry;

#[test]
fn ascii_passes_through_unchanged() {
    let mut c = Utf8Carry::new();
    // Single complete chunk: emitted as-is.
    assert_eq!(c.push(b"hello world"), "hello world");
    // Split chunks: each emitted once, no replay.
    assert_eq!(c.push(b"hello "), "hello ");
    assert_eq!(c.push(b"world"), "world");
    assert_eq!(c.push(b""), "");
}

#[test]
fn reassembles_two_byte_char_split_across_chunks() {
    // 'í' = C3 AD, split between chunks.
    let mut c = Utf8Carry::new();
    assert_eq!(c.push(b"resum"), "resum");
    assert_eq!(c.push(&[0xC3]), "");
    assert_eq!(c.push(&[0xAD]), "í");
}

#[test]
fn reassembles_three_byte_char_split_across_chunks() {
    // '─' (box drawing) = E2 94 80, the status-table divider.
    let mut c = Utf8Carry::new();
    assert_eq!(c.push(&[0xE2]), "");
    assert_eq!(c.push(&[0x94]), "");
    assert_eq!(c.push(&[0x80]), "─");
}

#[test]
fn substitutes_genuinely_invalid_bytes_once() {
    let mut c = Utf8Carry::new();
    assert_eq!(c.push(&[0xFF, b'a']), "\u{FFFD}a");
}
