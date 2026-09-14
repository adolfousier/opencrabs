//! Intermediate-message delivery: standalone narration posts (rich-first
//! with HTML fallback), footer append to the last intermediate, the
//! rate-limit-retrying send wrapper and the HTML-or-plain send.
//!
//! Moved VERBATIM out of handler.rs (#471 phase 1, pure decomposition —
//! only visibility widened to pub(crate) so the handler glob re-export
//! keeps every existing call site and test import stable).

use super::handler::StreamingState;
use super::markdown::{markdown_to_telegram_html, split_message, strip_html_tags};
use super::send::message_in_thread;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{MessageId, ParseMode, ReplyParameters};

/// Send an HTML message, falling back to plain text if Telegram rejects the HTML.
/// Returns the resulting `MessageId` so callers that need to track or later delete
/// the message (e.g. intermediate cleanup on cancellation) can do so.
/// Build the edited message body for appending the ctx/tok-s footer to the
/// last intermediate message.
///
/// Used when a turn's final response text deduped to empty because all of
/// it was already delivered as intermediate messages (the common tool-
/// using case). Rather than drop the footer (which left the user never
/// seeing ctx budget on Telegram — 2026-06-06) or send a standalone
/// footer bubble (removed in 7a0ca1c9), we edit the last intermediate
/// message to carry the footer inline.
///
/// Reconstructs the last chunk exactly as it was originally sent
/// (`markdown_to_telegram_html` + `split_message(_, 4096)` then `.last()`),
/// appends the footer, and returns `None` when:
/// - the footer or intermediate text is empty, OR
/// - the combined result would exceed Telegram's 4096-char cap (never
///   truncate real content to make room for metadata).
///
/// Pure + free function so the fit/reconstruct logic is unit-testable
/// without a live bot.
// Channel-unused since the ctx footer moved onto the flow message (the
// intermediate-footer append path went with the pre-block status bubble);
// kept because the reconstruct-last-chunk logic is nontrivial and its tests
// pin the split/fit contract meanwhile.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn build_last_intermediate_with_footer(
    last_intermediate_text: &str,
    footer: &str,
) -> Option<String> {
    if footer.is_empty() || last_intermediate_text.is_empty() {
        return None;
    }
    let html = markdown_to_telegram_html(last_intermediate_text);
    let chunks = split_message(&html, 4096);
    let last_chunk = chunks.last()?;
    let combined = format!("{last_chunk}\n\n{footer}");
    if combined.chars().count() > 4096 {
        None
    } else {
        Some(combined)
    }
}

/// Send a structured intermediate segment as a native rich message, returning
/// its id for tracking. Mermaid-aware (#1044/#1202): fences resolve to a media
/// array; with no fence this is byte-identical to `send_rich_markdown_id`.
/// Returns `None` when the text carries no rich structure
/// or the rich API rejects it — the caller then falls back to the HTML path.
pub(crate) async fn try_send_intermediate_rich(
    bot: &Bot,
    chat_id: ChatId,
    thread_id: Option<teloxide::types::ThreadId>,
    text: &str,
) -> Option<MessageId> {
    if !super::rich::should_send_native_rich(text) {
        return None;
    }
    // Mermaid-aware sender (#1044/#1202): resolves fences into the rich
    // markdown media array; byte-identical to send_rich_markdown_id when no
    // fence is present, so non-diagram reports are unaffected.
    match super::rich::send_rich_with_mermaid_id(
        bot.api_url().as_str(),
        bot.token(),
        chat_id.0,
        thread_id,
        text,
        None,
        "turn",
        "-",
    )
    .await
    {
        Ok(id) => Some(MessageId(id)),
        Err(e) => {
            tracing::warn!("Telegram: intermediate rich send failed, using HTML: {e}");
            None
        }
    }
}

