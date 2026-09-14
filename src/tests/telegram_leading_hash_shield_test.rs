//! Unit tests for leading-hash shielding on Telegram rich markdown (#193, adolfousier/opencrabs#1257).

use crate::channels::telegram::rich::{normalize_tables, shield_bare_leading_hashes};

#[test]
fn test_bare_issue_number_is_shielded() {
    let input = "#174 made callback turns write a PROCESSING row";
    let expected = "\\#174 made callback turns write a PROCESSING row";
    assert_eq!(shield_bare_leading_hashes(input), expected);
    assert_eq!(normalize_tables(input), expected);
}

#[test]
fn test_bare_issue_number_with_indent() {
    let input = "  #193 leading issue";
    let expected = "  \\#193 leading issue";
    assert_eq!(shield_bare_leading_hashes(input), expected);
}

#[test]
fn test_real_atx_headings_are_untouched() {
    let h1 = "# Real Heading";
    let h2 = "## Second Heading";
    let h6 = "###### Sixth Heading";
    assert_eq!(shield_bare_leading_hashes(h1), h1);
    assert_eq!(shield_bare_leading_hashes(h2), h2);
    assert_eq!(shield_bare_leading_hashes(h6), h6);
}

#[test]
fn test_fenced_code_blocks_are_untouched() {
    let input = "```\n#174 inside code block\n# comment\n```";
    assert_eq!(shield_bare_leading_hashes(input), input);

    let tilde = "~~~\n#174 inside tilde fence\n~~~";
    assert_eq!(shield_bare_leading_hashes(tilde), tilde);
}

#[test]
fn test_shield_is_idempotent() {
    let input = "#174 bare probe line";
    let once = shield_bare_leading_hashes(input);
    let twice = shield_bare_leading_hashes(&once);
    assert_eq!(once, "\\#174 bare probe line");
    assert_eq!(twice, "\\#174 bare probe line");
}

#[test]
fn test_multiline_mixed_prose() {
    let input = "Some intro text\n\n#174 is an issue\n# Real Heading\n\\#already escaped\n\n```\n#code\n```\n#999 final issue\n";
    let expected = "Some intro text\n\n\\#174 is an issue\n# Real Heading\n\\#already escaped\n\n```\n#code\n```\n\\#999 final issue\n";
    assert_eq!(shield_bare_leading_hashes(input), expected);
    assert_eq!(normalize_tables(input), expected);
}
