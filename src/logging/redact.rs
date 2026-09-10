//! Secret redaction for anything on its way into a log.
//!
//! The Telegram Bot API puts the bot token in the URL path, and `reqwest`'s
//! error `Display` carries the failing URL, so `tracing::warn!("...: {e}")` on
//! any Telegram call writes the token to disk in plaintext (#1322). Anyone
//! with read access to the log directory then has the bot, as does anyone
//! handed a log excerpt in a bug report.
//!
//! Nothing at those call sites is doing anything unusual: logging a transport
//! error is right, and the token rides along invisibly. That is what makes a
//! per-site fix unreliable, and there are over a hundred such sites. So the
//! redaction lives at the writer instead, where every event passes through
//! whatever its origin, and [`scrub`] is exposed for call sites that want to
//! redact earlier.

use std::borrow::Cow;

use regex::Regex;

/// `bot<id>:<secret>` as it appears in a Bot API URL. The id is the bot's
/// public numeric id and stays legible; only the secret half is replaced, so a
/// redacted line still says which bot failed.
///
/// The secret is at least 20 chars of the token alphabet, which is short
/// enough to catch every real token and long enough that ordinary prose
/// (`bot1:ok`) is left alone.
fn token_re() -> &'static Regex {
    use std::sync::OnceLock;
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"bot(\d+):[A-Za-z0-9_-]{20,}").expect("bot-token pattern is a valid regex")
    })
}

/// What a redacted token is replaced with, kept as one constant so a test can
/// assert on it without restating the format.
pub(crate) const REDACTED: &str = ":<redacted>";

/// `text` with any Telegram bot token replaced by `bot<id><REDACTED>`.
///
/// Borrows when there is nothing to redact, which is the overwhelming majority
/// of log lines, so the common path allocates nothing.
pub(crate) fn scrub(text: &str) -> Cow<'_, str> {
    // Telegram bot tokens first: they have their own shape (bot<id>:<secret>)
    // and their own replacement that keeps the public bot id for diagnosis. The
    // "bot" cheap-reject keeps this off nearly every line.
    let tg: Cow<'_, str> = if text.contains("bot") {
        token_re().replace_all(text, format!("bot${{1}}{REDACTED}").as_str())
    } else {
        Cow::Borrowed(text)
    };

    // Broad secret scrub on top (OC-05): provider keys (sk-, xoxb-, ghp_, …),
    // Authorization: Bearer, PEM blocks — everything the channel-output redactor
    // covers, now at the log writer too and unconditional. The log scrub used to
    // catch only Telegram bot tokens, so any other credential that rode a
    // provider error's Display landed in the daily log in plaintext.
    match crate::utils::sanitize::redact_secrets_for_logs(&tg) {
        // Broad pass changed something: take its owned result.
        Cow::Owned(s) => Cow::Owned(s),
        // Broad pass found nothing: keep whatever the Telegram pass produced.
        Cow::Borrowed(_) => tg,
    }
}

/// True when `text` still carries something shaped like a bot token. Exists so
/// tests can assert the negative without duplicating the pattern.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn contains_token(text: &str) -> bool {
    token_re().is_match(text)
}
