//! Telegram-side rendering for the OPTIONAL `suggest_options` tool (#597).
//!
//! Non-blocking: the agent surfaces
//! `ProgressEvent::SuggestedOptions`, and we post an inline keyboard under the
//! finished response with one button per suggestion. Tapping a button injects
//! that suggestion as the user's next message (a fresh turn) — see the
//! `followup:` arm in the callback dispatcher. Typing your own message is always
//! available and just starts a normal turn; there is no oneshot and no timeout.

use std::sync::Arc;
use std::time::Duration;

use teloxide::payloads::{EditMessageTextSetters, SendMessageSetters};
use teloxide::types::{
    ChatId, InlineKeyboardButton, InlineKeyboardMarkup, MessageId, ParseMode, ThreadId,
};
use uuid::Uuid;

use super::TelegramState;

/// Callback-data prefix for a tapped follow-up suggestion: `followup:<session>:<idx>`.
pub(crate) const FOLLOWUP_PREFIX: &str = "followup:";

/// Body for the standalone fallback bubble (#1226 item 4). Prose mode keeps
/// just the folded list (it has no buttons, nothing can expire); button
/// modes carry the bare lamp plus an expiry marker — this fallback fires
/// when the merge lost a rate-limit race, so the bubble is subject to the
/// stale-shell lifecycle and operators kept reading dead fallbacks as
/// fresh, answerable questions (msgs 30997 / 31010: one was tapped 32
/// minutes after its choices were consumed).
pub(crate) fn standalone_fallback_body(layout: &SuggestLayout, options: &[String]) -> String {
    if *layout == SuggestLayout::NumberedProse {
        folded_list_html(options).trim_start().to_string()
    } else {
        String::from("\u{1f4a1} <i>(choices may have expired)</i>")
    }
}

/// What the suggestion block becomes once one of its options is tapped.
///
/// Replaces the prompt and its keyboard in place. The Bot API has no
/// send-as-user, so posting the choice as a new message renders a
/// user-chosen continuation under the bot's name, avatar and badge. A `>`
/// quote does not change that: the bubble is still labelled as the bot
/// (#844). Editing the block reads as a selected control instead.
pub(crate) fn picked_block(text: &str, chooser: Option<&str>) -> String {
    match chooser {
        // Name the member who chose it (#893). Without this the record reads as
        // an anonymous line from the bot, which in a group says nothing about
        // who acted. The Bot API cannot post AS a user, but the callback query
        // carries the tapper's identity and it was simply discarded.
        Some(name) if !name.trim().is_empty() => {
            format!("\u{25b6}\u{fe0f} {} \u{2014} {text}", name.trim())
        }
        _ => format!("\u{25b6}\u{fe0f} {text}"),
    }
}

/// Last-resort record when the suggestion block cannot be edited, because it
/// is too old or no longer accessible. Worse attribution than editing, but
/// losing the record of what was chosen is worse still.
pub(crate) fn echo_fallback(text: &str, chooser: Option<&str>) -> String {
    match chooser {
        Some(name) if !name.trim().is_empty() => {
            format!("> \u{25b6}\u{fe0f} {} \u{2014} {text}", name.trim())
        }
        _ => format!("> \u{25b6}\u{fe0f} {text}"),
    }
}

/// What a tapped suggestion block is rewritten into (#39).
///
/// The pick record is baked into the body BEFORE any transport arm runs,
/// so no arm can drop it: a merged host — rich or classic — keeps its
/// answer HTML and gains the pick record; a standalone block becomes the
/// pick record. Before this shape the append lived inside the rich arm
/// only, and the classic merged host silently lost the choice (owner
/// report 2026-08-29 23:51Z).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PickRewrite {
    /// Rich merged host: body rides `edit_rich_html`, empty markup strips
    /// the dead buttons.
    RichHost(String),
    /// Markdown-plane merged host (#79 piece 4): body rides
    /// `edit_rich_markdown` — the server render keeps tables intact
    /// (#679); empty markup strips the dead buttons.
    RichMarkdownHost(String),
    /// Classic merged host: body rides `edit_message_text` + empty markup
    /// to strip the dead buttons.
    ClassicHost(String),
    /// Standalone suggestion block: body rides plain `edit_message_text`.
    Standalone(String),
}

/// The single construction site for a post-tap body (#39).
///
/// `host` is `(html, rich)` of the merged host bubble when the tapped
/// message IS it; merged host → answer HTML + pick record, standalone →
/// pick record alone.
pub(crate) fn pick_rewrite(
    host: Option<(&str, bool, Option<&str>)>,
    picked_html: &str,
    picked_md: &str,
    picked_idx: usize,
) -> PickRewrite {
    match host {
        Some((full, rich, markdown)) => {
            if rich {
                // #67 tap-redraw: the rows must not survive as live
                // controls — they are rewritten to the picked state
                // (success+✓+disabled / disabled) instead of stripped, so
                // the bubble keeps showing what was chosen. `mark_picked_button`
                // is a byte-level `<tg-button` scan, plane-agnostic — the
                // same rewrite serves the html and markdown planes.
                //
                // #96: each plane must redraw in ITS OWN plane. The md-plane
                // host's `markdown` column carries the `<tg-button>` rows as
                // raw widget markup (the only html the rich-markdown renderer
                // accepts); rewriting the `full` HTML strip-source instead
                // posts `<p>`/`<b>` tags into the markdown field — Telegram
                // renders them literally (tag soup). Markdown hosts get
                // `mark_picked_button(markdown)` + the pick line as plain
                // markdown; html hosts keep the html rewrite.
                if let Some(md) = markdown {
                    PickRewrite::RichMarkdownHost(format!(
                        "{}\n\n{picked_md}",
                        mark_picked_button(md, picked_idx)
                    ))
                } else {
                    PickRewrite::RichHost(format!(
                        "{}\n\n{picked_html}",
                        mark_picked_button(full, picked_idx)
                    ))
                }
            } else {
                // Classic hosts keep their buttons as reply markup (not in
                // the body) — the empty-markup arm strips those; nothing to
                // rewrite here.
                PickRewrite::ClassicHost(format!("{full}\n\n{picked_html}"))
            }
        }
        None => PickRewrite::Standalone(picked_html.to_string()),
    }
}

