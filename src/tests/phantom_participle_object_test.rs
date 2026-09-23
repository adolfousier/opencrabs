//! #1694 — a marker-less bare-participle announcement carrying its own object
//! pronoun.
//!
//! The live escape: a turn that emitted no tool call ended on
//!
//! > Reading them, with mtimes so I know each postdates the tree it claims to cover.
//!
//! which is an announcement of unfinished work with nothing to anchor it to:
//! `work_announcement_re` and `gerund_re` require a trailing now / … / :, and
//! `plan_announcement_re` requires then / before / after. All three missed it.
//!
//! The hard half is NOT the construction — it is separating the announcement
//! from the participial SUBJECT, which is the same surface form:
//! "Reading them is straightforward." / "Getting them took a while." No copula
//! list can do it, because the statement above that lacks a copula is still a
//! statement. What separates them is the WORD CLASS after the pronoun, which is
//! closed, whereas the verb list that would be needed instead is open and never
//! converges (#1122).
//!
//! Every case here mirrors `~/.opencrabs/projects/opencrabs/files/
//! validate_1694_v4.py`, which was run to 33/33 before any Rust was written.

use crate::brain::agent::service::phantom::{
    has_phantom_tool_intent, has_phantom_tool_intent_no_tools, matches_participle_object,
};
use crate::brain::agent::service::phantom_lang::config::{
    LANG_EN, LANG_ES, LANG_FR, LANG_ID, LANG_PT, LANG_RU,
};

fn en(t: &str) -> bool {
    matches_participle_object(t)
}

// ── English ────────────────────────────────────────────────────────────────

#[test]
fn the_incident_fires() {
    assert!(en(
        "Reading them, with mtimes so I know each postdates the tree it claims to cover."
    ));
}

#[test]
fn a_closed_class_tail_is_an_announcement() {
    for text in [
        "Checking them against the log before I trust either.",
        "Both receipts landed. Reading them now…",
        "Reading them one by one.",
        "Verifying them first.",
        "Reading them carefully.",
    ] {
        assert!(en(text), "should fire: {text}");
    }
}

#[test]
fn a_participial_subject_is_not_an_announcement() {
    for text in [
        "Reading them is straightforward.",
        "Reading the file is straightforward.",
        "Getting them took a while.",
        "Checking them revealed the gap.",
        "Loading them seemed pointless.",
        "Reading them carefully takes a while.",
    ] {
        assert!(!en(text), "should NOT fire: {text}");
    }
}

#[test]
fn non_verb_ing_stems_are_filtered_out() {
    // The stem rule is closed-set (>=3 chars, not ending a/h/r), so these
    // ordinary words never become gerund announcements.
    for text in [
        "Bring them the diff and I'll look.",
        "During them I meant to check.",
        "Something it could do is matter.",
        "Spring them a new branch.",
    ] {
        assert!(!en(text), "should NOT fire: {text}");
    }
}

#[test]
fn a_recap_of_completed_reading_does_not_fire() {
    assert!(!en("I finished reading them."));
}

// ── Portuguese (enclitic behind a hyphen) ─────────────────────────────────

#[test]
fn pt_enclitic_announcements_fire() {
    for text in [
        "Lendo-os com atenção, anoto as discrepâncias.",
        "Verificando-os um a um.",
        "Lendo-os rapidamente.",
    ] {
        assert!(matches_participle_object(text), "should fire: {text}");
    }
}

#[test]
fn pt_participial_subjects_do_not_fire() {
    for text in [
        "Lendo-os é o primeiro passo.",
        "Lendo-os revelou o problema.",
        "Lendo-os rapidamente revelou o problema.",
    ] {
        assert!(!matches_participle_object(text), "should NOT fire: {text}");
    }
}

// ── Spanish (fused enclitic, accent shift) ────────────────────────────────

