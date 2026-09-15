//! Unit tests for inferring missing GFM table separators (#239).

use crate::channels::telegram::rich::{
    contains_table, infer_missing_table_separators, normalize_tables,
};

#[test]
fn table_without_separator_gets_delimiter_injected() {
    let input = "Col A | Col B\nVal 1 | Val 2\nVal 3 | Val 4";
    let expected = "Col A | Col B\n|---|---|\nVal 1 | Val 2\nVal 3 | Val 4";
    assert_eq!(infer_missing_table_separators(input), expected);
}

#[test]
fn roadmap_5_column_table_without_separator_is_normalized_and_detected() {
    // Exact shape from the forwarded roadmap: 5 columns without separator line
    let input = "Product | Producing Process | Owner | Client & Delivered Value | Status\n\
Factory Template & Add-on Packs | Process 1: Work Delivery Pipeline | hq | Client: New Factory Operators | 🟢 HEALTHY\n\
Consulting Practice & Advisories | Process 3: Operational Measurement | surveys | Client: Member Factory HQs | 🟢 HEALTHY";

    assert!(
        contains_table(input),
        "contains_table must detect table with missing separator via normalize_tables"
    );

    let normalized = normalize_tables(input);
    assert!(
        normalized.contains("|---|---|---|---|---|"),
        "normalized text must contain 5-column synthesized separator: {normalized}"
    );
}

#[test]
fn existing_valid_table_is_untouched() {
    let input = "| Check | Result |\n|---|---|\n| uptime | ok |";
    assert_eq!(infer_missing_table_separators(input), input);
}

#[test]
fn table_in_code_fence_is_untouched() {
    let input = "```text\nCol A | Col B\nVal 1 | Val 2\n```";
    assert_eq!(infer_missing_table_separators(input), input);

    let tilde_input = "~~~markdown\nCol A | Col B\nVal 1 | Val 2\n~~~";
    assert_eq!(infer_missing_table_separators(tilde_input), tilde_input);
}

#[test]
fn single_pipe_prose_is_untouched() {
    let input = "Option 1 | Option 2\nThis is just a normal sentence following it.";
    assert_eq!(infer_missing_table_separators(input), input);
}

#[test]
fn infer_separator_is_idempotent() {
    let input = "Header 1 | Header 2\nRow 1 | Row 2\nRow 3 | Row 4";
    let once = infer_missing_table_separators(input);
    let twice = infer_missing_table_separators(&once);
    assert_eq!(once, twice);
}

#[test]
fn normalize_tables_inserts_blank_line_before_inferred_table() {
    let input = "## Roadmap Section\nProduct | Owner | Status\nTemplate | hq | Active\nAdvisory | ops | Active";
    let normalized = normalize_tables(input);
    let expected = "## Roadmap Section\n\nProduct | Owner | Status\n|---|---|---|\nTemplate | hq | Active\nAdvisory | ops | Active";
    assert_eq!(normalized, expected);
}