/// True when an intermediate message contains a substantial markdown status report
/// (e.g. status/progress/pipeline heading or substantial section) worth delivering
/// as its own message rather than burying in collapsible flow (#215).
pub(crate) fn is_deliverable_status_report(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }

    let total_chars = trimmed.chars().count();
    let line_count = text.lines().filter(|l| !l.trim().is_empty()).count();

    let mut in_fence = false;
    let mut fence_char = ' ';
    let mut fence_len = 0;
    let mut has_keyword_heading = false;
    let mut has_atx_heading = false;
    let mut has_status_callout = false;

    const STATUS_KEYWORDS: &[&str] = &[
        "status",
        "update",
        "progress",
        "pipeline",
        "summary",
        "verification",
        "verdict",
        "phase",
        "findings",
        "plan",
        "results",
    ];

    for line in text.lines() {
        let t = line.trim_start();
        let leading_spaces = line.len() - t.len();
        if leading_spaces < 4 {
            let fence_run: String = t.chars().take_while(|&c| c == '`' || c == '~').collect();
            if fence_run.len() >= 3 {
                let f_char = fence_run.chars().next().unwrap();
                if !in_fence {
                    in_fence = true;
                    fence_char = f_char;
                    fence_len = fence_run.len();
                    continue;
                } else if f_char == fence_char && fence_run.len() >= fence_len {
                    in_fence = false;
                    continue;
                }
            }
        }
        if in_fence {
            continue;
        }

        if super::rich::is_atx_heading(t) {
            has_atx_heading = true;
            let lower = t.to_lowercase();
            if STATUS_KEYWORDS.iter().any(|kw| lower.contains(kw)) {
                has_keyword_heading = true;
            }
        }

        // Rule C — structured callout: non-empty line starting with `>` and `**` followed by status/update/progress
        let trimmed_line = line.trim();
        if let Some(after_gt) = trimmed_line.strip_prefix('>') {
            let after_gt = after_gt.trim_start();
            if let Some(after_stars) = after_gt.strip_prefix("**") {
                let lower_callout = after_stars.to_lowercase();
                if lower_callout.starts_with("status")
                    || lower_callout.starts_with("update")
                    || lower_callout.starts_with("progress")
                {
                    has_status_callout = true;
                }
            }
        }
    }

    // Rule A — keyword heading: line_count >= 2 and total_chars >= 50
    if has_keyword_heading && line_count >= 2 && total_chars >= 50 {
        return true;
    }

    // Rule B — substantial section: any ATX heading, line_count >= 3 and total_chars >= 150
    if has_atx_heading && line_count >= 3 && total_chars >= 150 {
        return true;
    }

    // Rule C — structured callout: line_count >= 2 and total_chars >= 60
    if has_status_callout && line_count >= 2 && total_chars >= 60 {
        return true;
    }

    false
}

/// True when a folded intermediate is a substantial rich report worth
/// delivering as its OWN message rather than burying in the collapsed
/// processing log (#582). Keyed on a real markdown table plus some length, so
/// thin narration (no table) keeps folding and only report-shaped content —
/// which the model may emit before a tool call (e.g. text + `plan complete` in
/// one step) — is surfaced.
pub(crate) fn is_deliverable_rich_report(text: &str) -> bool {
    // #690 follow-up (#980): a table collapsed onto ONE line is invisible to
    // contains_table (which needs the header and separator each on their own
    // line), so a collapsed report would fail this gate and get buried in the
    // folded log as raw pipes. Reflow first — the same recovery the final-
    // response and HTML-render paths already apply. Idempotent.
    let reflowed = super::rich::reflow_collapsed_tables(text);
    // A mermaid fence (tagged or content-classified, #1202) is report-shaped
    // on its own: folding buries the diagram behind a tap-to-expand tap AND
    // leaves raw fence text in the log, because neither the fold renderer nor
    // the pre-fix rich path resolved fences. Surfaced intermediates go
    // through deliver_intermediate_message, which now resolves them.
    if super::rich::mermaid::has_mermaid_fence(&reflowed) {
        return true;
    }
    if super::rich::contains_table(&reflowed) && text.trim().chars().count() >= 200 {
        return true;
    }
    is_deliverable_status_report(text)
}

