//! Suggestion controls ride the reply bubble (#1204).
//!
//! Every turn ending in `suggest_options` used to produce TWO messages: the
//! formatted reply, then a standalone plain `💡 Suggested next:` bubble
//! carrying the keyboard, because the picker bypassed the rich pipeline
//! entirely. The controls now attach to the delivered answer.
//!
//! The layout cases below were lifted out of `suggest_options.rs`: the house
//! rule is that no source file carries a `#[cfg(test)] mod tests` block.

use crate::channels::telegram::suggest_options::{
    BUTTON_LABEL_MAX_UNITS, FOLLOWUP_PREFIX, MAX_NUMBERS_PER_ROW, SHARED_ROW_MAX_CHARS,
    SuggestLayout, enforce_button_fit, folded_list_html, pick_layout, suggestion_rows_rich_html,
};

fn opts(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

// ── Layout ladder (lifted, unchanged in substance) ────────────────────────

#[test]
fn test_short_few_options_share_one_row() {
    assert_eq!(
        pick_layout(&opts(&["Yes", "No", "Skip"])),
        SuggestLayout::SharedRow
    );
}

#[test]
fn test_five_short_options_do_not_share_a_row() {
    // More than MAX_NUMBERS_PER_ROW tap targets in one row leaves each too
    // small for a finger, so they drop to the Column tier.
    let o = opts(&["alpha", "beta", "gamma", "delta", "eps"]);
    assert!(o.iter().all(|s| s.chars().count() <= SHARED_ROW_MAX_CHARS));
    assert!(o.len() > MAX_NUMBERS_PER_ROW);
    assert_eq!(pick_layout(&o), SuggestLayout::Column);
}

#[test]
fn test_one_long_label_kills_the_shared_row() {
    let o = vec!["Yes".to_string(), "x".repeat(SHARED_ROW_MAX_CHARS + 1)];
    assert_eq!(pick_layout(&o), SuggestLayout::Column);
}

#[test]
fn test_the_button_width_boundary_is_exclusive() {
    // Recalibrated 2026-09-04 (#79 owner smokes): BUTTON_LABEL_MAX_UNITS
    // rides a full-width button, past it the set folds — a clipped label
    // is a correctness bug, an over-eager fold is only cosmetic.
    assert_eq!(
        pick_layout(&["x".repeat(BUTTON_LABEL_MAX_UNITS)]),
        SuggestLayout::Column
    );
    assert_eq!(
        pick_layout(&[
            "Ship it".to_string(),
            "x".repeat(BUTTON_LABEL_MAX_UNITS + 1)
        ]),
        SuggestLayout::NumberedProse
    );
}

#[test]
fn test_shared_row_respects_the_total_budget() {
    // #79: shared rows cut at 36 total; only a 24 slim-tail pair held, so
    // the total budget is 20. Two 12-char labels fit neither the budget
    // nor one row — they drop to Column (one full-width button per row).
    let o = vec![
        "x".repeat(SHARED_ROW_MAX_CHARS),
        "y".repeat(SHARED_ROW_MAX_CHARS),
    ];
    assert_eq!(pick_layout(&o), SuggestLayout::Column);
}

#[test]
fn test_a_folded_list_carries_no_header_and_is_escaped() {
    let body = folded_list_html(&opts(&["Ship it", "Review & merge"]));
    assert!(
        !body.contains("Suggested next"),
        "#1204: the list rides under the answer, so it has no header of its own"
    );
    assert!(body.contains("1. Ship it"));
    assert!(
        body.contains("2. Review &amp; merge"),
        "escaping proves the shared renderer ran, not a private one: {body}"
    );
}

// ── Callback data ─────────────────────────────────────────────────────────

#[test]
fn test_callback_data_carries_the_index_not_the_text() {
    // Telegram caps callback_data at 64 BYTES and an option's text can exceed
    // that on its own, so the index is what travels and the stash resolves it.
    // #1217: the token is opaque (8 hex chars), NOT the session id — the tap
    // resolves the serving session from the stash entry, never from data the
    // client could forge or mix across overlapping keyboards.
    let token = "ab12cd34";
    let html = suggestion_rows_rich_html(&opts(&["Ship it", "Hold"]), token);

    for i in 0..2 {
        let expected = format!("{FOLLOWUP_PREFIX}{token}:{i}");
        assert!(
            html.contains(&expected),
            "missing callback data {expected} in {html}"
        );
        assert!(
            expected.len() <= 64,
            "#1204: callback data must fit Telegram's 64-byte cap, got {}",
            expected.len()
        );
    }
    // The option text must not ride in the payload (it can exceed the cap).
    assert!(
        !html.contains("Ship it\" data="),
        "option text must not ride in callback data"
    );
}

#[test]
fn test_callback_data_stays_within_the_cap_at_the_worst_index() {
    // Token form is 9 + 8 + 1 + idx chars; pin the widest realistic index
    // against the cap (#1204).
    let token = "ffffffff";
    let widest = format!("{FOLLOWUP_PREFIX}{token}:{}", usize::from(u8::MAX));
    assert!(
        widest.len() <= 64,
        "#1204: {} bytes exceeds the callback_data cap",
        widest.len()
    );
}

#[test]
fn test_enforce_button_fit_ships_fitting_bodies_byte_identical() {
    let body = "<p>Готово</p>\n<tg-button-row>\
                <tg-button type=\"callback_data\" data=\"followup:tok:0\" \
                style=\"primary\">Полёт</tg-button></tg-button-row>";
    assert_eq!(enforce_button_fit(body), body);
}

#[test]
fn test_enforce_button_fit_folds_oversized_labels_keeping_routing() {
    // 30-char Cyrillic label: past BUTTON_LABEL_MAX_UNITS=20 -> the set
    // folds; the button keeps its attrs (routing untouched) but shows its
    // index, and the original label moves into the <ol>.
    let long = "Проверка ширины кнопки хххххххххЖЖЖ";
    let body = format!(
        "<tg-button-row><tg-button type=\"url\" data=\"https://x/{long}\">{long}\
         </tg-button></tg-button-row>"
    );
    let out = enforce_button_fit(&body);
    assert!(
        out.contains("<tg-button type=\"url\" data=\"https://x/"),
        "{out}"
    );
    assert!(out.contains(">1</tg-button>"), "{out}");
    assert!(out.contains("<li>"), "{out}");
    assert!(out.contains(long), "{out}");
    // Idempotent: the folded body passes through unchanged.
    assert_eq!(enforce_button_fit(&out), out);
}

#[test]
fn test_enforce_button_fit_folds_rows_over_the_total_budget() {
    // Two 12-char labels = 24 total: past SHARED_ROW_TOTAL_UNITS=20 (#79).
    let body = "<tg-button-row>\
                <tg-button type=\"callback_data\" data=\"followup:t:0\">Полёт норм!!\
                </tg-button><tg-button type=\"callback_data\" data=\"followup:t:1\">\
                Всё чётко!!!</tg-button></tg-button-row>";
    let out = enforce_button_fit(body);
    assert!(
        out.contains(">1</tg-button>") && out.contains(">2</tg-button>"),
        "{out}"
    );
    assert!(out.contains("<li>Полёт норм!!</li>"), "{out}");
    assert!(out.contains("<li>Всё чётко!!!</li>"), "{out}");
}
