//! Regression for #1513: a colon-terminated work announcement that is
//! followed by more paragraphs was invisible to `work_announcement_re`.
//!
//! The regex accepts three imminence endings after the gerund: " now", an
//! ellipsis, or a colon. The colon and bare-now forms were written as
//! `:\s*$` with no multiline flag, so `$` meant end of the whole window.
//! `announcement_matches_anywhere` offers each sentence-start suffix running
//! to the END of the lead-in, so an announcement with any text after it
//! could never reach the anchor. A post-success reply that narrated a plan
//! paragraph by paragraph ("Anchors confirmed. Executing all wiring edits,
//! batch 1 (five files, parallel):" then "Batch 1 landed. Moving to batch
//! 2:") was logged as a pure completion acknowledgement and the turn closed
//! on an undelivered promise.
//!
//! Fix: the colon and bare-now markers match at end of LINE, in all six
//! language files. The per-gerund span bound is left alone on purpose
//! (#1193 chose connectives over a wider bound).

use crate::brain::agent::service::phantom::{
    has_forward_intent_post_success, matches_work_announcement,
};

const PARAGRAPH_NARRATION: &str = "\nAll anchors re-verified against the current tree, nothing drifted since the plan:\n\nAnchors confirmed. Executing all wiring edits, batch 1 (five different files, parallel):\n\nBatch 1 landed. Moving to batch 2 (three files):\n\nAll wiring in place.";

#[test]
fn colon_announcement_in_second_paragraph_is_forward_intent() {
    assert!(
        has_forward_intent_post_success(PARAGRAPH_NARRATION),
        "a colon-terminated announcement followed by more paragraphs must count as forward intent"
    );
}

#[test]
fn colon_announcement_matches_when_text_continues_after_it() {
    let text =
        "Executing all wiring edits, batch 1 (five different files, parallel):\n\nBatch 1 landed.";
    assert!(matches_work_announcement(text));
}

#[test]
fn bare_now_announcement_matches_when_text_continues_after_it() {
    let text = "Pushing the three commits now\n\nThen I will update the changelog.";
    assert!(matches_work_announcement(text));
}

#[test]
fn colon_announcement_matches_in_other_languages_mid_text() {
    let pt = "Âncoras confirmadas. Executando todas as edições, lote 1 (cinco arquivos):\n\nLote 1 concluído.";
    let es = "Anclas confirmadas. Ejecutando todos los cambios, lote 1 (cinco archivos):\n\nListo el lote 1.";
    assert!(matches_work_announcement(pt), "pt");
    assert!(matches_work_announcement(es), "es");
}

#[test]
fn colon_heading_without_gerund_is_still_a_completion_ack() {
    let text = "Summary of the changes:\n\nThe build passed and every test is green. Nothing else is pending.";
    assert!(
        !has_forward_intent_post_success(text),
        "a plain colon heading must not become forward intent"
    );
}

#[test]
fn gerund_without_imminence_marker_is_not_an_announcement() {
    let text = "Executing the full suite was slow, but every test passed on the second run.";
    assert!(!matches_work_announcement(text));
}

#[test]
fn every_language_file_anchors_colon_marker_at_end_of_line() {
    let files = [
        (
            "en",
            include_str!("../brain/agent/service/phantom_lang/en.toml"),
        ),
        (
            "es",
            include_str!("../brain/agent/service/phantom_lang/es.toml"),
        ),
        (
            "fr",
            include_str!("../brain/agent/service/phantom_lang/fr.toml"),
        ),
        (
            "id",
            include_str!("../brain/agent/service/phantom_lang/id.toml"),
        ),
        (
            "pt",
            include_str!("../brain/agent/service/phantom_lang/pt.toml"),
        ),
        (
            "ru",
            include_str!("../brain/agent/service/phantom_lang/ru.toml"),
        ),
    ];
    for (lang, src) in files {
        let line = src
            .lines()
            .find(|l| l.starts_with("work_announcement_re = "))
            .unwrap_or_else(|| panic!("{lang}: work_announcement_re missing"));
        assert!(
            !line.contains("\\\\s*$)"),
            "{lang}: work_announcement_re still anchors a marker at end of text"
        );
        assert!(
            line.contains("(?:\\\\n|$)"),
            "{lang}: work_announcement_re must anchor markers at end of line"
        );
    }
}