/// Deliver `text` as its own message (rich-first, HTML fallback) and record it
/// in `sent_intermediates` so the final-response dedup will not resend it.
/// Returns true when something was delivered. Used to surface a rich report the
/// model emitted before a tool call, which folding would otherwise bury (#582).
pub(crate) async fn deliver_intermediate_message(
    bot: &Bot,
    chat: ChatId,
    thread_id: Option<teloxide::types::ThreadId>,
    streaming: &Arc<std::sync::Mutex<StreamingState>>,
    tg: &super::state::TelegramState,
    text: &str,
) -> bool {
    // #690 follow-up (#980): re-expand a collapsed table once, up front, so the
    // dedup record, the rich send and the HTML fallback all see the same
    // expanded shape. The HTML path reflows again internally but is idempotent.
    let expanded = super::rich::reflow_collapsed_tables(text);
    let text = expanded.as_str();
    {
        let s = streaming.lock().unwrap_or_else(|e| e.into_inner());
        if s.sent_intermediates.iter().any(|prev| prev == text) {
            return true;
        }
    }
    if let Some(id) = try_send_intermediate_rich(bot, chat, thread_id, text).await {
        // The bubble is non-sticky burial evidence (#1150): the flow block must
        // restick below its own output on the next append.
        tg.note_bot_bubble(chat.0, id.0);
        let mut s = streaming.lock().unwrap_or_else(|e| e.into_inner());
        s.sent_intermediates.push(text.to_string());
        s.intermediate_msg_ids.push(id);
        return true;
    }
    // Resolve fences here too (#1142 parity): when the rich path rejected the
    // message, the HTML fallback must still render the diagram instead of
    // shipping raw fence text. Identical to markdown_to_telegram_html when
    // the feature is off or no fence is present.
    let html = super::rich::markdown_to_html_mermaid(text).await;
    if html.is_empty() {
        return false;
    }
    let mut sent_ids: Vec<MessageId> = Vec::new();
    for chunk in split_message(&html, 4096) {
        match send_html_or_plain(bot, chat, thread_id, chunk, "turn", None).await {
            Ok(id) => {
                tg.note_bot_bubble(chat.0, id.0);
                sent_ids.push(id);
            }
            Err(e) => {
                tracing::warn!("Telegram: rich-intermediate send failed ({e})");
                return false;
            }
        }
    }
    let mut s = streaming.lock().unwrap_or_else(|e| e.into_inner());
    s.sent_intermediates.push(text.to_string());
    s.intermediate_msg_ids.extend(sent_ids);
    true
}

/// Threshold for treating a Telegram 429 as a "long rate-limit" (#1110).
///
/// When Telegram returns `Retry-After: N` where N > this threshold, the chat
/// is flood-banned for hours (28442s = 7.9 hours observed). Retrying the
/// send ladder burns 90 seconds (3 × 30s clamped wait) for no gain: the
/// window won't clear in that time. Instead, bail immediately and let the
/// caller surface the rate-limit to the user.
///
/// One hour is the boundary: typical flood windows (placeholder-edit churn,
/// command bursts) are seconds and stay under the inline cap. Anything over
/// an hour is a multi-hour ban, not a throttle.
const LONG_RATE_LIMIT_THRESHOLD: std::time::Duration = std::time::Duration::from_secs(3600);

/// Run a Telegram send, waiting out `RetryAfter` (429) up to 3 attempts.
///
/// Command replies are programmatic: a per-chat rate limit (typically a
/// streaming turn editing its placeholder into the same chat) must DELAY
/// them, never drop them. The command branches used a bare `.await?`, so
/// the 429 propagated out of the handler and the reply vanished with a
/// single error log line — /models looked "stuck" while a turn streamed
/// and worked right after it completed (#297). Non-429 errors and
/// exhausted retries still propagate to the caller.
///
/// Long rate-limits (>1 hour) bail immediately without retrying (#1110).
pub(crate) async fn send_retrying_rate_limit<T, F, Fut>(
    what: &str,
    mut send: F,
) -> std::result::Result<T, teloxide::RequestError>
where
    F: FnMut() -> Fut,
    Fut: std::future::IntoFuture<Output = std::result::Result<T, teloxide::RequestError>>,
{
    const MAX_RETRIES: u32 = 3;
    let mut attempt = 0u32;
    loop {
        match send().await {
            Err(teloxide::RequestError::RetryAfter(secs)) => {
                let requested = secs.duration();
                // Long rate-limit (>1 hour): bail immediately, don't retry (#1110).
                // The chat is flood-banned for hours; retrying burns 90s for no gain.
                if requested > LONG_RATE_LIMIT_THRESHOLD {
                    tracing::error!(
                        "Telegram: {what} long rate-limit ({}s > {}s threshold) — bailing immediately, \
                         no retry ladder (#1110)",
                        requested.as_secs(),
                        LONG_RATE_LIMIT_THRESHOLD.as_secs()
                    );
                    return Err(teloxide::RequestError::RetryAfter(secs));
                }
                if attempt < MAX_RETRIES {
                    attempt += 1;
                    super::rate_limit::wait_out(
                        what,
                        requested,
                        &format!(" (attempt {attempt}/{MAX_RETRIES})"),
                    )
                    .await;
                } else {
                    tracing::error!(
                        "Telegram: {what} still rate-limited after {MAX_RETRIES} retries ({}s) — giving up",
                        requested.as_secs()
                    );
                    return Err(teloxide::RequestError::RetryAfter(secs));
                }
            }
            // No success line here (review F1): the wrapper is generic and
            // has no correlation fields, so its line carried nothing the
            // chokepoint telemetry doesn't already say with full fields.
            other => return other,
        }
    }
}

