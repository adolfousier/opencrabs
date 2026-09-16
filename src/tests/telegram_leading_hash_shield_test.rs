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

#[test]
fn test_list_items_with_leading_hashes_are_shielded() {
    // Unordered dash bullets (#243)
    let dash = "- #224 -> a5b34466 (Goal turn budget progress)\n- #236 -> 2ed8adeb (Strip tool outcome icons)";
    let exp_dash = "- \\#224 -> a5b34466 (Goal turn budget progress)\n- \\#236 -> 2ed8adeb (Strip tool outcome icons)";
    assert_eq!(shield_bare_leading_hashes(dash), exp_dash);
    assert_eq!(normalize_tables(dash), exp_dash);

    // Unordered asterisk & plus bullets
    let star = "* #237 cron+notify\n+ #238 align tests";
    let exp_star = "* \\#237 cron+notify\n+ \\#238 align tests";
    assert_eq!(shield_bare_leading_hashes(star), exp_star);

    // Indented unordered bullets
    let indented = "  - #239 table separator\n    * #240 fence balance";
    let exp_indented = "  - \\#239 table separator\n    * \\#240 fence balance";
    assert_eq!(shield_bare_leading_hashes(indented), exp_indented);
}

#[test]
fn test_ordered_list_items_with_leading_hashes_are_shielded() {
    let ordered = "1. #224 first item\n2) #225 second item\n12. #226 multi digit item";
    let exp_ordered = "1. \\#224 first item\n2) \\#225 second item\n12. \\#226 multi digit item";
    assert_eq!(shield_bare_leading_hashes(ordered), exp_ordered);
    assert_eq!(normalize_tables(ordered), exp_ordered);
}

#[test]
fn test_checkbox_items_with_leading_hashes_are_shielded() {
    let checkboxes =
        "- [ ] #241 background sweep\n- [x] #242 closed task\n  * [ ] #243 nested task";
    let exp_checkboxes =
        "- [ ] \\#241 background sweep\n- [x] \\#242 closed task\n  * [ ] \\#243 nested task";
    assert_eq!(shield_bare_leading_hashes(checkboxes), exp_checkboxes);
    assert_eq!(normalize_tables(checkboxes), exp_checkboxes);
}

#[test]
fn test_blockquotes_with_leading_hashes_are_shielded() {
    let quotes = "> #243 quote issue\n>> #244 nested quote issue\n> - #245 quote list issue";
    let exp_quotes =
        "> \\#243 quote issue\n>> \\#244 nested quote issue\n> - \\#245 quote list issue";
    assert_eq!(shield_bare_leading_hashes(quotes), exp_quotes);
    assert_eq!(normalize_tables(quotes), exp_quotes);
}

#[test]
fn test_headings_and_inline_hashes_in_lists_preserved() {
    // Valid ATX headings in or out of lists should remain intact
    let heading_in_list = "- # Heading in list";
    assert_eq!(shield_bare_leading_hashes(heading_in_list), heading_in_list);

    // Mid-line issue numbers should not be touched
    let inline_hash = "- Fixed issue #224 in commit abc";
    assert_eq!(shield_bare_leading_hashes(inline_hash), inline_hash);
}
