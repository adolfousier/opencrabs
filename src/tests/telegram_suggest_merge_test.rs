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
    SINGLE_BUTTON_MAX_UNITS, SuggestLayout, append_rows_and_trailer_md, enforce_button_fit,
    go_button_label, go_tier_line, go_tier_lines_rich, pick_layout, row_fits,
    suggestion_rows_rich_html,
};

fn opts(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn styled_opts(v: &[&str]) -> Vec<crate::brain::tools::suggest_options::SuggestionItem> {
    v.iter()
        .map(|s| crate::brain::tools::suggest_options::SuggestionItem::default_styled(*s))
        .collect()
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
fn test_single_option_rides_full_width_past_the_shared_budget() {
    // #119: the fold tier's cramming premise never applies at n=1 — a
    // lone option folded into "1. <label>" prose plus a bare "1" button
    // renders absurdly (live smoke on ff234125: 27-char confirm label).
    // One option rides a full-width button up to its own clip point.
    let label = "Smoke OK — ack both units".to_string(); // 25 chars, > 20
    assert!(label.chars().count() > BUTTON_LABEL_MAX_UNITS);
    assert_eq!(pick_layout(&[label]), SuggestLayout::Column);
    assert_eq!(
        pick_layout(&["x".repeat(SINGLE_BUTTON_MAX_UNITS)]),
        SuggestLayout::Column
    );
    // Past the single-button clip point the label still folds — but a
    // lone option has nothing to share the prose list with, so the fold
    // is the graceful degradation the issue accepts.
    assert_eq!(
        pick_layout(&["x".repeat(SINGLE_BUTTON_MAX_UNITS + 1)]),
        SuggestLayout::NumberedProse
    );
    // n >= 2 behavior unchanged: long labels still fold the whole set.
    assert_eq!(
        pick_layout(&["ok".to_string(), "x".repeat(BUTTON_LABEL_MAX_UNITS + 1)]),
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
fn test_go_tier_body_lines_carry_the_119_shape() {
    // #119 set-size split (owner order 2026-09-09, "back to the way the
    // numbered lists were before the Go button introduction"):
    // n>=2 = the ORIGINAL plain numbered list — `1. <label>`, no Go
    // prefix, no `?` mutation, no bold; n=1 keeps the confirmed Go! tier.
    let body = go_tier_lines_rich(&opts(&["Ship it", "Review & merge"]));
    assert!(
        !body.contains("Suggested next"),
        "#1204: the lines ride under the answer, no header of their own"
    );
    assert!(
        body.contains("1. Ship it") && body.contains("2. Review &amp; merge"),
        "plain numbered list for n>=2, escaping via the shared renderer: {body}"
    );
    assert!(
        !body.contains("Go:") && !body.contains("<b>"),
        "no Go-tier artifacts on the n>=2 fold: {body}"
    );
    assert!(
        !body.contains("<ol>"),
        "no html <ol> — plain numbered prose: {body}"
    );
    // n=1 keeps the confirmed Go! tier (bold, deduped ?).
    let single = go_tier_lines_rich(&opts(&["Ship it"]));
    assert_eq!(single, "<b>Go: Ship it?</b>");
}

#[test]
fn test_go_tier_dedupes_trailing_question_mark() {
    // Owner correction 2026-09-09: a label ending in '?' must not gain a
    // second question mark (n=1 Go! tier only).
    assert_eq!(go_tier_line("Confirm render?"), "**Go: Confirm render?**");
    assert_eq!(go_tier_line("Confirm render??"), "**Go: Confirm render??**");
}

#[test]
fn test_go_tier_verb_repeat_rule() {
    // #119 owner amendment 06:46Z (n=1 Go! tier): a label that already
    // starts with the verb renders verbatim + `?` (deduped); otherwise
    // the `Go:` prefix is added. Whole line is bold per the 2026-09-09
    // correction.
    assert_eq!(
        go_tier_line("Go — implement #98 after the #96 gate lands"),
        "**Go — implement #98 after the #96 gate lands?**"
    );
    assert_eq!(
        go_tier_line("Smoke OK — ack both units"),
        "**Go: Smoke OK — ack both units?**"
    );
    // Case-insensitive verb match (Go!/go both start the label).
    assert_eq!(go_tier_line("go fast"), "**go fast?**");
    // Prefix-verb must not false-positive mid-word.
    assert_eq!(go_tier_line("Gossip about it"), "**Go: Gossip about it?**");
}

#[test]
fn test_go_tier_buttons_carry_their_option_number() {
    // Fold tier button labels: n>=2 carries `N. <FirstWord>` (e.g. `1. Ship`, `2. xxx...`),
    // pairing with the `1. <label>` body line while giving immediate context on the button.
    // Callback data still carries the absolute index. n=1 folds to the single `Go!` button.
    let token = "ab12cd34";
    let html = suggestion_rows_rich_html(&styled_opts(&["Ship it", &"x".repeat(30)]), token);
    assert!(
        html.contains(&format!(
            ">{}</tg-button>",
            go_button_label(1, "Ship it", false)
        )) && html.contains(&format!(
            ">{}</tg-button>",
            go_button_label(2, &"x".repeat(30), false)
        )),
        "one numbered button per option: {html}"
    );
    assert!(
        !html.contains(">Go!<"),
        "no Go! buttons on the n>=2 fold: {html}"
    );
    assert!(
        html.contains(&format!("{FOLLOWUP_PREFIX}{token}:0")),
        "callback data still routes by index: {html}"
    );
    // n=1: the single Go! button.
    assert_eq!(go_button_label(1, "Ship it", true), "Go!");
    assert_eq!(go_button_label(1, "Deploy to prod", false), "1. Deploy");
    assert_eq!(go_button_label(2, "Cancel run", false), "2. Cancel");
    let single_html = suggestion_rows_rich_html(&styled_opts(&[&"x".repeat(31)]), token);
    assert!(
        single_html.contains(">Go!</tg-button>"),
        "single fold keeps the Go! button: {single_html}"
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
    let html = suggestion_rows_rich_html(&styled_opts(&["Ship it", "Hold"]), token);

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
    // 35-char Cyrillic label: past BUTTON_LABEL_MAX_UNITS=20 and past the
    // solo budget (30), so the set folds; the button keeps its attrs
    // (routing untouched), carries its fold-tier label, and the original
    // label moves into the <ol>. One button in the set is the Go! tier
    // (owner order 2026-09-14) — a bare index here was the #396 defect.
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
    assert!(out.contains(">Go!</tg-button>"), "{out}");
    assert!(out.contains("<li>"), "{out}");
    assert!(out.contains(long), "{out}");
    // Idempotent: the folded body passes through unchanged.
    assert_eq!(enforce_button_fit(&out), out);
}

#[test]
fn test_enforce_button_fit_reshapes_over_budget_shared_row_to_column() {
    // Two 12-char labels = 24 total: past SHARED_ROW_TOTAL_UNITS=20 (#79).
    // Under #137, because both labels fit within SINGLE_BUTTON_MAX_UNITS=30,
    // the enforcer re-shapes the row into a Column layout (one button per row)
    // with original labels intact, rather than folding into digits + <ol>.
    let body = "<tg-button-row>\
                <tg-button type=\"callback_data\" data=\"followup:t:0\">Полёт норм!!\
                </tg-button><tg-button type=\"callback_data\" data=\"followup:t:1\">\
                Всё чётко!!!</tg-button></tg-button-row>";
    let out = enforce_button_fit(body);
    assert!(!out.contains("<ol>"), "{out}");
    assert!(!out.contains(">1</tg-button>"), "{out}");
    assert!(out.contains("Полёт норм!!"), "{out}");
    assert!(out.contains("Всё чётко!!!"), "{out}");
    // Verify each button is in its own row
    assert_eq!(
        out,
        "<tg-button-row><tg-button type=\"callback_data\" data=\"followup:t:0\">Полёт норм!!</tg-button></tg-button-row>\n<tg-button-row><tg-button type=\"callback_data\" data=\"followup:t:1\">Всё чётко!!!</tg-button></tg-button-row>"
    );
    // Idempotent:
    assert_eq!(enforce_button_fit(&out), out);
}

#[test]
fn test_enforce_button_fit_folds_multi_button_row_when_label_exceeds_solo_budget() {
    // Multi-button row where one button label exceeds SINGLE_BUTTON_MAX_UNITS (30).
    // Column re-shaping cannot accommodate a label > 30 units, so it must fall back
    // to the NumberedProse fold at the owner's 2026-09-14 tier: each button
    // carries `N. <FirstWord>` while the full labels move into the <ol>.
    let long_label = "Проверка ширины кнопки хххххххххЖЖЖ"; // 35 chars
    let body = format!(
        "<tg-button-row>\
         <tg-button type=\"callback_data\" data=\"followup:t:0\">{long_label}</tg-button>\
         <tg-button type=\"callback_data\" data=\"followup:t:1\">OK</tg-button>\
         </tg-button-row>"
    );
    let out = enforce_button_fit(&body);
    assert!(out.contains(">1. Проверка</tg-button>"), "{out}");
    assert!(out.contains(">2. OK</tg-button>"), "{out}");
    assert!(out.contains(&format!("<li>{long_label}</li>")), "{out}");
    assert!(out.contains("<li>OK</li>"), "{out}");
    // Idempotent:
    assert_eq!(enforce_button_fit(&out), out);
}

#[test]
fn test_enforce_button_fit_groups_contiguous_rows_into_single_set() {
    // Two rows separated only by whitespace belong to the same contiguous set.
    // Row 1 has one button (10 chars), Row 2 has two buttons (14 + 15 chars).
    // Row 2 is over budget (total 29 > 20, labels > 12), but all labels <= 30.
    // Entire set re-shapes into 3 single-button rows.
    let body = "<tg-button-row>\
                <tg-button type=\"callback_data\" data=\"followup:t:0\">Option One</tg-button>\
                </tg-button-row>\n\n<tg-button-row>\
                <tg-button type=\"callback_data\" data=\"followup:t:1\">Option Two Long</tg-button>\
                <tg-button type=\"callback_data\" data=\"followup:t:2\">Option Three Long</tg-button>\
                </tg-button-row>";
    let out = enforce_button_fit(body);
    assert!(!out.contains("<ol>"), "{out}");
    assert!(!out.contains(">1</tg-button>"), "{out}");
    let expected = "<tg-button-row><tg-button type=\"callback_data\" data=\"followup:t:0\">Option One</tg-button></tg-button-row>\n<tg-button-row><tg-button type=\"callback_data\" data=\"followup:t:1\">Option Two Long</tg-button></tg-button-row>\n<tg-button-row><tg-button type=\"callback_data\" data=\"followup:t:2\">Option Three Long</tg-button></tg-button-row>";
    assert_eq!(out, expected);
    assert_eq!(enforce_button_fit(&out), out);
}

#[test]
fn test_enforce_button_fit_splits_sets_on_intervening_non_whitespace_text() {
    // Three sets separated by non-whitespace text nodes:
    // Set 1: 2 buttons that fit as-authored (3 + 2 <= 20).
    // Set 2: 2 buttons over shared budget (17 + 16 > 20), but both <= 30 -> re-shapes to Column.
    // Set 3: 1 button with 35 chars > 30 -> folds to NumberedProse.
    let body = "<p>First question:</p>\n\
                <tg-button-row><tg-button type=\"callback_data\" data=\"followup:t:0\">Yes</tg-button><tg-button type=\"callback_data\" data=\"followup:t:1\">No</tg-button></tg-button-row>\n\
                <p>Second question:</p>\n\
                <tg-button-row><tg-button type=\"callback_data\" data=\"followup:t:2\">Choice Alpha Long</tg-button><tg-button type=\"callback_data\" data=\"followup:t:3\">Choice Beta Long</tg-button></tg-button-row>\n\
                <p>Third question:</p>\n\
                <tg-button-row><tg-button type=\"callback_data\" data=\"followup:t:4\">Проверка ширины кнопки хххххххххЖЖЖ</tg-button></tg-button-row>";
    let out = enforce_button_fit(body);

    // Set 1 remains as-authored:
    assert!(out.contains("<tg-button-row><tg-button type=\"callback_data\" data=\"followup:t:0\">Yes</tg-button><tg-button type=\"callback_data\" data=\"followup:t:1\">No</tg-button></tg-button-row>"), "{out}");
    // Set 2 re-shapes into 2 separate rows:
    assert!(out.contains("<tg-button-row><tg-button type=\"callback_data\" data=\"followup:t:2\">Choice Alpha Long</tg-button></tg-button-row>\n<tg-button-row><tg-button type=\"callback_data\" data=\"followup:t:3\">Choice Beta Long</tg-button></tg-button-row>"), "{out}");
    // Set 3 folds with its own <ol> list; its one button is the Go! tier:
    assert!(out.contains("<tg-button-row><tg-button type=\"callback_data\" data=\"followup:t:4\">Go!</tg-button></tg-button-row>\n<ol><li>Проверка ширины кнопки хххххххххЖЖЖ</li></ol>"), "{out}");
    // Intervening text is preserved:
    assert!(out.contains("<p>First question:</p>"), "{out}");
    assert!(out.contains("<p>Second question:</p>"), "{out}");
    assert!(out.contains("<p>Third question:</p>"), "{out}");
    // Idempotent:
    assert_eq!(enforce_button_fit(&out), out);
}

#[test]
fn test_enforce_button_fit_handles_malformed_and_empty_rows() {
    let body = "<tg-button-row></tg-button-row>\n<tg-button-row><tg-button type=\"callback_data\" data=\"test\">OK</tg-button></tg-button-row>";
    let out = enforce_button_fit(body);
    assert!(out.contains("OK"), "{out}");
    assert_eq!(enforce_button_fit(&out), out);
}

// ── row_fits — the single budget verdict (#119 option A) ─────────────────

#[test]
fn test_row_fits_solo_arm_grants_the_solo_budget() {
    // One button renders full-width: it rides SINGLE_BUTTON_MAX_UNITS,
    // NOT the 20-unit shared caps — the arm whose absence made the #79
    // funnel re-fold full-width single-button rows the emitter approved.
    let label = "x".repeat(SINGLE_BUTTON_MAX_UNITS);
    assert!(row_fits(&[&label]));
    assert!(!row_fits(&[&"x".repeat(SINGLE_BUTTON_MAX_UNITS + 1)]));
    // A solo label past the SHARED caps but inside the solo budget:
    // exactly the live-failure class (25-27 char confirm labels).
    let confirm = "Smoke OK — ack both units".to_string();
    assert!(confirm.chars().count() > BUTTON_LABEL_MAX_UNITS);
    assert!(row_fits(&[&confirm]));
}

#[test]
fn test_row_fits_shared_arm_keeps_the_79_caps() {
    // Multi-button rows: per-label SHARED_ROW_MAX_CHARS AND row-total
    // SHARED_ROW_TOTAL_UNITS — byte-identical to the pre-refactor rule.
    let a = "x".repeat(SHARED_ROW_MAX_CHARS);
    let b = "y".repeat(SHARED_ROW_MAX_CHARS);
    // 2x12 = 24 total > 20: does not fit (folds, as before the refactor).
    assert!(!row_fits(&[&a, &b]));
    let short = "x".repeat(SHARED_ROW_MAX_CHARS - 4);
    // 2x8 = 16 total, labels within per-label cap: fits.
    assert!(row_fits(&[&short, &short]));
    // One label past the per-label cap kills the row even under budget.
    let long = "x".repeat(SHARED_ROW_MAX_CHARS + 1);
    assert!(!row_fits(&[&short, &long]));
}

#[test]
fn test_chokepoint_ships_solo_rows_the_emitter_approved() {
    // THE #119 live regression, pinned at the funnel: a single-button row
    // with a 25-char label used to be re-folded by enforce_button_fit
    // (old inline caps) after pick_layout's Column tier approved it —
    // owner saw "button 1 + 1. <label>" prose on ship ed0ae1d9. Under
    // the single-verdict refactor the funnel ships it byte-identical.
    let label = "Smoke OK — ack both units"; // 25 chars: > 20, <= 30
    let body = format!(
        "<tg-button-row><tg-button type=\"callback_data\" data=\"followup:tok:0\" \
         style=\"primary\">{label}</tg-button></tg-button-row>"
    );
    assert_eq!(enforce_button_fit(&body), body);
}

/// #108: the button rows and the trailer must start a FRESH markdown block.
/// The old construction appended both after a single `\n`, so the server's
/// parser fused the last text paragraph into the controls block and rendered
/// it indented (owner-reported 2026-09-05 on the board chat, msg 41990).
#[test]
fn test_rows_and_trailer_start_a_fresh_markdown_block() {
    let _raw_options: Vec<String> = vec!["One".into(), "Two".into(), "Three".into()];
    let options = styled_opts(&["One", "Two", "Three"]);
    for prose in [false, true] {
        let mut md = String::from("Answer paragraph.\nSecond line.");
        append_rows_and_trailer_md(&mut md, &options, "tok", prose, Some("Sign-off."));
        let rows = suggestion_rows_rich_html(&options, "tok");
        let sep = "\n\n<tg-button-row>";
        assert!(
            md.contains(sep),
            "prose={prose}: rows not block-separated: {md}"
        );
        let trailer_sep = "\n\nSign-off.";
        assert!(
            md.contains(trailer_sep),
            "prose={prose}: trailer not block-separated: {md}"
        );
        assert!(md.ends_with("Sign-off."));
        assert!(md.contains(&rows));
        if prose {
            // 2026-09-09, owner order: n>=2 fold = the original PLAIN
            // numbered list — no Go prefix, no bold. Blank-line-separated
            // from the body.
            // #207: separated by horizontal rule (---) to isolate list counters
            assert!(
                md.contains("---\n\n1. One"),
                "isolated numbered fold body line: {md}"
            );
            assert!(
                !md.contains("Go:") && !md.contains("**"),
                "no Go-tier artifacts: {md}"
            );
        }
    }
    // Body already ending in a newline must not grow a triple gap.
    let mut md = String::from("Answer.\n");
    append_rows_and_trailer_md(&mut md, &options, "tok", false, None);
    assert!(md.starts_with("Answer.\n\n<tg-button-row>"), "{md}");
    assert!(!md.starts_with("Answer.\n\n\n"), "{md}");
}

// ── #396 — the funnel must measure the label's DISPLAY width ──────────────

#[test]
fn test_owner_label_with_ampersand_survives_the_rich_funnel() {
    // #396: `pick_layout` measured the RAW label (30 units) and approved a
    // Column layout, but `enforce_button_fit` re-measured the ESCAPED form —
    // `&` becomes `&amp;`, four units longer — read 34 > 30, and folded the
    // row, so the button rendered as a bare `1` instead of its label. Both
    // feeders now measure the same unit (the label's display text), so a row
    // the emitter approved ships through the funnel untouched.
    let label = "Acknowledge & stamp gap closed";
    let raw_units = label.chars().count();
    let escaped_units = crate::channels::telegram::markdown::escape_html(label).chars().count();
    assert_eq!(raw_units, 30, "the solo budget is 30 display units");
    assert_eq!(
        escaped_units, 34,
        "the escaped form is what the enforcer used to measure"
    );
    // Compare MEASURED values, never two literals: a condition whose operands
    // are both compile-time constants is resolved before the test runs, proves
    // nothing, and is rejected by clippy::assertions_on_constants under
    // `-D warnings`. The property under test is the MISMATCH — the raw label
    // fits the solo budget while its escaped form does not.
    assert!(
        escaped_units > SINGLE_BUTTON_MAX_UNITS && raw_units <= SINGLE_BUTTON_MAX_UNITS,
        "the overcount tripped the fold: {escaped_units} > {SINGLE_BUTTON_MAX_UNITS} >= {raw_units}"
    );

    let token = "ab12cd34";
    let emitted = suggestion_rows_rich_html(&styled_opts(&[label]), token);
    let shipped = enforce_button_fit(&emitted);

    assert_eq!(
        shipped, emitted,
        "a row the emitter approved must ship byte-identical through the funnel"
    );
    assert!(
        shipped.contains("Acknowledge &amp; stamp gap closed"),
        "the escaped label rides the button verbatim: {shipped}"
    );
    assert!(!shipped.contains("<ol>"), "no fold list: {shipped}");
    assert!(
        !shipped.contains(">1</tg-button>"),
        "no bare-digit button: {shipped}"
    );
    assert!(
        shipped.contains(&format!("{FOLLOWUP_PREFIX}{token}:0")),
        "callback routing is untouched: {shipped}"
    );
}

// ── #396 — the fold tier and its width guard ──────────────────────────────

#[test]
fn test_fold_button_label_decodes_the_first_word_and_re_escapes_it() {
    // #396, second half: the fold branch used to write a bare `index.to_string()`,
    // so every folded button rendered as `1`, `2`, … with no hint of its action.
    // It now writes the owner's 2026-09-14 tier — `N. <FirstWord>` for n>=2 —
    // taken from the DECODED label and re-escaped for the HTML context.
    //
    // `R&D` is the sharp case. Labels are sliced raw out of the HTML, so this
    // one still reads `R&amp;D` at the enforcer. Taking the first word WITHOUT
    // decoding yields `1. R&amp;D`, and re-escaping THAT double-escapes to
    // `1. R&amp;amp;D`. Both wrong shapes are asserted absent below.
    let a_raw = "R&D review the whole roadmap thing"; // 34 raw units
    let a_esc = "R&amp;D review the whole roadmap thing"; // 38 escaped units
    let b = "Escalate to the vendor immediately"; // 34 raw units
    assert!(
        a_raw.chars().count() > SINGLE_BUTTON_MAX_UNITS
            && b.chars().count() > SINGLE_BUTTON_MAX_UNITS,
        "both labels must exceed the solo budget for the set to fold"
    );
    assert_eq!(
        crate::channels::telegram::markdown::escape_html(a_raw),
        a_esc,
        "the enforcer sees the escaped form"
    );

    let body = format!(
        "<tg-button-row><tg-button type=\"callback_data\" data=\"followup:t:0\">{a_esc}\
         </tg-button><tg-button type=\"callback_data\" data=\"followup:t:1\">{b}\
         </tg-button></tg-button-row>"
    );
    let out = enforce_button_fit(&body);

    assert!(
        out.contains(">1. R&amp;D</tg-button>"),
        "the fold label is the decoded first word, re-escaped once: {out}"
    );
    assert!(out.contains(">2. Escalate</tg-button>"), "{out}");
    assert!(
        !out.contains("R&amp;amp;D"),
        "the fold label must not be double-escaped: {out}"
    );
    assert!(
        !out.contains("1. R&amp;D review"),
        "the fold label is the first word only, not the whole label: {out}"
    );
    // The <ol> keeps the raw (already-escaped) labels, untouched.
    assert!(out.contains(&format!("<li>{a_esc}</li>")), "{out}");
    assert!(out.contains(&format!("<li>{b}</li>")), "{out}");
    assert_eq!(enforce_button_fit(&out), out, "idempotent");
}

#[test]
fn test_fold_falls_back_to_the_bare_index_when_the_first_word_overflows() {
    // The fold label must itself fit the solo budget, or the folded body
    // re-triggers the fold on the next pass and grows without bound. The only
    // way to overflow is a long first word: `N. ` costs (digits + 2) units
    // against the 30-unit budget, so a 28-unit first word overflows at a
    // single-digit index. The guard then falls back to the no-first-word tier
    // (`N. ` -> bare index), which always fits.
    let long_first = format!("{} tail", "x".repeat(28)); // 33 units, 28-unit first word
    let other = "Escalate to the vendor immediately"; // 34 units
    let naive = go_button_label(1, &long_first, false);
    assert_eq!(naive.chars().count(), 31, "`1. ` + 28 units");
    assert!(
        !row_fits(&[naive.as_str()]),
        "the naive fold label overflows the solo budget"
    );
    let fallback = go_button_label(1, "", false);
    assert!(
        row_fits(&[fallback.as_str()]),
        "the bare-index fallback always fits"
    );

    let body = format!(
        "<tg-button-row><tg-button type=\"callback_data\" data=\"followup:t:0\">{long_first}\
         </tg-button><tg-button type=\"callback_data\" data=\"followup:t:1\">{other}\
         </tg-button></tg-button-row>"
    );
    let out = enforce_button_fit(&body);

    assert!(
        out.contains(">1</tg-button>"),
        "an over-long first word falls back to the bare index: {out}"
    );
    assert!(
        !out.contains(">1. x"),
        "the overflowing tier label is not written: {out}"
    );
    assert!(
        out.contains(">2. Escalate</tg-button>"),
        "the sibling button keeps its tier label: {out}"
    );
    assert!(out.contains(&format!("<li>{long_first}</li>")), "{out}");
    assert_eq!(enforce_button_fit(&out), out, "idempotent");
}