pub(crate) async fn send_html_or_plain(
    bot: &Bot,
    chat_id: ChatId,
    thread_id: Option<teloxide::types::ThreadId>,
    html: &str,
    origin: &str,
    reply_to: Option<i32>,
) -> std::result::Result<MessageId, teloxide::RequestError> {
    // G3 send pacing (#1211): the universal outbox ladder funnels cron
    // deliveries, tool sends and chunked replies through here, so the
    // ~1/s + 18/min per-forum pacer applies at this one seam. DMs pass
    // through untouched; pacing delays, never drops (#297).
    super::governor::pace_send(chat_id).await;
    // Correlation telemetry (#1085 P1a, review F8): this is the chokepoint
    // carrying chunked final replies, command acks and error notices.
    // `origin` is threaded by the caller (turn | tool | cron | system) so
    // an outbox/cron send is never mislabeled "turn". Every exit logs;
    // metadata only, never content.
    let thread = thread_id.map(|t| t.0.0);
    let hash8 = super::telemetry::content_hash8(html);
    let len = html.len();
    let log_ok = |path: &str, m: &MessageId, len: usize, hash8: &str| {
        super::telemetry::log_send_success(
            origin,
            "-",
            "-",
            "html_or_plain",
            path,
            chat_id.0,
            thread,
            m.0,
            len,
            hash8,
        );
    };
    // HTML rides the shared retry ladder (#1085 P1b R1): up to 3 attempts
    // with `rate_limit::wait_out` between them (#297 delay-never-drop),
    // matching every other send path — previously this hand-rolled a single
    // retry. Only a final failure falls back to plain text, and the
    // fallback rides the same ladder so a 429 cannot drop it either.
    // `reply_to` (optional) attaches Telegram reply_parameters so the same
    // seam carries tool-reply targeting without a separate writer (#1230).
    match send_retrying_rate_limit("HTML send", || {
        let mut req = message_in_thread(bot, chat_id, thread_id, html);
        if let Some(mid) = reply_to {
            req = req.reply_parameters(ReplyParameters::new(MessageId(mid)));
        }
        req.parse_mode(ParseMode::Html)
    })
    .await
    {
        Ok(m) => {
            log_ok("html", &m.id, len, &hash8);
            Ok(m.id)
        }
        Err(e) => {
            tracing::warn!("Telegram: HTML send failed after retries ({e}), sending as plain text");
            let plain = strip_html_tags(html);
            // Review F2: hash and len must describe the text that actually
            // landed on the wire (the stripped plain text), not the HTML
            // source — a duplicate-correlation query must match payloads.
            let plain_hash8 = super::telemetry::content_hash8(&plain);
            let plain_len = plain.len();
            match send_retrying_rate_limit("plain fallback", || {
                let mut req = message_in_thread(bot, chat_id, thread_id, plain.as_str());
                if let Some(mid) = reply_to {
                    req = req.reply_parameters(ReplyParameters::new(MessageId(mid)));
                }
                req
            })
            .await
            {
                Ok(m) => {
                    log_ok("plain_fallback", &m.id, plain_len, &plain_hash8);
                    Ok(m.id)
                }
                Err(e2) => {
                    super::telemetry::log_send_failure(
                        origin,
                        "-",
                        "-",
                        "html_or_plain",
                        "plain_fallback",
                        chat_id.0,
                        thread,
                        plain_len,
                        &plain_hash8,
                        &e2.to_string(),
                    );
                    Err(e2)
                }
            }
        }
    }
}