/// #67 tap-redraw: rewrite the follow-up button rows of a rich merged host
/// to the post-tap state. The tapped button flips to success style, gains a
/// ✓ and `disabled`; every sibling follow-up button loses its style (bare
/// `disabled` is the only reliably grayed form — #71) and gains `disabled`.
/// Non-follow-up markup passes through byte-identical.
pub(crate) fn mark_picked_button(html: &str, picked_idx: usize) -> String {
    let mut out = String::with_capacity(html.len() + 64);
    let mut rest = html;
    while let Some(tag_start) = rest.find("<tg-button") {
        let after_open = &rest[tag_start + "<tg-button".len()..];
        // `<tg-button-row>` and any other `<tg-button…` lookalike that is
        // not the button tag itself passes through untouched.
        if !after_open.starts_with('>') && !after_open.starts_with(' ') {
            out.push_str(&rest[..tag_start + "<tg-button".len()]);
            rest = after_open;
            continue;
        }
        let Some(attrs_rel) = after_open.find('>') else {
            // Unterminated tag: emit the remainder verbatim.
            out.push_str(rest);
            return out;
        };
        let tag_open_len = "<tg-button".len();
        let attrs = &after_open[..attrs_rel];
        let after_attrs = &after_open[attrs_rel + 1..];
        let body_rel = attrs_rel + 1;
        let Some(label_rel) = after_attrs.find("</tg-button>") else {
            // Unterminated label: emit through the tag opener verbatim.
            out.push_str(&rest[..tag_start + tag_open_len + body_rel]);
            rest = after_attrs;
            continue;
        };
        let label = &after_attrs[..label_rel];
        let tail = &after_attrs[label_rel + "</tg-button>".len()..];
        let idx = attrs
            .split("data=\"followup:")
            .nth(1)
            .and_then(|v| v.split('"').next())
            .and_then(|v| v.rsplit(':').next())
            .and_then(|v| v.parse::<usize>().ok());
        let picked = idx == Some(picked_idx);
        if picked || idx.is_some() {
            let mut new_attrs = attrs.to_string();
            if picked {
                // The picked button flips to success regardless of its
                // original style.
                if let Some(s) = new_attrs.find("style=\"")
                    && let Some(e) = new_attrs[s + "style=\"".len()..].find('"')
                {
                    let style_end = s + "style=\"".len() + e;
                    new_attrs.replace_range(s + "style=\"".len()..style_end, "success");
                }
            } else {
                // #71: a styled button still renders enabled-looking even
                // with `disabled` (owner A/B: `style="primary" disabled`
                // stays blue), so siblings drop their style attribute —
                // bare `disabled` is the only form that renders grayed.
                if let Some(s) = new_attrs.find(" style=\"")
                    && let Some(e) = new_attrs[s + " style=\"".len()..].find('"')
                {
                    let style_end = s + " style=\"".len() + e + 1;
                    new_attrs.replace_range(s..style_end, "");
                }
            }
            if !new_attrs.contains("disabled") {
                new_attrs.push_str(" disabled");
            }
            out.push_str(&rest[..tag_start]);
            out.push_str("<tg-button");
            out.push_str(&new_attrs);
            out.push('>');
            if picked {
                out.push_str("\u{2713} ");
            }
            out.push_str(label);
            out.push_str("</tg-button>");
        } else {
            // Not a follow-up button: emit the whole span verbatim.
            out.push_str(&rest[..tag_start + tag_open_len + body_rel + label_rel]);
            out.push_str("</tg-button>");
        }
        rest = tail;
    }
    out.push_str(rest);
    out
}

