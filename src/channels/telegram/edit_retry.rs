//! Shared Retry-After handling for best-effort UI-edit legs (#68).
//!
//! Before this module every decoration leg (tap pick-record, courtesy
//! notes) fired exactly once: a Telegram 429 killed it, the immediate
//! fallback fired ~100ms later INTO the same flood window and was dead by
//! construction, and the user saw a consumed tap with no visible effect
//! (incident 2026-09-01 18:17:26Z: pick-record edit 429 "Retry after 12s",
//! echo fallback 429 105ms later, bubble showed nothing).
//!
//! Model, proven by #30's deferred placement: attempt 1 fires immediately;
//! on a Retry-After the caller spawns a deferred task that sleeps EXACTLY
//! the server-instructed window (capped — nothing blocks while it waits),
//! re-fires the SAME prebuilt payload once, and only on exhaustion hands
//! off to the legacy fallback — which by then runs safely outside the 429
//! window.

use std::future::Future;
use std::time::Duration;

use teloxide::Bot;
use teloxide::payloads::{EditMessageTextSetters, SendMessageSetters};
use teloxide::prelude::Requester;
use teloxide::types::{ChatId, InlineKeyboardMarkup, MessageId, ParseMode};

/// Hard ceiling on a server-instructed deferral. Nothing user-facing is
/// blocked while the deferred task sleeps (the tap flow and the turn
/// continue), so this only bounds how stale a decoration may get — chosen
/// above the 31–42s windows observed in the #30 ledger.
pub const MAX_DEFERRED_WAIT: Duration = Duration::from_secs(35);

/// Wait used when only a stringified "(429)" marker survives (the rich arm
/// buries the true retry_after inside its own internal retry loop) — same
/// middle-of-the-road default as suggest_options (#30).
const RICH_429_FALLBACK_WAIT_SECS: u64 = 30;

/// Error class for a best-effort edit (#68): a Retry-After defers one
/// identical retry; anything else is final.
pub enum EditErr {
    /// Telegram answered 429 with a Retry-After — retry once after the wait.
    RetryAfter(Duration),
    /// Retrying cannot fix it (bad markup, message gone, ...): fall back now.
    Fatal(String),
}

/// Classify a typed teloxide error.
pub fn classify(e: &teloxide::RequestError) -> EditErr {
    match e {
        teloxide::RequestError::RetryAfter(secs) => EditErr::RetryAfter(secs.duration()),
        other => EditErr::Fatal(other.to_string()),
    }
}

/// Classify an already-stringified error. Call sites whose arms fold the
/// typed error into a `String` before returning (the tap pick-record arms
/// map `e.to_string()`) lose the enum, so classification keys off the wire
/// surfaces: teloxide renders `RequestError::RetryAfter` as
/// `"Retry after <n>s"`, and the rich arm buries `"(429)"`.
pub fn classify_str(e: &str) -> EditErr {
    if let Some(rest) = e
        .strip_prefix("Retry after ")
        .and_then(|r| r.strip_suffix('s'))
        && let Ok(secs) = rest.parse::<u64>()
    {
        return EditErr::RetryAfter(Duration::from_secs(secs));
    }
    if e.contains("(429)") {
        return EditErr::RetryAfter(Duration::from_secs(RICH_429_FALLBACK_WAIT_SECS));
    }
    EditErr::Fatal(e.to_string())
}

/// Spawn the deferred retry (#68). The caller continues immediately — the
/// callback ack, the tap flow and the turn are never blocked. After the
/// (capped) server-instructed wait the SAME payload is re-fired once; on
/// exhaustion the caller's fallback finally runs, now safely outside the
/// 429 window that killed both the edit and the old immediate fallback.
pub fn spawn_deferred<E, F, Fut, G, Fut2>(wait: Duration, retry: F, exhausted: G)
where
    E: std::fmt::Display + Send + 'static,
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), E>> + Send + 'static,
    G: FnOnce() -> Fut2 + Send + 'static,
    Fut2: Future<Output = ()> + Send + 'static,
{
    let wait = wait.min(MAX_DEFERRED_WAIT);
    tokio::spawn(async move {
        tokio::time::sleep(wait).await;
        match retry().await {
            Ok(()) => {
                tracing::info!("Telegram: deferred UI-edit retry landed after {wait:?} wait");
            }
            Err(e) => {
                tracing::warn!(
                    "Telegram: deferred UI-edit retry exhausted after {wait:?} wait ({e}) — running fallback"
                );
                exhausted().await;
            }
        }
    });
}

/// The dominant callback-UI edit shape (#62 fold): editMessageText with
/// optional HTML parse mode and reply markup, warn on death. Attempt now;
/// on Retry-After defer ONE identical retry past the window (the #68
/// model); Fatal or exhausted retry → warn with the site label. Fire and
/// forget — the callback ack and the handler flow are never blocked.
pub fn edit_text_ui(
    bot: Bot,
    chat_id: ChatId,
    message_id: MessageId,
    text: String,
    parse_html: bool,
    markup: Option<InlineKeyboardMarkup>,
    label: &'static str,
) {
    let bot_for_fire = bot.clone();
    let text_for_fire = text.clone();
    let markup_for_fire = markup.clone();
    let fire = move || {
        let bot = bot_for_fire.clone();
        let text = text_for_fire.clone();
        let markup = markup_for_fire.clone();
        async move {
            let mut req = bot.edit_message_text(chat_id, message_id, &text);
            if parse_html {
                req = req.parse_mode(ParseMode::Html);
            }
            if let Some(kb) = markup {
                req = req.reply_markup(kb);
            }
            req.await.map(|_| ())
        }
    };
    tokio::spawn(async move {
        // G2 governor admission for interactive UI edits (#117 / #171):
        // spends reserved floor token and clears any queued conflicting final.
        let _ = super::governor::edit_admission(
            &bot,
            chat_id,
            message_id,
            super::governor::EditClass::Interactive,
            String::new(),
            false,
        )
        .await;

        match fire().await {
            Ok(()) => {}
            Err(e) => match classify(&e) {
                EditErr::RetryAfter(wait) => {
                    let bot_fb = bot.clone();
                    let text_fb = text.clone();
                    let markup_fb = markup.clone();
                    spawn_deferred(wait, fire, move || async move {
                        tracing::warn!(
                            "Telegram: {label} deferred retry exhausted (still rate-limited) — falling back to fresh send"
                        );
                        let mut req = bot_fb.send_message(chat_id, &text_fb);
                        if parse_html {
                            req = req.parse_mode(ParseMode::Html);
                        }
                        if let Some(kb) = markup_fb {
                            req = req.reply_markup(kb);
                        }
                        if let Err(e) = req.await {
                            tracing::warn!("Telegram: {label} fallback send failed: {e}");
                        }
                    });
                }
                EditErr::Fatal(msg) => {
                    tracing::warn!("Telegram: {label} failed: {msg}");
                }
            },
        }
    });
}
