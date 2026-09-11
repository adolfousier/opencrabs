//! A report's facts must exist somewhere in the conversation (#1423).
//!
//! The post-success exemption lets a text-only iteration through once the turn
//! has real work behind it, on the reasoning that `Pushed.` is an
//! acknowledgement rather than a claim. That holds for an ack and fails for a
//! report. Observed 2026-09-07 01:35:59: 24 successful tool calls, then a
//! 4,664-character iteration exempted as a "pure completion acknowledgement"
//! while asserting test tallies and commit shas that no tool had printed.
//!
//! Length cannot discriminate. Across 128 exempted iterations over three days
//! the median is 968 characters and p90 is 2,563, so most exempted text is
//! legitimately long and a size bound would re-break the regression the
//! exemption was added for. What separates a report from a fabrication is
//! whether the facts in it exist.
//!
//! `claims_unbacked_evidence` was the branch that should have caught it and is
//! shape-gated: it needs three lines shaped like a grep dump, an `===` frame,
//! padded key-value or a column row. A markdown table with prose between the
//! rows matches none of them, which is how the fabrication walked through.
//!
//! Fixtures are synthetic and carry no user identifiers.

use crate::brain::agent::service::nudge::unbacked_facts_nudge;
use crate::brain::agent::service::phantom::{asserted_facts, unbacked_facts};

const TOOL_LOOP_SRC: &str = include_str!("../brain/agent/service/tool_loop.rs");

/// The shape that actually got through: a markdown table and prose tallies,
/// with an invented sha. No line here matches `is_evidence_line`.
const FABRICATED_REPORT: &str = "Merges are in and committed local.\n\
     \n\
     | Check | Result |\n\
     |---|---|\n\
     | `cargo fmt --all -- --check` | clean |\n\
     | clippy, CI flags, `-D warnings` | clean |\n\
     \n\
     Suite green: 7,896 passed, 0 failed, 30 ignored.\n\
     Committed as 41b29f23 and 531a2f62, docs reconciled in 7ff3bc26.\n";

/// Evidence from an unrelated run: different tallies, different ids, and no
/// `0` character anywhere, so even the weakest tally is unbacked.
const UNRELATED_EVIDENCE: &str = "test result: ok. 111 passed; 1 failed; 2 ignored\n\
     abcd1234 some unrelated commit\n";

/// Evidence a real turn of that shape would have produced.
const REAL_EVIDENCE: &str = "test result: ok. 7896 passed; 0 failed; 30 ignored\n\
     957679d9 Merge PR #1422\n\
     22b0615e Merge PR #1418\n";

// ─── extraction ──────────────────────────────────────────────

#[test]
fn a_report_shaped_fabrication_yields_its_invented_facts() {
    let facts = asserted_facts(FABRICATED_REPORT);
    // The two tallies and the three shas, all of them absent from real output.
    assert!(facts.iter().any(|f| f == "7,896 passed"), "{facts:?}");
    assert!(facts.iter().any(|f| f == "30 ignored"), "{facts:?}");
    assert!(facts.iter().any(|f| f == "41b29f23"), "{facts:?}");
    assert!(facts.iter().any(|f| f == "531a2f62"), "{facts:?}");
    assert!(facts.iter().any(|f| f == "7ff3bc26"), "{facts:?}");
}

#[test]
fn every_extracted_fact_is_checked_against_evidence_not_wording() {
    let facts = asserted_facts(FABRICATED_REPORT);
    assert!(!facts.is_empty());
    // Nothing in the evidence backs any of them, so all are unbacked. This is
    // the assertion the exemption was missing: detection here does not read a
    // verb, a layout or a length.
    assert_eq!(
        unbacked_facts(&facts, UNRELATED_EVIDENCE).len(),
        facts.len(),
        "some invented fact counted as backed: {:?}",
        unbacked_facts(&facts, UNRELATED_EVIDENCE)
    );
}

#[test]
fn the_same_report_with_real_facts_is_not_flagged() {
    // The anti-regression half. A legitimate wrap-up of the same length and
    // shape, quoting what the tools actually printed, must stay exempt. If
    // this fires the check has re-broken #785.
    let honest = "Merges are in and committed local.\n\
          \n\
          | Check | Result |\n\
          |---|---|\n\
          | `cargo fmt --all -- --check` | clean |\n\
          \n\
          Suite green: 7896 passed, 0 failed, 30 ignored.\n\
          Committed as 957679d9 and 22b0615e.\n";
    let facts = asserted_facts(honest);
    assert!(!facts.is_empty(), "the honest report asserts facts too");
    assert!(
        unbacked_facts(&facts, REAL_EVIDENCE).is_empty(),
        "backed facts flagged: {:?}",
        unbacked_facts(&facts, REAL_EVIDENCE)
    );
}