/// Button-width calibration, measured 2026-08-25 on Alexey's client
/// Fold threshold: a label wider than this never rides a button — the
/// whole set folds to [`SuggestLayout::NumberedProse`]. Recalibrated
/// 2026-09-04 (fork issue #79 owner smokes): the old 50-char gate shipped
/// clipped Cyrillic — measured cuts at 40-44 chars rich-plane, 32
/// wide-glyph native-plane, and a byte-identical label flipped clean/cut
/// across reads, so no char-count cap is provably safe at any precision.
/// The constant is conservative by design: 20 sits below every cut
/// datapoint observed on any plane. Units are worst-case (wide-glyph)
/// display width; plain `chars().count()` overcounts slim-glyph labels,
/// which errs safe.
pub(crate) const BUTTON_LABEL_MAX_UNITS: usize = 20;
/// Total width one row of buttons may carry before the set folds (issue
/// #79: 3x12=36 shared-row cut, 4x12=43 cut; only a 24 slim-tail pair
/// held — same conservative-by-design rule as [`BUTTON_LABEL_MAX_UNITS`]).
pub(crate) const SHARED_ROW_TOTAL_UNITS: usize = 20;
/// Longest label allowed to share one row with its siblings. Recalibrated
/// 2026-08-31 (live probes, fork issue #49): the real constraint is
/// ROW-TOTAL width (~32 chars shared equally across the row, before the
/// body-width-dependent truncation Alexey observed), not per-label chars —
/// 4×8=32 held, 15+18=33 clipped ~2 symbols, and bubble width varies with
/// the message body, so 12 keeps a safety margin under the worst bubble
/// while doubling information per row vs the old 8.
pub(crate) const SHARED_ROW_MAX_CHARS: usize = 12;
/// Tap ergonomics (Alexey, 2026-08-25): numbered buttons never pack more
/// than 4 per row, so every target stays big enough for a finger.
pub(crate) const MAX_NUMBERS_PER_ROW: usize = 4;

/// Which shape the suggestion controls take for a given option list.
/// Tiers are measured, not guessed — see [`BUTTON_LABEL_MAX_UNITS`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SuggestLayout {
    /// Every label short AND few options: all buttons share ONE row.
    SharedRow,
    /// Every label fits a full-width button: one button per row.
    Column,
    /// Some label too long even full-width: texts fold into the message
    /// body as a numbered list, buttons collapse to bare numbers packed
    /// [`MAX_NUMBERS_PER_ROW`] per row.
    NumberedProse,
}

pub(crate) fn pick_layout(options: &[String]) -> SuggestLayout {
    let width = |o: &String| o.chars().count();
    let total: usize = options.iter().map(&width).sum();
    if options.len() <= MAX_NUMBERS_PER_ROW
        && options.iter().all(|o| width(o) <= SHARED_ROW_MAX_CHARS)
        && total <= SHARED_ROW_TOTAL_UNITS
    {
        SuggestLayout::SharedRow
    } else if options.iter().all(|o| width(o) <= BUTTON_LABEL_MAX_UNITS) {
        SuggestLayout::Column
    } else {
        SuggestLayout::NumberedProse
    }
}

