//! The correction injected when a turn claims work it never ran (#796, #797).
//!
//! The old wording stated what was missing ("your last response produced ZERO
//! tool_use blocks"), which argues against a position the model does not hold.
//! A model that believes it already ran `gh issue list` reads that as a
//! formatting complaint and keeps the belief. These pin the wording that
//! replaces it: the mechanism, and a check the model can actually apply.
//!
//! Fixtures are synthetic and carry no user identifiers.

#[cfg(feature = "telegram")]
use crate::brain::agent::service::nudge::mermaid_regen_nudge;
use crate::brain::agent::service::nudge::{no_tool_calls_nudge, uncalled_commands_nudge};

#[test]
fn a_fabricated_command_is_quoted_back() {
    // The whole point of #797: cite the fact, do not gesture at a category.
    let nudge = uncalled_commands_nudge(&["gh issue list --state open".to_string()]);
    assert!(
        nudge.contains("`gh issue list --state open`"),
        "the claimed command must appear verbatim: {nudge}"
    );
    assert!(
        nudge.contains("No such call ran this turn"),
        "the correction must state the fact plainly: {nudge}"
    );
}

#[test]
fn several_fabricated_commands_are_all_quoted() {
    let nudge = uncalled_commands_nudge(&[
        "git log --oneline -5".to_string(),
        "cargo test --all-features".to_string(),
    ]);
    assert!(nudge.contains("`git log --oneline -5`"), "{nudge}");
    assert!(nudge.contains("`cargo test --all-features`"), "{nudge}");
}

#[test]
fn the_named_variant_shares_the_mechanism_and_the_escape() {
    // It must not drift from the generic wording: same premise, same exit.
    let nudge = uncalled_commands_nudge(&["ls -la".to_string()]);
    assert!(
        nudge.contains("nothing runs inside your reasoning"),
        "{nudge}"
    );
    assert!(nudge.contains("imagined it"), "{nudge}");
    assert!(nudge.contains("genuinely done"), "{nudge}");
    assert!(nudge.starts_with("[System:"), "{nudge}");
    assert!(nudge.ends_with(']'), "{nudge}");
}

#[test]
fn every_variant_states_that_reasoning_cannot_execute() {
    // The mechanism is the load-bearing sentence. Without it the correction is
    // just a complaint about formatting, which is what failed before.
    for nudge in [no_tool_calls_nudge(true), no_tool_calls_nudge(false)] {
        assert!(
            nudge.contains("nothing runs inside your reasoning"),
            "must state the mechanism: {nudge}"
        );
        assert!(
            nudge.contains("imagined it"),
            "must name the false belief: {nudge}"
        );
    }
}

#[test]
fn every_variant_keeps_the_finished_escape() {
    // Without an exit that is not a tool call, a model that genuinely finished
    // gets nudged, calls something pointless to comply, and is nudged again.
    for nudge in [no_tool_calls_nudge(true), no_tool_calls_nudge(false)] {
        assert!(
            nudge.contains("genuinely done"),
            "real completion needs a non-tool exit: {nudge}"
        );
    }
}

#[test]
fn the_local_variant_avoids_the_word_stop() {
    // Qwen/Kimi/DeepSeek read "STOP" as "wait for further instruction" and
    // reply with an acknowledgement instead of calling anything.
    let nudge = no_tool_calls_nudge(true);
    assert!(
        !nudge.contains("STOP"),
        "local models treat STOP as an instruction to wait: {nudge}"
    );
}

#[test]
fn the_local_variant_names_the_structured_api() {
    // These models write `{"tool_call": {...}}` as message text believing that
    // IS the invocation, so the channel has to be named.
    let nudge = no_tool_calls_nudge(true);
    assert!(nudge.contains("structured tool-call API"), "{nudge}");
    assert!(
        nudge.contains("does not execute"),
        "must say that text-shaped calls do nothing: {nudge}"
    );
}

#[test]
fn a_nudge_is_framed_as_a_system_message() {
    // The loop injects these as user-role messages; the bracketed [System: ...]
    // framing is what keeps them from reading as the user's own words.
    for nudge in [no_tool_calls_nudge(true), no_tool_calls_nudge(false)] {
        assert!(nudge.starts_with("[System:"), "{nudge}");
        assert!(nudge.ends_with(']'), "{nudge}");
    }
}

// ── Mermaid regen nudge (#37) ──

#[cfg(feature = "telegram")]
#[test]
fn mermaid_regen_nudge_quotes_renderer_errors_and_counts_attempts() {
    let errors = vec!["Parse error on line 2: Lexical error".to_string()];
    let nudge = mermaid_regen_nudge(&errors, 1, 3);
    assert!(
        nudge.contains("Parse error on line 2: Lexical error"),
        "must quote the renderer's own error text: {nudge}"
    );
    assert!(nudge.contains("Regen attempt 1/3"), "{nudge}");
    assert!(
        nudge.contains("Correction rules:"),
        "must include unified correction rules: {nudge}"
    );
    assert!(nudge.contains("Mobile Layout & Aspect Ratio:"), "{nudge}");
    assert!(nudge.starts_with("[System:"), "{nudge}");
    assert!(nudge.ends_with(']'), "{nudge}");
}
