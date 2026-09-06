//! A tapped follow-up is recorded on its own block, not spoken by the bot (#844).
//!
//! Tapping a suggestion posted a new message containing the chosen text. The
//! Bot API has no send-as-user, so that bubble carries the bot's name, avatar
//! and badge, and a continuation the user chose reads as the bot saying it.
//!
//! #787 tried to fix this by prefixing the echo with a `>` quote. Quote
//! formatting sits inside a bubble that is still labelled as the bot, so the
//! attribution did not change. The block is now edited in place instead, which
//! reads as a selected control and drops the keyboard at the moment the rest of
//! the options stop working.
//!
//! Fixtures are synthetic and carry no user identifiers.

use std::sync::Arc;

use teloxide::types::MessageId;

use crate::channels::telegram::state::{MergedHost, TelegramState};
use crate::channels::telegram::suggest_options::{
    PickRewrite, echo_fallback, folded_list_markdown, mark_picked_button, pick_rewrite,
    picked_block, suggestion_rows_rich_html,
};

const CHOICE: &str = "Update the SKILL.md with the new callback routing";

#[test]
fn the_edited_block_is_not_quoted() {
    // A leading `>` is what #787 added. It renders as a quote inside a bot
    // bubble, which is the appearance being fixed.
    let block = picked_block(CHOICE, None);
    assert!(
        !block.starts_with('>'),
        "the in-place record must not be quoted: {block}"
    );
}

#[test]
fn the_edited_block_marks_the_choice_and_keeps_the_text() {
    let block = picked_block(CHOICE, None);
    assert!(block.starts_with('\u{25b6}'), "must lead with the marker");
    assert!(block.contains(CHOICE), "the chosen text must survive");
}

#[test]
fn the_fallback_stays_quoted() {
    // Only reached when the block cannot be edited. Quoting is the weaker
    // presentation, which is why it is the fallback and not the default.
    let echo = echo_fallback(CHOICE, None);
    assert!(
        echo.starts_with("> "),
        "fallback must remain a quote: {echo}"
    );
    assert!(echo.contains(CHOICE));
}

#[test]
fn the_two_shapes_differ() {
    // If these ever converge, the fix has been undone: the whole point is
    // that the in-place record does not look like the old echo.
    assert_ne!(picked_block(CHOICE, None), echo_fallback(CHOICE, None));
}

#[test]
fn text_is_passed_through_untouched() {
    // The suggestion may contain markdown, backticks or angle brackets. This
    // layer must not mangle them; escaping is md_to_html's job at the call
    // site, and doing it twice would double-escape.
    for text in [
        "Run `cargo clippy` and report",
        "Compare <old> against <new>",
        "Fix the **bold** claim",
        "",
    ] {
        assert!(
            picked_block(text, None).ends_with(text),
            "text was altered: {text}"
        );
    }
}

// ── Attribution (#893) ──────────────────────────────────────────────────────

#[test]
fn the_chooser_is_named_when_known() {
    // In a group, an unattributed line says nothing about who acted.
    let block = picked_block(CHOICE, Some("Daniel"));
    assert!(block.contains("Daniel"), "chooser missing: {block}");
    assert!(block.contains(CHOICE), "the choice must survive: {block}");
}

#[test]
fn an_unknown_chooser_falls_back_to_the_plain_record() {
    // Identity is not always available; the pick must still be recorded.
    for chooser in [None, Some(""), Some("   ")] {
        let block = picked_block(CHOICE, chooser);
        assert!(block.contains(CHOICE), "the choice was lost: {block}");
        assert!(
            !block.contains("—"),
            "an empty name left a dangling separator"
        );
    }
}

#[test]
fn the_fallback_echo_names_the_chooser_too() {
    let echo = echo_fallback(CHOICE, Some("Daniel"));
    assert!(echo.starts_with("> "), "fallback must stay quoted: {echo}");
    assert!(echo.contains("Daniel"));
    assert!(echo.contains(CHOICE));
}

// ── Post-tap rewrite body (#39) ─────────────────────────────────────────────

#[test]
fn classic_host_body_keeps_answer_and_pick() {
    // The exact regression from the owner report 2026-08-29 23:51Z: the
    // classic merged host edited the answer HTML alone and the pick
    // record vanished. The body must carry BOTH, answer first.
    let rewrite = pick_rewrite(
        Some(("<b>the answer</b>", false, None)),
        picked_block(CHOICE, None),
        0,
    );
    let PickRewrite::ClassicHost(body) = rewrite else {
        panic!("a classic host must stay classic: {rewrite:?}")
    };
    assert!(
        body.starts_with("<b>the answer</b>"),
        "answer html first: {body}"
    );
    assert!(body.contains(CHOICE), "pick record survives: {body}");
    assert!(body.contains('\u{25b6}'), "pick marker survives: {body}");
}

