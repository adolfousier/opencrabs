//! Rule size cap and the narrow consolidation path (#857, #858).
//!
//! RSI wrote rules as paragraphs into always-loaded files, so every byte was
//! paid on every turn of every channel, permanently. 645 of 722 improvements
//! target AGENTS.md or SOUL.md, and one application added 1,329 bytes.
//!
//! It also could not consolidate: rewording a rule shorter is a shrink, and
//! append-only rejects shrinks unless every removed byte reappears, which
//! rewording never satisfies. So the files could only grow, and 51 entries are
//! RSI manually removing its own duplicates.
//!
//! The consolidation allowance is deliberately narrow. The general bypass it
//! replaces was used to delete 47% of SOUL.md in one pass, so these tests weigh
//! toward what it must REFUSE.
//!
//! Fixtures are synthetic and carry no user identifiers.

use crate::brain::tools::brain_file_safety::{
    MAX_CONSOLIDATION_SHRINK, MAX_RULE_CHARS, ShrinkCheck, check_no_shrink, is_rule_consolidation,
};
use std::path::PathBuf;

fn protected() -> PathBuf {
    let mut p = crate::config::opencrabs_home();
    p.push("SOUL.md");
    p
}

/// The essay form, sized like the real thing: rules of ~1,300 characters with
/// their incident histories inline are what RSI was actually writing into the
/// always-loaded files.
const LONG_RULE: &str = "**Verify file paths first.** Never guess a path. Use ls or glob before \
     any read, edit or grep. Incident 1: a wrong path was used and the agent continued for \
     several minutes without listing the parent directory, which would have resolved it \
     immediately. Incident 2: another wrong path was used in the same session, after the rule \
     had already been written, showing that restating it inline does not prevent recurrence. \
     Incident 3: a server path was used against the local filesystem, where it does not exist, \
     and the failure was misread as a missing file rather than a wrong root. Incident 4: the \
     agent assumed a directory layout from a project name instead of confirming it, then \
     issued three reads against paths that were never checked. Incident 5: a path was inferred \
     from a previous session rather than verified in the current one, and the parent directory \
     had since been renamed, so every subsequent operation in that turn failed for the same \
     reason without the cause being identified.";

const TIGHT_RULE: &str = "**Verify file paths first.** Never guess. Use ls/glob first. \
     Violations: 10+.";

// ── The cap ─────────────────────────────────────────────────────────────────

#[test]
fn the_essay_form_is_rejected_by_the_budget() {
    // The concrete pair that motivates the cap: the paragraph form of a rule
    // must not fit, or the cap changes nothing. Asserted against real text
    // rather than against the constant, which clippy correctly calls a
    // constant assertion.
    assert!(
        LONG_RULE.chars().count() > MAX_RULE_CHARS,
        "the essay fixture ({}) no longer exceeds MAX_RULE_CHARS ({MAX_RULE_CHARS}) — \
         either the cap was raised past usefulness or the fixture shrank",
        LONG_RULE.chars().count()
    );
}

#[test]
fn a_real_condensed_rule_fits_the_budget() {
    // Taken from the shape rules were manually condensed to. The cap is only
    // fair if the short form actually fits.
    assert!(TIGHT_RULE.chars().count() <= MAX_RULE_CHARS);
}

// ── Consolidation: what it allows ───────────────────────────────────────────

#[test]
fn tightening_a_rule_keeps_its_identifier_and_qualifies() {
    assert!(is_rule_consolidation(LONG_RULE, TIGHT_RULE));
}

#[test]
fn a_heading_identifier_also_qualifies() {
    let old = "## Respond First, Investigate Second\n\nA long explanation follows here.";
    let new = "## Respond First, Investigate Second\n\nReply first.";
    assert!(is_rule_consolidation(old, new));
}

#[test]
fn a_bounded_consolidation_is_allowed_through_the_gate() {
    let existing = format!("intro\n\n{LONG_RULE}\n\ntail");
    let updated = format!("intro\n\n{TIGHT_RULE}\n\ntail");
    assert_eq!(
        check_no_shrink(&protected(), &existing, &updated, false, true),
        ShrinkCheck::Allowed
    );
}

// ── Consolidation: what it must refuse ──────────────────────────────────────

#[test]
fn deleting_a_rule_outright_is_not_consolidation() {
    // The failure mode this exists to prevent.
    assert!(!is_rule_consolidation(LONG_RULE, ""));
    assert!(!is_rule_consolidation(LONG_RULE, "   \n  "));
}

#[test]
fn replacing_a_rule_with_a_different_one_is_not_consolidation() {
    // Losing the identifier means the rule did not survive, whatever the
    // replacement says.
    let unrelated = "**Always use cargo clippy.** Not cargo check.";
    assert!(!is_rule_consolidation(LONG_RULE, unrelated));
}

#[test]
fn text_with_no_identifiable_rule_cannot_be_consolidated() {
    // Without an identifier there is nothing to prove survived, so the safe
    // answer is no.
    let prose = "Some paragraph of ordinary text with no directive in it at all, going on \
                 for a while so length is not what disqualifies it.";
    assert!(!is_rule_consolidation(prose, "Shorter prose."));
}

#[test]
fn a_short_bold_span_is_not_treated_as_an_identifier() {
    // `**No.**` matching loosely would let almost any replacement qualify.
    let old = "**No.** Followed by a long explanation that goes on at some length here.";
    assert!(!is_rule_consolidation(old, "**No.**"));
}

#[test]
fn the_byte_cap_stops_a_consolidation_claim_from_deleting_a_section() {
    // Even a TRUE consolidation claim cannot remove more than one rule's worth.
    // This is what bounds the blast radius, independent of the caller's claim.
    let existing = format!("{}\n{LONG_RULE}", "filler line\n".repeat(400));
    let updated = TIGHT_RULE.to_string();
    let removed = existing.len() - updated.len();
    assert!(
        removed > MAX_CONSOLIDATION_SHRINK,
        "fixture must exceed the cap to be meaningful"
    );
    assert!(matches!(
        check_no_shrink(&protected(), &existing, &updated, false, true),
        ShrinkCheck::Rejected { .. }
    ));
}

#[test]
fn without_the_consolidation_flag_append_only_still_governs() {
    let existing = format!("intro\n\n{LONG_RULE}\n\ntail");
    let updated = format!("intro\n\n{TIGHT_RULE}\n\ntail");
    assert!(matches!(
        check_no_shrink(&protected(), &existing, &updated, false, false),
        ShrinkCheck::Rejected { .. }
    ));
}

#[test]
fn growth_is_unaffected_by_any_of_this() {
    let existing = TIGHT_RULE.to_string();
    let updated = format!("{TIGHT_RULE}\n\n{LONG_RULE}");
    for consolidation in [true, false] {
        assert_eq!(
            check_no_shrink(
                &protected(),
                &existing,
                &updated,
                false,
                consolidation
            ),
            ShrinkCheck::Allowed
        );
    }
}

#[test]
fn unprotected_files_are_untouched() {
    let mut p = crate::config::opencrabs_home();
    p.push("scratch.txt");
    assert_eq!(
        check_no_shrink(&p, LONG_RULE, "x", false, false),
        ShrinkCheck::Allowed
    );
}
