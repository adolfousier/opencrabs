//! Tests for QR code Unicode rendering.

use crate::brain::tools::whatsapp_connect::render_qr_unicode;

#[test]
fn render_qr_returns_some_for_valid_data() {
    let result = render_qr_unicode("https://example.com");
    assert!(result.is_some());
}

#[test]
fn render_qr_output_is_not_empty() {
    let qr = render_qr_unicode("test data").unwrap();
    assert!(!qr.is_empty());
}

#[test]
fn render_qr_contains_only_expected_chars() {
    let qr = render_qr_unicode("hello").unwrap();
    for ch in qr.chars() {
        assert!(
            ch == '\u{2588}' || ch == '\u{2580}' || ch == '\u{2584}' || ch == ' ' || ch == '\n',
            "unexpected character in QR output: {:?} (U+{:04X})",
            ch,
            ch as u32
        );
    }
}

#[test]
fn render_qr_line_widths_are_consistent() {
    let qr = render_qr_unicode("consistent width test").unwrap();
    let lines: Vec<&str> = qr.lines().collect();
    assert!(!lines.is_empty());
    let first_len = lines[0].chars().count();
    for (i, line) in lines.iter().enumerate() {
        assert_eq!(
            line.chars().count(),
            first_len,
            "line {} has different char width than line 0",
            i
        );
    }
}

#[test]
fn render_qr_display_width_matches_char_count() {
    // Each QR character (full block, half blocks, space) is 1 display column wide.
    // This test verifies that unicode_width agrees with char count for QR output.
    use unicode_width::UnicodeWidthStr;
    let qr = render_qr_unicode("width test").unwrap();
    for (i, line) in qr.lines().enumerate() {
        assert_eq!(
            line.width(),
            line.chars().count(),
            "line {} display width != char count (byte len: {})",
            i,
            line.len()
        );
    }
}

#[test]
fn render_qr_byte_len_exceeds_char_count() {
    // Unicode block chars are 3 bytes each in UTF-8 but 1 char.
    // This confirms the old .len() approach would have been wrong for width calculation.
    let qr = render_qr_unicode("byte length test").unwrap();
    let first_line = qr.lines().next().unwrap();
    assert!(
        first_line.len() > first_line.chars().count(),
        "byte len ({}) should exceed char count ({}) due to multi-byte Unicode",
        first_line.len(),
        first_line.chars().count()
    );
}

#[test]
fn render_qr_has_quiet_zone() {
    // QR spec requires a quiet zone (white border). The first and last lines
    // should be all full-block characters (light = white border).
    let qr = render_qr_unicode("quiet zone test").unwrap();
    let lines: Vec<&str> = qr.lines().collect();
    let first = lines.first().unwrap();
    let last = lines.last().unwrap();
    // Quiet zone rows should be all full blocks (light modules) or half blocks
    // where paired with the next row. At minimum, first char should be full block.
    assert!(
        first.starts_with('\u{2588}'),
        "first line should start with full block (quiet zone)"
    );
    assert!(
        last.contains('\u{2588}'),
        "last line should contain full blocks (quiet zone)"
    );
}

#[test]
fn render_qr_different_data_produces_different_output() {
    let qr1 = render_qr_unicode("data one").unwrap();
    let qr2 = render_qr_unicode("data two").unwrap();
    assert_ne!(qr1, qr2);
}