#[test]
fn rich_host_body_keeps_answer_and_pick() {
    let rewrite = pick_rewrite(
        Some(("<b>the answer</b>", true, None)),
        picked_block(CHOICE, None),
        0,
    );
    let PickRewrite::RichHost(body) = rewrite else {
        panic!("a rich host must stay rich: {rewrite:?}")
    };
    assert!(
        body.starts_with("<b>the answer</b>"),
        "answer html first: {body}"
    );
    assert!(body.contains(CHOICE), "pick record survives: {body}");
}

#[test]
fn standalone_body_is_the_pick_record_alone() {
    let record = picked_block(CHOICE, None);
    assert_eq!(
        pick_rewrite(None, record.clone(), 0),
        PickRewrite::Standalone(record)
    );
}

#[test]
fn the_rich_flag_decides_the_transport_not_the_body() {
    // Same host html, same pick — only the rich flag flips, so the two
    // bodies must match byte for byte; only the variant differs.
    let picked = picked_block(CHOICE, None);
    let classic = pick_rewrite(Some(("host", false, None)), picked.clone(), 0);
    let rich = pick_rewrite(Some(("host", true, None)), picked, 0);
    fn body_of(r: &PickRewrite) -> &str {
        match r {
            PickRewrite::RichHost(b)
            | PickRewrite::RichMarkdownHost(b)
            | PickRewrite::ClassicHost(b)
            | PickRewrite::Standalone(b) => b.as_str(),
        }
    }
    assert_eq!(body_of(&classic), body_of(&rich));
    assert_ne!(classic, rich, "the variant must flip with the flag");
}

// ---- #59: stale-shell taps must know the host shape after a #597 clear ----
//
// The #597 clear wipes the stash when the user sends their own message, but
// RICH-merged buttons keep rendering inside the bubble body. The clear now
// rescues the merged-host record into a bounded stale map, so the stale-shell
// tap can strip by host shape instead of firing a blind markup strip (a
// guaranteed no-op on rich bubbles — the zombie).
//
// (Adapted to the upstream MergedHost shape: no `glued` field — that is the
// fork's #55 glue tier, disposition ASK, not ported.)

fn host(mid: i32, rich: bool) -> MergedHost {
    MergedHost {
        message_id: MessageId(mid),
        html: "<p>answer</p><tg-button-row><tg-button>Go</tg-button></tg-button-row>".into(),
        rich,
        markdown: None,
    }
}

async fn register_one(state: &TelegramState, sid: uuid::Uuid, token_tag: u8) -> String {
    let token = state
        .register_pending_followups(sid, vec![format!("Option {token_tag}")])
        .await;
    state
        .attach_followup_host(&token, host(100 + i32::from(token_tag), true))
        .await;
    token
}

#[tokio::test]
async fn clear_rescues_host_records_for_stale_taps() {
    let state = Arc::new(TelegramState::new());
    let sid = uuid::Uuid::new_v4();
    let token = register_one(&state, sid, 1).await;

    // No stale record before the clear.
    assert!(state.peek_stale_host(&token).await.is_none());

    state.clear_pending_followups(sid).await;

    // After the clear the stash is gone, but the rescued record survives.
    let h = state
        .peek_stale_host(&token)
        .await
        .expect("clear must rescue the merged-host record (#59)");
    assert!(h.rich);
    assert_eq!(h.message_id.0, 101);
}

#[tokio::test]
async fn forget_removes_only_the_stripped_record() {
    let state = Arc::new(TelegramState::new());
    let sid = uuid::Uuid::new_v4();
    let a = register_one(&state, sid, 1).await;
    let b = register_one(&state, sid, 2).await;
    state.clear_pending_followups(sid).await;

    state.forget_stale_host(&a).await;
    assert!(state.peek_stale_host(&a).await.is_none());
    assert!(
        state.peek_stale_host(&b).await.is_some(),
        "unrelated records must survive a forget"
    );
    // Idempotent: forgetting again is a no-op, not a panic.
    state.forget_stale_host(&a).await;
}

#[tokio::test]
async fn unmerged_entries_leave_no_stale_record() {
    let state = Arc::new(TelegramState::new());
    let sid = uuid::Uuid::new_v4();
    // Registered but the merge never landed: no host to attach, nothing to
    // strip later — the clear must NOT mint a stale record for it.
    let _ = state
        .register_pending_followups(sid, vec!["Bare option".to_string()])
        .await;
    state.clear_pending_followups(sid).await;
    assert_eq!(state.stale_host_count().await, 0);
}

#[tokio::test]
async fn stale_map_is_bounded_at_the_cap() {
    let state = Arc::new(TelegramState::new());
    let sid = uuid::Uuid::new_v4();
    // CAP + 5 registrations, all cleared in one sweep: the deque must shed
    // the oldest records down to the cap.
    for i in 0..37 {
        let _ = register_one(&state, sid, (i % 250 + 1) as u8).await;
    }
    state.clear_pending_followups(sid).await;
    assert!(
        state.stale_host_count().await <= 32,
        "stale-host map must be bounded (FIFO eviction)"
    );
}