/// The folded option list as rich HTML. REUSES the canonical inline
/// primitives from `super::markdown` — `escape_html` → `format_inline`,
/// the exact pair the outbound renderer's default line branch applies —
/// instead of a private formatter. Options are independent ONE-line texts,
/// so they deliberately skip document-level interpretation (a stray `|`
/// must not turn the list into a table); inline markup (`code`, bold) and
/// HTML escaping behave identically to every other Telegram surface.
/// No "Suggested next" header — the list rides directly under the answer
/// text in the same bubble (#tg-suggest-merge), so the label would only
/// duplicate what the buttons already say.
pub(crate) fn folded_list_html(options: &[String]) -> String {
    options
        .iter()
        .enumerate()
        .map(|(i, opt)| {
            format!(
                "{}. {}",
                i + 1,
                super::markdown::format_inline(&super::markdown::escape_html(opt))
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// NumberedProse fold for the markdown plane (#79 piece 4): a plain
/// numbered list — the server's markdown render keeps the numbering and
/// the line breaks, no `<p>` wrapping needed on this plane.
pub(crate) fn folded_list_markdown(options: &[String]) -> String {
    options
        .iter()
        .enumerate()
        .map(|(i, opt)| format!("{}. {}", i + 1, opt))
        .collect::<Vec<_>>()
        .join("\n")
}

/// host-aware stale-shell strip: the #597 clear killed the stash, but the
/// buttons keep rendering inside the body until the body is rewritten clean.
pub(crate) fn strip_button_rows(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find("<tg-button-row>") {
        match rest[start..].find("</tg-button-row>") {
            Some(rel) => {
                let end = start + rel + "</tg-button-row>".len();
                out.push_str(&rest[..start]);
                rest = &rest[end..];
            }
            None => break, // unterminated span: leave the remainder untouched
        }
    }
    out.push_str(rest);
    out.trim_end().to_string()
}

/// #59 (DRY): the empty reply-markup used to strip dead keyboards — one
/// construction site instead of one per strip arm.
pub(crate) fn empty_keyboard() -> teloxide::types::InlineKeyboardMarkup {
    teloxide::types::InlineKeyboardMarkup::new(
        Vec::<Vec<teloxide::types::InlineKeyboardButton>>::new(),
    )
}

/// #79 chokepoint: EVERY rich body that may carry `<tg-button-row>`
/// controls passes through here on send and edit (the `rich/api.rs`
/// funnel), so hand-authored button rows from any lane get the same
/// measured fit rule the suggestion cards enforce at their own emitter —
/// the n8n-card cut class. If every label fits [`BUTTON_LABEL_MAX_UNITS`]
/// and every row's total fits [`SHARED_ROW_TOTAL_UNITS`], the body ships
/// byte-identical. Otherwise the whole set folds to the NumberedProse
/// shape (proven cut-free twice in #79): buttons keep their attributes —
/// callback data and URL routing are untouched — but render their 1-based
/// index, and the original labels move into an `<ol>` after the last row.
/// Idempotent: a folded body's digit labels never re-trigger the fold.
/// Width is the raw character count of the (already escaped) label text;
/// entities overcount display width, which errs safe.
pub(crate) fn enforce_button_fit(html: &str) -> String {
    const ROW_OPEN: &str = "<tg-button-row>";
    const ROW_CLOSE: &str = "</tg-button-row>";
    const BTN_OPEN: &str = "<tg-button";
    const BTN_CLOSE: &str = "</tg-button>";

    let width = |s: &str| s.chars().count();

    // Pass 1 — collect row spans, per-row open tags, and all labels.
    let mut rows: Vec<(usize, usize)> = Vec::new();
    let mut open_tags: Vec<Vec<&str>> = Vec::new();
    let mut labels: Vec<&str> = Vec::new();
    let mut fits = true;
    let mut scan_from = 0usize;
    while let Some(rel) = html[scan_from..].find(ROW_OPEN) {
        let row_start = scan_from + rel;
        let Some(crel) = html[row_start..].find(ROW_CLOSE) else {
            break; // unterminated row: leave the remainder untouched
        };
        let row_end = row_start + crel + ROW_CLOSE.len();
        let block = &html[row_start + ROW_OPEN.len()..row_end - ROW_CLOSE.len()];
        let mut row_total = 0usize;
        let mut tags_in_row: Vec<&str> = Vec::new();
        let mut bscan = 0usize;
        while let Some(brel) = block[bscan..].find(BTN_OPEN) {
            let bstart = bscan + brel;
            // Skip a prefix hit like `<tg-button-row>` itself.
            let after = &block[bstart + BTN_OPEN.len()..];
            if !after.starts_with(' ') && !after.starts_with('>') {
                bscan = bstart + BTN_OPEN.len();
                continue;
            }
            let Some(orel) = after.find('>') else {
                break;
            };
            let open_tag = &block[bstart..bstart + BTN_OPEN.len() + orel];
            let label_start = bstart + BTN_OPEN.len() + orel + 1;
            let Some(lrel) = block[label_start..].find(BTN_CLOSE) else {
                break;
            };
            let label = &block[label_start..label_start + lrel];
            row_total += width(label);
            fits &= width(label) <= BUTTON_LABEL_MAX_UNITS;
            tags_in_row.push(open_tag);
            labels.push(label);
            bscan = label_start + lrel + BTN_CLOSE.len();
        }
        fits &= row_total <= SHARED_ROW_TOTAL_UNITS;
        rows.push((row_start, row_end));
        open_tags.push(tags_in_row);
        scan_from = row_end;
    }
    if rows.is_empty() || fits {
        return html.to_string();
    }

    // Pass 2 — fold: index digits on the buttons, labels into an `<ol>`.
    let mut out = String::with_capacity(html.len() + 64);
    let mut pos = 0usize;
    let mut index = 0usize;
    let last_row = rows.len() - 1;
    for (i, &(row_start, row_end)) in rows.iter().enumerate() {
        out.push_str(&html[pos..row_start]);
        out.push_str(ROW_OPEN);
        for tag in &open_tags[i] {
            index += 1;
            out.push_str(tag);
            out.push('>');
            out.push_str(&index.to_string());
            out.push_str(BTN_CLOSE);
        }
        out.push_str(ROW_CLOSE);
        pos = row_end;
        if i == last_row {
            out.push_str("\n<ol>");
            for label in &labels {
                out.push_str("<li>");
                out.push_str(label);
                out.push_str("</li>");
            }
            out.push_str("</ol>");
        }
    }
    out.push_str(&html[pos..]);
    out
}

/// #108: button rows and the trailer must start a FRESH markdown block —
/// a single leading newline lets the server parser fuse the preceding
/// paragraph into the controls block, rendering it indented. Guarantees a
/// blank-line separator regardless of whether the body already ends in
/// a newline.
fn push_blank_line(md: &mut String) {
    if !md.ends_with('\n') {
        md.push('\n');
    }
    md.push('\n');
}

/// Shared markdown-plane tail construction (#79 piece 4 + #31 + #108):
/// folded list (prose mode) + blank-line-separated in-body button rows +
/// trailer. One helper so the merge arm and the cross-turn glue arm stay
/// byte-identical in construction — and so the block-boundary property is
/// unit-testable (rows and trailer must start a FRESH markdown block; a
/// single newline fuses the preceding paragraph into the controls block and
/// the server renders it indented).
pub(crate) fn append_rows_and_trailer_md(
    md: &mut String,
    options: &[String],
    token: &str,
    prose: bool,
    trailer: Option<&str>,
) {
    if prose {
        // Markdown plane: plain numbered list — the server's
        // markdown render keeps numbering and line breaks.
        md.push('\n');
        md.push_str(&folded_list_markdown(options));
    }
    push_blank_line(md);
    md.push_str(&suggestion_rows_rich_html(options, token));
    if let Some(t) = trailer {
        push_blank_line(md);
        md.push_str(t);
    }
}

/// The suggestion controls as native rich-button rows (Bot API 10.3
/// `<tg-button-row>`), laid out per the measured ladder. Primary style
/// throughout — picked over app-default after Alexey compared both live.
/// Callback payloads stay `followup:<session>:<idx>`, so taps route through
/// the existing callback dispatcher unchanged regardless of surface.
pub(crate) fn suggestion_rows_rich_html(options: &[String], token: &str) -> String {
    let btn = |i: usize, label: &str| {
        format!(
            "<tg-button type=\"callback_data\" data=\"{FOLLOWUP_PREFIX}{token}:{i}\" \
             style=\"primary\">{}</tg-button>",
            super::markdown::escape_html(label)
        )
    };
    match pick_layout(options) {
        SuggestLayout::SharedRow => format!(
            "<tg-button-row>{}</tg-button-row>",
            options
                .iter()
                .enumerate()
                .map(|(i, opt)| btn(i, opt))
                .collect::<String>()
        ),
        SuggestLayout::Column => options
            .iter()
            .enumerate()
            .map(|(i, opt)| format!("<tg-button-row>{}</tg-button-row>", btn(i, opt)))
            .collect::<Vec<_>>()
            .join("\n"),
        SuggestLayout::NumberedProse => (0..options.len())
            .map(|i| btn(i, &(i + 1).to_string()))
            .collect::<Vec<_>>()
            .chunks(MAX_NUMBERS_PER_ROW)
            .map(|c| format!("<tg-button-row>{}</tg-button-row>", c.concat()))
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

#[allow(clippy::too_many_arguments)] // #31: trailer rides the existing arg set
pub(crate) async fn render_suggestions(
    bot: &teloxide::Bot,
    state: &Arc<TelegramState>,
    session_id: Uuid,
    chat_id: ChatId,
    thread_id: Option<ThreadId>,
    options: Vec<String>,
    // Merge candidate captured by deliver_final_response: the bubble the final
    // response landed in, whatever surface sent it (classic HTML, or table-free
    // rich markdown). Some = attach the controls to THAT bubble — one message
    // instead of two, no "Suggested next" header. None or failed edit =
    // standalone fallback below.
    merge_host: Option<super::state::MergeBubble>,
    // #31: the post-halt sign-off run reclaimed from the flow (after the
    // suggest_options Tool entry). Rich merge: embedded as a paragraph AFTER
    // the in-body button rows (one message, never removed). Every other
    // shape: its own bubble after placement — content, not chrome, so it
    // ships even when the buttons die.
    trailer: Option<String>,
) {
    if options.is_empty() {
        // Stash cleared between delivery and render (mid-turn tap, newer
        // turn) — the trailer still ships (#31); there is nothing to
        // register and no keyboard to place.
        if let Some(t) = &trailer {
            send_trailer_bubble(bot, chat_id, thread_id, t).await;
        }
        return;
    }

    // Per-keyboard identity (#1217): register BEFORE building buttons so the
    // opaque token rides in every callback payload; taps resolve against this
    // exact set even when a newer turn registers its own keyboard meanwhile.
    let token = state
        .register_pending_followups(session_id, options.clone())
        .await;

    // Layout tiers are measured, not guessed (see BUTTON_LABEL_MAX_UNITS):
    // short
    // labels share one row, medium labels get a full-width row each, and
    // anything longer folds into the body as a numbered list with compact
    // number buttons (<=4 per row). The absolute index is encoded in the
    // callback data; the option text itself can exceed Telegram's 64-byte
    // callback-data limit, so we never put it there.
    let layout = pick_layout(&options);
    let text_btn = |i: usize, opt: &str| {
        InlineKeyboardButton::callback(opt.to_string(), format!("{FOLLOWUP_PREFIX}{token}:{i}"))
    };
    let num_btn = |i: usize| {
        InlineKeyboardButton::callback((i + 1).to_string(), format!("{FOLLOWUP_PREFIX}{token}:{i}"))
    };
    let rows: Vec<Vec<InlineKeyboardButton>> = match layout {
        SuggestLayout::SharedRow => vec![
            options
                .iter()
                .enumerate()
                .map(|(i, opt)| text_btn(i, opt))
                .collect(),
        ],
        SuggestLayout::Column => options
            .iter()
            .enumerate()
            .map(|(i, opt)| vec![text_btn(i, opt)])
            .collect(),
        SuggestLayout::NumberedProse => {
            let all: Vec<InlineKeyboardButton> = (0..options.len()).map(num_btn).collect();
            all.chunks(MAX_NUMBERS_PER_ROW)
                .map(|c| c.to_vec())
                .collect()
        }
    };

    let keyboard = InlineKeyboardMarkup::new(rows);

    // Primary path: MERGE onto the answer bubble (#tg-suggest-merge). Prose
    // mode appends the numbered list under the answer text; button modes add
    // nothing on the classic surface (the buttons carry everything). Rich
    // bubbles additionally get native <tg-button-row> controls INSIDE the
    // message body. Both placement payloads are built ONCE — before the first
    // attempt — so a Retry-After deferral (#30) re-sends byte-identical
    // content instead of re-deriving it.
    let merge_payload: Option<MergePayload> = merge_host.map(|host| {
        let mid = host.message_id;
        // Base body + plane: classic bubbles keep their exact delivered
        // HTML; markdown-plane hosts (#79 piece 4) keep the raw markdown —
        // including tables, which the markdown plane renders intact
        // server-side (C1 probe) where rich HTML input flattens them
        // (#679). The html conversion of the merged payload is kept only
        // as the host record's strip source.
        let (mut new_html, rich, new_markdown) = match host.body {
            super::state::BubbleBody::Html(html) => {
                let mut body = html;
                if layout == SuggestLayout::NumberedProse {
                    // Classic hosts preserve the raw newline join via
                    // folded_list_html.
                    body.push('\n');
                    body.push_str(folded_list_html(&options).trim_start());
                }
                (body, false, None)
            }
            super::state::BubbleBody::Markdown(md) => {
                let mut new_md = md;
                append_rows_and_trailer_md(
                    &mut new_md,
                    &options,
                    &token,
                    layout == SuggestLayout::NumberedProse,
                    trailer.as_deref(),
                );
                let strip_source = super::rich::markdown_to_html_p(&new_md);
                (strip_source, true, Some(new_md))
            }
        };
        if rich && new_markdown.is_none() {
            // Legacy html-plane rich host: the rows ride the html dialect
            // as before (#1226 newline handling applies).
            new_html.push('\n');
            new_html.push_str(&suggestion_rows_rich_html(&options, &token));
            if let Some(t) = &trailer {
                new_html.push('\n');
                new_html.push_str(&super::rich::markdown_to_html_p(t));
            }
        }
        MergePayload {
            message_id: mid,
            new_html,
            rich,
            new_markdown,
        }
    });

    // Standalone fallback (no merge candidate, or the edit lost a race / grew
    // too old): the header sentence is still gone per #tg-suggest-merge —
    // prose mode shows just the numbered list, button modes need SOME text
    // for the Bot API to accept the message, so they degrade to the bare 💡.
    let standalone_body = standalone_fallback_body(&layout, &options);

    let option_count = options.len();
    match place_once(
        bot,
        state,
        chat_id,
        thread_id,
        &token,
        option_count,
        &keyboard,
        merge_payload.as_ref(),
        &standalone_body,
    )
    .await
    {
        Ok(()) => {
            // #31: send the trailer bubble only if it wasn't already embedded
            // in a rich merge. The followup host is attached only on merge
            // success, so peek_followup_host tells us if the merge landed.
            // If it did and was rich, the trailer is already in the HTML.
            let embedded = state
                .peek_followup_host(&token)
                .await
                .map(|h| h.rich && trailer.is_some())
                .unwrap_or(false);
            if !embedded && let Some(t) = &trailer {
                send_trailer_bubble(bot, chat_id, thread_id, t).await;
            }
        }
        Err(PlaceErr::Fatal(e)) => {
            tracing::warn!("Telegram suggest_options: send failed: {e}");
            // The buttons never landed — drop the stash so a stale entry can't
            // swallow an unrelated future tap.
            state.drop_pending_followup(&token).await;
        }
        Err(PlaceErr::RetryAfter(wait)) => {
            // #30: a 429 here used to drop the stash at once — but BOTH arms
            // die inside the same flood window (the standalone send followed
            // the merge edit by 22ms into the same 41s ban), and the buttons
            // were lost forever. Keep the stash instead and re-place after
            // the TRUE Retry-After, budget-capped.
            tracing::warn!(
                "Telegram suggest_options: placement hit Retry-After {}s (token {token}) — \
                 stash kept, deferring",
                wait.as_secs()
            );
            let bot = bot.clone();
            let state = state.clone();
            let token = token.clone();
            let keyboard = keyboard.clone();
            let trailer = trailer.clone();
            tokio::spawn(async move {
                let mut wait = wait;
                for attempt in 1..=MAX_DEFERRED_PLACEMENT_ATTEMPTS {
                    use rand::Rng;
                    // Jitter so several placements deferred by the same ban
                    // don't all re-hit Telegram on the same second.
                    let jitter = Duration::from_millis(rand::rng().random_range(0..=2000));
                    tokio::time::sleep(wait + jitter).await;
                    match place_once(
                        &bot,
                        &state,
                        chat_id,
                        thread_id,
                        &token,
                        option_count,
                        &keyboard,
                        merge_payload.as_ref(),
                        &standalone_body,
                    )
                    .await
                    {
                        Ok(()) => {
                            tracing::info!(
                                "Telegram suggest_options: deferred placement {attempt}/\
                                 {MAX_DEFERRED_PLACEMENT_ATTEMPTS} landed (token {token})"
                            );
                            // #31: send the trailer bubble only if it wasn't
                            // already embedded in a rich merge.
                            let embedded = state
                                .peek_followup_host(&token)
                                .await
                                .map(|h| h.rich && trailer.is_some())
                                .unwrap_or(false);
                            if !embedded && let Some(t) = &trailer {
                                send_trailer_bubble(&bot, chat_id, thread_id, t).await;
                            }
                            return;
                        }
                        Err(PlaceErr::Fatal(e)) => {
                            tracing::warn!(
                                "Telegram suggest_options: deferred placement {attempt} \
                                 failed permanently: {e}"
                            );
                            state.drop_pending_followup(&token).await;
                            return;
                        }
                        Err(PlaceErr::RetryAfter(w)) => {
                            tracing::warn!(
                                "Telegram suggest_options: deferred placement {attempt} hit \
                                 Retry-After {}s again (token {token})",
                                w.as_secs()
                            );
                            wait = w;
                        }
                    }
                }
                tracing::warn!(
                    "Telegram suggest_options: placement budget spent after \
                     {MAX_DEFERRED_PLACEMENT_ATTEMPTS} deferred attempts (token {token}) — dropping"
                );
                state.drop_pending_followup(&token).await;
            });
        }
    }
}

/// Pre-built merge-edit payload (#30): computed ONCE per suggestion block so
/// a deferred re-placement after a Retry-After re-sends byte-identical
/// content. `rich` picks the wire: rich bubbles edit via the rich API with
/// in-body button rows, classic bubbles via editMessageText + reply_markup.
#[derive(Clone)]
struct MergePayload {
    message_id: MessageId,
    new_html: String,
    rich: bool,
    /// Markdown-plane payload (#79 piece 4): when set, the merge edit and
    /// every later redraw ride `edit_rich_markdown`; `new_html` then only
    /// feeds the host record's strip source.
    new_markdown: Option<String>,
}

/// Placement error class (#30): decides whether the stash survives the
/// failure.
enum PlaceErr {
    /// Telegram answered 429 with a Retry-After — the placement may succeed
    /// once the window passes, so the stash MUST survive the wait.
    RetryAfter(Duration),
    /// Anything else: retrying cannot fix it; the stash drops as before.
    Fatal(String),
}

/// Deferred placement attempts after a Retry-After, on top of the inline
/// first pass (#30). Two deferrals cap the chase at roughly two flood
/// windows while comfortably covering the 31–42s windows observed in the
/// #30 ledger.
const MAX_DEFERRED_PLACEMENT_ATTEMPTS: u32 = 2;

/// Wait used when only the rich arm's stringified "(429)" survives — see
/// [`classify_rich_err`].
const RICH_429_FALLBACK_WAIT_SECS: u64 = 30;

fn classify_request_err(e: teloxide::RequestError) -> PlaceErr {
    match e {
        teloxide::RequestError::RetryAfter(secs) => PlaceErr::RetryAfter(secs.duration()),
        other => PlaceErr::Fatal(other.to_string()),
    }
}

/// The rich arm buries Telegram's exact retry_after inside its own internal
/// retry loop (`post_rich`) and surfaces only an anyhow string, so
/// classification keys off the status marker. The wait is a middle-of-the-
/// road default: the rich path already slept out the true value
/// RICH_MAX_RETRIES times before bailing, and the observed flood windows
/// run 31–42s (#30 ledger).
fn classify_rich_err(e: &str) -> PlaceErr {
    if e.contains("(429)") {
        PlaceErr::RetryAfter(Duration::from_secs(RICH_429_FALLBACK_WAIT_SECS))
    } else {
        PlaceErr::Fatal(e.to_string())
    }
}

/// Fence-safe markdown-plane edit (#98): every redraw that re-sends a
/// rich-md host body must preserve a delivered mermaid diagram. When the
/// body carries a mermaid fence, resolve it to markdown+media — the send
/// path's own mechanism (#1044) — and edit via the media-capable rich edit,
/// so the image re-renders instead of degrading to raw fence text.
///
/// - No fence → byte-identical plain `edit_rich_markdown` (no resolve call).
/// - Fence but empty media (every fence degraded to a failure block) →
///   plain edit of the resolved body; the failure block is the legible form.
/// - Media edit rejected with 429 → propagate WITHOUT fallback: retries must
///   re-send the same body (#30 byte-identity), a fallback body would flip
///   the plane mid-retry.
/// - Media edit rejected otherwise → warn + plain edit of the ORIGINAL body,
///   so the keyboard always lands (diagram degrades to pre-#98 fence text,
///   never worse than today).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn edit_rich_md_fencesafe(
    api_url: &str,
    token: &str,
    chat_id: i64,
    message_id: i32,
    markdown: &str,
    reply_markup: Option<&serde_json::Value>,
    origin: &str,
    origin_detail: &str,
) -> Result<(), String> {
    if !super::rich::mermaid::has_mermaid_fence(markdown) {
        return super::rich::api::edit_rich_markdown(
            api_url,
            token,
            chat_id,
            message_id,
            markdown,
            reply_markup,
            origin,
            origin_detail,
        )
        .await
        .map_err(|e| e.to_string());
    }
    let (resolved, media) = super::rich::mermaid::resolve_markdown_media(markdown).await;
    if media.is_empty() {
        return super::rich::api::edit_rich_markdown(
            api_url,
            token,
            chat_id,
            message_id,
            &resolved,
            reply_markup,
            origin,
            origin_detail,
        )
        .await
        .map_err(|e| e.to_string());
    }
    match super::rich::api::edit_rich_markdown_media(
        api_url,
        token,
        chat_id,
        message_id,
        &resolved,
        &media,
        reply_markup,
        origin,
        origin_detail,
    )
    .await
    {
        Ok(()) => Ok(()),
        Err(e) => {
            let s = e.to_string();
            if s.contains("(429)") {
                return Err(s);
            }
            tracing::warn!(
                "Telegram rich edit: media edit rejected ({s}) — plain markdown fallback \
                 (keyboard lands; fence degrades to text)"
            );
            super::rich::api::edit_rich_markdown(
                api_url,
                token,
                chat_id,
                message_id,
                markdown,
                reply_markup,
                origin,
                origin_detail,
            )
            .await
            .map_err(|e| e.to_string())
        }
    }
}

/// One placement pass (#30): merge onto the answer bubble when a payload
/// exists, standalone otherwise. RetryAfter-class errors bubble up so the
/// caller can defer with the stash intact; anything else is Fatal.
#[allow(clippy::too_many_arguments)]
async fn place_once(
    bot: &teloxide::Bot,
    state: &Arc<TelegramState>,
    chat_id: ChatId,
    thread_id: Option<ThreadId>,
    token: &str,
    option_count: usize,
    keyboard: &InlineKeyboardMarkup,
    merge: Option<&MergePayload>,
    standalone_body: &str,
) -> Result<(), PlaceErr> {
    use teloxide::prelude::Requester;

    if let Some(mp) = merge {
        let mid = mp.message_id;
        let outcome: Result<(), PlaceErr> = if let Some(md) = &mp.new_markdown {
            // Markdown-plane host (#79 piece 4): raw markdown + embedded
            // button rows — the server render keeps tables intact (#679).
            // Fence-safe (#98): a delivered mermaid diagram re-renders
            // instead of degrading to raw fence text.
            edit_rich_md_fencesafe(
                bot.api_url().as_str(),
                bot.token(),
                chat_id.0,
                mid.0,
                md,
                None,
                "turn",
                "-",
            )
            .await
            .map_err(|e| classify_rich_err(&e))
        } else if mp.rich {
            super::rich::api::edit_rich_html(
                bot.api_url().as_str(),
                bot.token(),
                chat_id.0,
                mid.0,
                &mp.new_html,
                None,
                "turn",
                "-",
            )
            .await
            .map_err(|e| classify_rich_err(&e.to_string()))
        } else {
            bot.edit_message_text(chat_id, mid, &mp.new_html)
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard.clone())
                .await
                .map(|_| ())
                .map_err(classify_request_err)
        };
        match outcome {
            Ok(()) => {
                // Name the arm, the host message and the token so any tap can
                // be mapped back to its panel from logs alone.
                tracing::info!(
                    "Telegram suggest_options: keyboard merged onto msg {mid} \
                     ({} host, token {token}, {option_count} options)",
                    if mp.new_markdown.is_some() {
                        "rich-md"
                    } else if mp.rich {
                        "rich"
                    } else {
                        "classic"
                    }
                );
                state
                    .attach_followup_host(
                        token,
                        super::state::MergedHost {
                            message_id: mid,
                            html: mp.new_html.clone(),
                            rich: mp.rich,
                            markdown: mp.new_markdown.clone(),
                        },
                    )
                    .await;
                return Ok(());
            }
            Err(PlaceErr::RetryAfter(wait)) => return Err(PlaceErr::RetryAfter(wait)),
            Err(PlaceErr::Fatal(e)) => {
                tracing::warn!(
                    "Telegram suggest_options: merge onto msg {mid} failed ({e}) — standalone fallback"
                );
            }
        }
    }

    let mut req = bot
        .send_message(chat_id, standalone_body)
        .reply_markup(keyboard.clone());
    req = req.parse_mode(ParseMode::Html);
    if let Some(tid) = thread_id {
        req = req.message_thread_id(tid);
    }
    match req.await {
        Ok(msg) => {
            tracing::info!(
                "Telegram suggest_options: standalone block msg {} \
                 (token {token}, {option_count} options)",
                msg.id
            );
            Ok(())
        }
        Err(e) => Err(classify_request_err(e)),
    }
}

/// The #31 sign-off trailer as its own bubble: Markdown rendered with the
/// same HTML wire as every other telegram bubble, thread-routed, with a
/// plain-text retry when the parse-mode send is rejected — a malformed
/// markdown construct must degrade the sign-off, never discard it
/// (keep-never-discard is the whole point of #31).
async fn send_trailer_bubble(
    bot: &teloxide::Bot,
    chat_id: ChatId,
    thread_id: Option<ThreadId>,
    trailer: &str,
) {
    use teloxide::prelude::Requester;

    let html = super::markdown::markdown_to_telegram_html(trailer);
    let mut req = bot.send_message(chat_id, html).parse_mode(ParseMode::Html);
    if let Some(tid) = thread_id {
        req = req.message_thread_id(tid);
    }
    match req.await {
        Ok(msg) => {
            tracing::info!("Telegram: #31 trailer bubble delivered as msg {}", msg.id);
        }
        Err(e) => {
            tracing::warn!("Telegram: #31 trailer bubble HTML send failed ({e}) — retrying plain");
            let mut plain = bot.send_message(chat_id, trailer);
            if let Some(tid) = thread_id {
                plain = plain.message_thread_id(tid);
            }
            match plain.await {
                Ok(msg) => {
                    tracing::info!(
                        "Telegram: #31 trailer bubble delivered plain as msg {}",
                        msg.id
                    );
                }
                Err(e2) => {
                    tracing::warn!("Telegram: #31 trailer bubble dropped after plain retry: {e2}");
                }
            }
        }
    }
}