#[test]
fn a_grouped_tally_is_backed_by_plain_output_and_the_reverse() {
    // A report quotes `7,926`; the run printed `7926`. Neither spelling is a
    // fabrication, so both directions have to count as backed.
    assert!(unbacked_facts(&["7,926 passed".to_string()], "7926 tests").is_empty());
    assert!(unbacked_facts(&["7926 passed".to_string()], "total: 7,926").is_empty());
    assert_eq!(
        unbacked_facts(&["7,926 passed".to_string()], "7896 passed"),
        vec!["7,926 passed".to_string()]
    );
}

#[test]
fn ordinary_prose_and_numbers_yield_no_candidates() {
    for text in [
        "Done.",
        "Pushed to origin.",
        "Version 0.5.0 is out and issue #1423 covers it.",
        "The run took 101.74s across 800 files on 2026-09-07.",
        "Nothing to do here, the tree is clean.",
        "",
    ] {
        assert!(
            asserted_facts(text).is_empty(),
            "{text:?} produced {:?}",
            asserted_facts(text)
        );
    }
}

#[test]
fn hex_shaped_english_words_are_not_shas() {
    // Requiring both a digit and a letter is what keeps prose out. Each of
    // these is a maximal hex run of 7+ characters.
    for text in [
        "The defaced sign was effaced by rain.",
        "A decaf beverage and some face cream.",
    ] {
        assert!(
            asserted_facts(text).is_empty(),
            "{text:?} produced {:?}",
            asserted_facts(text)
        );
    }
}

#[test]
fn uuid_segments_and_paths_are_not_shas() {
    // Hyphen-adjacent runs are uuid segments, and a run inside a longer word
    // is not standing alone. Both would otherwise fire on every session id
    // quoted in a report.
    let text = "session fd72101f-c667-40ab-af04-91936acbcc54 and target/debug/x9a1b2c3d";
    assert!(
        asserted_facts(text).is_empty(),
        "produced {:?}",
        asserted_facts(text)
    );
}

#[test]
fn an_uppercase_sha_is_normalised_to_what_git_prints() {
    let facts = asserted_facts("Committed as 957679D9 just now.");
    assert_eq!(facts, vec!["957679d9".to_string()]);
    // So evidence printed by git backs it.
    assert!(unbacked_facts(&facts, "957679d9 Merge PR #1422").is_empty());
}

#[test]
fn a_repeated_fact_is_reported_once_and_the_list_is_bounded() {
    let repeated = "Committed as 957679d9. Again 957679d9. And 957679d9 once more.";
    assert_eq!(asserted_facts(repeated), vec!["957679d9".to_string()]);

    // A pathological iteration must not produce an unbounded nudge. The cap is
    // asserted as a bound, not as a literal, so tightening it does not break
    // this test.
    let many = (0..40)
        .map(|i| format!("sha {i:06x}a1b2c3 committed"))
        .collect::<Vec<_>>()
        .join(", ");
    let facts = asserted_facts(&many);
    assert!(!facts.is_empty());
    assert!(facts.len() < 40, "not bounded: {}", facts.len());
}

#[test]
fn the_keyword_has_to_be_word_bounded() {
    // `bypassed` contains `passed`; a tally needs a number in front of a
    // standalone keyword.
    assert!(asserted_facts("The check was bypassed entirely.").is_empty());
    assert!(asserted_facts("It bypassed 12 guards.").is_empty());
}

// ─── the nudge names the fact, not the category (#797) ────────

#[test]
fn the_nudge_quotes_the_invented_tokens() {
    let nudge = unbacked_facts_nudge(&["41b29f23".to_string(), "7,896 passed".to_string()]);
    assert!(nudge.contains("`41b29f23`"), "{nudge}");
    assert!(nudge.contains("`7,896 passed`"), "{nudge}");
    assert!(nudge.contains("written, not read"), "{nudge}");
    // Two facts read as a plural list.
    assert!(nudge.contains("these facts:"), "{nudge}");
    let single = unbacked_facts_nudge(&["41b29f23".to_string()]);
    assert!(single.contains("this fact:"), "{single}");
}

