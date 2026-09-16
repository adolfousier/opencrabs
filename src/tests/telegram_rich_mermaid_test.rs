//! Tests for Telegram rich-text mermaid fence location.
//!
//! Extracted from an inline `#[cfg(test)]` block in
//! `src/channels/telegram/rich/mermaid.rs`; project policy
//! (CONTRIBUTING.md) requires all tests under `src/tests/`.

use crate::channels::telegram::rich::mermaid::{find_mermaid_fences, has_mermaid_fence};

const DIAGRAM: &str = "flowchart TD\n    A --> B";

#[test]
fn clean_tagged_block_locates() {
    let text = format!("```mermaid\n{DIAGRAM}\n```\nafter");
    assert!(has_mermaid_fence(&text));
    let fences = find_mermaid_fences(&text);
    assert_eq!(fences.len(), 1);
    assert_eq!(fences[0].source.trim(), DIAGRAM);
}

#[test]
fn stray_bare_fence_before_tagged_no_longer_desyncs() {
    // 14:23Z bug class: the stray bare opener used to pair with the
    // ```mermaid line as its CLOSER, desyncing the machine so the real
    // diagram never located and the raw fence shipped.
    let text = format!("```\n```mermaid\n{DIAGRAM}\n```\nafter");
    assert!(has_mermaid_fence(&text));
    let fences = find_mermaid_fences(&text);
    assert_eq!(
        fences.len(),
        1,
        "tagged diagram must locate past a stray bare opener"
    );
    assert_eq!(fences[0].source.trim(), DIAGRAM);
    // The range must cover the diagram fence, not swallow the next line.
    let replaced = text[..fences[0].start].to_string() + &text[fences[0].end..];
    assert!(
        replaced.contains("after"),
        "trailing text must survive the swap"
    );
}

#[test]
fn two_clean_blocks_both_locate() {
    let text = "```mermaid\nflowchart TD\n    A --> B\n```\ntext\n```mermaid\nflowchart LR\n    C --> D\n```\n";
    let fences = find_mermaid_fences(text);
    assert_eq!(fences.len(), 2);
    assert!(fences[0].source.contains("A --> B"));
    assert!(fences[1].source.contains("C --> D"));
}

#[test]
fn non_mermaid_untagged_block_ignored() {
    let text = "```\njust some text\n```";
    assert!(!has_mermaid_fence(text));
    assert!(find_mermaid_fences(text).is_empty());
}