// ── #67 tap-redraw: mark_picked_button ──────────────────────────────────────

fn shared_row_html() -> String {
    // Build via the real renderer so the transform is tested against the
    // production markup shape, not a hand-typed lookalike.
    suggestion_rows_rich_html(&["Approve".to_string(), "Decline".to_string()], "tok")
}

#[test]
fn picked_button_flips_to_success_check_disabled() {
    let marked = mark_picked_button(&shared_row_html(), 0);
    let picked_btn = marked.split("<tg-button ").nth(1).unwrap();
    let picked_span = &picked_btn[..picked_btn.find("</tg-button>").unwrap()];
    assert!(
        picked_span.contains("style=\"success\""),
        "picked flips to success: {picked_span}"
    );
    assert!(
        picked_span.contains(" disabled"),
        "picked disabled: {picked_span}"
    );
    assert!(
        picked_span.ends_with("\u{2713} Approve"),
        "check prefix on the picked label: {picked_span}"
    );
}

#[test]
fn unpicked_buttons_disabled_drop_style_and_label() {
    let marked = mark_picked_button(&shared_row_html(), 0);
    let second = marked.split("<tg-button ").nth(2).unwrap();
    let span = &second[..second.find("</tg-button>").unwrap()];
    assert!(span.contains(" disabled"), "sibling disabled: {span}");
    assert!(
        !span.contains("style="),
        "#71: sibling drops its style (style visually eats disabled): {span}"
    );
    assert!(!span.contains('\u{2713}'), "no check on siblings: {span}");
    assert!(span.ends_with(">Decline"), "label untouched: {span}");
}

#[test]
fn non_followup_markup_passes_through_byte_for_byte() {
    let html = "<p>hi</p><tg-button-row><tg-button type=\"url\" data=\"https://x\">x</tg-button></tg-button-row>";
    assert_eq!(mark_picked_button(html, 0), html);
}

#[test]
fn html_without_buttons_is_identity() {
    let html = "<b>answer</b>\n\n<p>record</p>";
    assert_eq!(mark_picked_button(html, 1), html);
}

#[test]
fn tap_redraw_rich_host_body_has_marked_rows_and_record() {
    // End-to-end through pick_rewrite: rows rewritten to the picked state,
    // record appended, #39 order preserved (answer/rows first, record last).
    let host = format!("<b>the answer</b>\n{}", shared_row_html());
    let rewrite = pick_rewrite(Some((&host, true, None)), picked_block(CHOICE, None), 1);
    let PickRewrite::RichHost(body) = rewrite else {
        panic!("rich host stays rich")
    };
    assert!(body.contains("style=\"success\""), "picked marked: {body}");
    assert!(body.contains(" disabled"), "buttons disabled: {body}");
    assert!(
        !body.contains("style=\"primary\"\">Approve"),
        "the picked label must be check-prefixed"
    );
    assert!(body.contains(CHOICE), "pick record survives: {body}");
    assert!(
        body.rfind(CHOICE).unwrap() > body.rfind("</tg-button-row>").unwrap(),
        "record rides after the rows (#39)"
    );
}

#[test]
fn markdown_host_routes_to_the_markdown_plane() {
    // #79 piece 4: a markdown-plane host (markdown: Some) must produce the
    // RichMarkdownHost variant even though its body bytes rewrite exactly
    // like an html-plane host — the plane decides the transport, not the
    // content.
    let host = "<b>answer</b>\n| a | b |\n|---|---|\n| 1 | 2 |".to_string();
    let rewrite = pick_rewrite(
        Some((host.as_str(), true, Some(host.as_str()))),
        picked_block(CHOICE, None),
        0,
    );
    let PickRewrite::RichMarkdownHost(body) = rewrite else {
        panic!("markdown host must ride the markdown plane: {rewrite:?}")
    };
    assert!(body.starts_with("<b>answer</b>"), "answer first: {body}");
    assert!(
        body.contains("| a | b |"),
        "markdown table survives: {body}"
    );
    assert!(body.contains(CHOICE), "pick record survives: {body}");
}

#[test]
fn markdown_and_html_hosts_rewrite_identically() {
    // Same body bytes, different plane: the rewritten bodies must match
    // byte for byte — only the variant (transport) differs.
    let host = "<b>the answer</b>";
    let picked = picked_block(CHOICE, None);
    let html_plane = pick_rewrite(Some((host, true, None)), picked.clone(), 0);
    let md_plane = pick_rewrite(Some((host, true, Some(host))), picked, 0);
    match (html_plane, md_plane) {
        (PickRewrite::RichHost(a), PickRewrite::RichMarkdownHost(b)) => {
            assert_eq!(a, b, "rewrite is plane-agnostic")
        }
        other => panic!("unexpected variants: {other:?}"),
    }
}

#[test]
fn folded_list_markdown_numbers_each_option() {
    let list = folded_list_markdown(&["первый".to_string(), "второй|с таблицей".to_string()]);
    assert_eq!(list, "1. первый\n2. второй|с таблицей");
}