#[test]
fn es_fused_enclitic_announcements_fire() {
    // Leyéndolos / Corriéndolos / Siguiéndolos carry é; a flat `endo`
    // alternation matches none of them, which is why the accented forms are
    // in the pattern.
    for text in [
        "Leyéndolos con calma, encuentro tres diferencias.",
        "Verificándolos uno por uno.",
        "Corriéndolos antes de salir.",
        "Siguiéndolos paso a paso.",
        "Entendiéndolos mejor, cambio el enfoque.",
        "Leyéndolos rápidamente.",
        "Haciéndolos ahora.",
    ] {
        assert!(matches_participle_object(text), "should fire: {text}");
    }
}

#[test]
fn es_participial_subjects_do_not_fire() {
    for text in [
        "Leyéndolos no es difícil.",
        "Leyéndolos reveló el fallo.",
        "Leyéndolos rápidamente reveló el fallo.",
    ] {
        assert!(!matches_participle_object(text), "should NOT fire: {text}");
    }
}

// ── The arms that stay inert, enforced rather than assumed ────────────────

#[test]
fn the_three_ship_languages_carry_the_arm() {
    for (name, lang) in [("en", &*LANG_EN), ("pt", &*LANG_PT), ("es", &*LANG_ES)] {
        assert!(
            !lang.participle_object_re.is_empty(),
            "{name}: participle_object_re must be configured"
        );
        assert!(
            !lang.announcement_tail_words.is_empty(),
            "{name}: announcement_tail_words must be configured — an empty \
             closed class would reject every tail"
        );
    }
}

#[test]
fn fr_id_ru_are_inert_on_purpose() {
    // Not an oversight: French « en les lisant » is idiomatically an adverbial
    // of means, and id/ru have no present-tense copula for the tail rule to
    // bite on. A guessed arm there would nag legitimate replies, which is the
    // #1506 harm. Asserted so that a future addition is a deliberate change to
    // this test, not an accident.
    for (name, lang) in [("fr", &*LANG_FR), ("id", &*LANG_ID), ("ru", &*LANG_RU)] {
        assert!(
            lang.participle_object_re.is_empty(),
            "{name}: arm is deliberately absent (#1694) — see its TOML header"
        );
    }
}

// ── Wiring ────────────────────────────────────────────────────────────────

#[test]
fn the_zero_tool_detector_fires_on_the_incident() {
    // The arm only matters if the detector that actually gates delivery
    // reaches it.
    assert!(has_phantom_tool_intent_no_tools(
        "Both receipts landed. Reading them, with mtimes so I know each postdates the tree."
    ));
}

#[test]
fn the_detector_does_not_fire_on_an_ordinary_answer() {
    assert!(!has_phantom_tool_intent_no_tools(
        "Both receipts landed. Reading them is straightforward, so here is the summary: \
         clippy exited zero and 343 tests passed."
    ));
}

#[test]
fn the_arm_is_zero_tool_only() {
    // It must be reachable from the no-tools detector and from NOTHING else:
    // after a real call the shape is a legitimate recap (#1506/#1172).
    let src = include_str!("../brain/agent/service/phantom.rs");
    let hits = src.matches("matches_participle_object(").count();
    assert_eq!(
        hits, 2,
        "expected exactly the definition and the one call inside \
         has_phantom_tool_intent_no_tools, found {hits}"
    );
    let call = src
        .find("if matches_participle_object(window)")
        .expect("the arm is not wired into the zero-tool detector");
    let detector = src
        .find("pub fn has_phantom_tool_intent_no_tools")
        .expect("detector vanished");
    assert!(
        call > detector,
        "the arm is wired before the detector that should own it"
    );
}

#[test]
fn the_post_tool_detector_stays_blind_to_the_shape() {
    // Required regression case 4 from #1694. The structural pin above says the
    // arm is unreachable from the post-tool path; this pins the behaviour on
    // the incident's own sentence, so that a future refactor which hoists the
    // call out of the zero-tool detector fails here and not in production.
    // After a real call the shape is a legitimate recap of work just done
    // (#1506/#1172), and nagging that is worse than missing one phantom.
    assert!(
        !has_phantom_tool_intent(
            "Both receipts landed. Reading them, with mtimes so I know each postdates the tree."
        ),
        "the participle arm must not reach the post-tool detector"
    );
}
