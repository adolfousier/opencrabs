//! Tests the `browser_eval` output size cap — pins the truncation
//! behaviour so large `document.body.outerHTML` dumps don't burn the
//! LLM's entire context window on one tool result.

use crate::brain::tools::browser::cap_eval_output;

#[test]
fn passes_through_small_output_unchanged() {
    let small = "some small output".to_string();
    assert_eq!(cap_eval_output(small.clone()), small);
}

#[test]
fn truncates_oversized_output_and_notes_the_original_size() {
    let huge = "a".repeat(100_000);
    let capped = cap_eval_output(huge);
    assert!(
        capped.len() < 100_000,
        "output must shrink, but is {} bytes",
        capped.len()
    );
    assert!(
        capped.contains("truncated"),
        "note must tell the model the result was trimmed"
    );
    assert!(
        capped.contains("100000"),
        "note must include the original byte count so the model can reason about it"
    );
}

#[test]
fn truncation_lands_on_a_char_boundary_for_multibyte_utf8() {
    // Build a string ~60 KB of a 4-byte emoji. Simple ascii-byte
    // truncation would split the emoji mid-sequence and panic at
    // str indexing. The cap helper must walk backwards to a valid
    // char boundary.
    let emoji_count = 16_000;
    let big = "🦀".repeat(emoji_count); // each 🦀 is 4 bytes
    assert!(big.len() > 50_000);
    let capped = cap_eval_output(big); // must not panic
    // Capped body must itself be valid UTF-8 and contain only whole
    // emoji — easiest check: the crab-count in the prefix divides
    // evenly.
    let prefix_end = capped.find("\n\n[truncated").unwrap_or(capped.len());
    let prefix = &capped[..prefix_end];
    assert!(prefix.is_char_boundary(prefix.len()));
    assert_eq!(prefix.len() % 4, 0, "truncation sliced a 🦀 in half");
}

#[test]
fn empty_output_passes_through() {
    assert_eq!(cap_eval_output(String::new()), "");
}

#[test]
fn exact_cap_boundary_passes_through_unchanged() {
    // Output exactly the cap size shouldn't trigger truncation —
    // off-by-one at the boundary would emit a pointless "truncated
    // to 50000 of 50000 bytes" note.
    let at_cap = "a".repeat(50_000);
    let capped = cap_eval_output(at_cap.clone());
    assert_eq!(capped, at_cap);
}