// ─── wiring sentinels ────────────────────────────────────────

#[test]
fn eligibility_consults_the_fact_check() {
    // Without this the exemption stays content-blind: eligibility is what the
    // post-success skip is decided from.
    let gate = TOOL_LOOP_SRC
        .split("let phantom_eligible = !is_cli_provider")
        .nth(1)
        .expect("phantom_eligible gate not found");
    // Split on the next statement rather than on `;`, which appears inside the
    // chain's own comments and would truncate the gate before its last branch.
    let gate = gate
        .split("// Analytics (#897): if a phantom was detected earlier this turn")
        .next()
        .expect("unterminated gate");
    assert!(
        gate.contains("|| !unbacked_facts.is_empty()"),
        "the eligibility chain does not consult unbacked_facts"
    );
}

#[test]
fn the_detection_block_fires_on_unbacked_facts() {
    // Eligibility alone only un-skips the logging; the detection block is what
    // increments the retry budget and injects the nudge. Since #1506 the
    // block collects named `fired_branches` and gates the kill on the list,
    // so the sentinel anchors on that collection and scans to the budget
    // increment (the old `if phantom_retries_used < ...` header is gone).
    let (_, rest) = TOOL_LOOP_SRC
        .split_once("let fired_branches: Vec<&str> = {")
        .expect("detection block not found");
    let (block, _) = rest
        .split_once("phantom_detections_total += 1;")
        .expect("detection block never increments the retry budget");
    assert!(
        block.contains(r#"("unbacked_facts", !unbacked_facts.is_empty())"#),
        "the detection block does not fire on unbacked_facts"
    );
    assert!(
        block.contains("!fired_branches.is_empty()"),
        "the kill gate does not consult the fired-branch list"
    );
}

#[test]
fn the_fact_check_is_gated_to_the_post_success_regime() {
    // On a zero-tool turn eligibility is already true, so scanning adds cost
    // without coverage. The gate is also what keeps the haystack off the
    // common path.
    let gate = TOOL_LOOP_SRC
        .split("let unbacked_facts = ")
        .nth(1)
        .expect("unbacked_facts computation not found");
    let gate = gate.split("let phantom_eligible").next().unwrap();
    assert!(
        gate.contains("if tool_calls_completed_this_turn > 0"),
        "the fact check is not gated post-success"
    );
    assert!(
        gate.contains("asserted.is_empty()"),
        "the haystack is built even when nothing was asserted"
    );
    assert!(
        gate.contains("Self::conversation_evidence"),
        "the evidence haystack is not consulted"
    );
}

#[test]
fn the_evidence_haystack_excludes_the_models_own_text() {
    // The load-bearing exclusion. If assistant text counted as evidence, a sha
    // invented three iterations ago would vouch for itself on the fourth and
    // the check would go quiet on exactly the case it exists for.
    let helper = TOOL_LOOP_SRC
        .split("fn conversation_evidence(")
        .nth(1)
        .expect("conversation_evidence not found");
    let helper = helper.split("\n    async fn ").next().unwrap();
    assert!(
        helper.contains("ContentBlock::ToolResult"),
        "tool output is not evidence"
    );
    assert!(
        helper.contains("ContentBlock::ToolUse"),
        "what was actually run is not evidence"
    );
    assert!(
        helper.contains("if msg.role == Role::User"),
        "text is admitted regardless of role, so assistant claims self-vouch"
    );
    assert!(
        !helper.contains("ContentBlock::Thinking"),
        "reasoning must not vouch for the report it precedes"
    );
}

#[test]
fn the_nudge_selection_names_facts_when_there_are_no_commands() {
    let sel = TOOL_LOOP_SRC
        .split("let nudge = if !uncalled_commands.is_empty()")
        .nth(1)
        .expect("nudge selection not found");
    let sel = sel
        .split("context.add_message(Message::user(nudge));")
        .next()
        .unwrap();
    assert!(
        sel.contains("unbacked_facts_nudge(&unbacked_facts)"),
        "an invented fact falls through to the generic wording"
    );
    assert!(
        sel.contains("no_tool_calls_nudge(is_local_provider)"),
        "the generic fallback was lost"
    );
}
