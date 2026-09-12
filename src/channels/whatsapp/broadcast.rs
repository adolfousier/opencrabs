//! Safety rails for the broadcast surfaces (#1485).
//!
//! Newsletter posts, status updates and labels are things no other channel we
//! support can do at all. They are also the highest-signal spam pattern Meta
//! watches, and the number IS the bot: there is no token to rotate if it gets
//! banned. So the rails are part of the feature rather than a follow-up.
//!
//! Two rules, both enforced here:
//!
//! * **Pacing.** Every broadcast send waits a configurable minimum, defaulting
//!   to a deliberately slow 5 seconds. N sends therefore take at least N times
//!   that, and there is no fire-and-forget mass-send path at all.
//! * **Opt-in.** A broadcast target must be on an explicit allowlist. The tool
//!   refuses an arbitrary number rather than trusting the caller to have
//!   checked, because the caller here is a language model.

use std::time::Duration;

/// Deliberately slow default: broadcast is not a hot path, and the cost of
/// being wrong is the number, not a retry.
pub(crate) const DEFAULT_DELAY_SECONDS: u64 = 5;

/// Strip a phone number down to digits so `+1 (555) 123-4567`, `+15551234567`
/// and `15551234567@s.whatsapp.net` all compare equal. An allowlist that only
/// matched one formatting would be an allowlist in name only.
pub(crate) fn normalize(target: &str) -> String {
    let head = target.split('@').next().unwrap_or(target);
    head.chars().filter(char::is_ascii_digit).collect()
}

/// Minimum wait between two broadcast sends.
///
/// A configured `0` does NOT disable pacing; it falls back to the default.
/// Turning the rail off is not a knob this feature offers, because a config
/// typo must not become a mass-send.
pub(crate) fn delay(configured: Option<u64>) -> Duration {
    match configured {
        Some(seconds) if seconds > 0 => Duration::from_secs(seconds),
        _ => Duration::from_secs(DEFAULT_DELAY_SECONDS),
    }
}

/// May this target receive a broadcast?
///
/// An EMPTY allowlist allows nobody. That is the opposite of the usual
/// "empty means everyone" convention, and it is deliberate: an unconfigured
/// install must not be one tool call away from messaging arbitrary numbers.
pub(crate) fn is_allowed(target: &str, allowlist: &[String]) -> bool {
    let target = normalize(target);
    if target.is_empty() {
        return false;
    }
    allowlist.iter().any(|entry| normalize(entry) == target)
}

/// Wall-clock floor for a burst of `count` sends, used to state the cost up
/// front in the tool result instead of letting it be a surprise.
pub(crate) fn burst_floor(count: usize, delay: Duration) -> Duration {
    delay * (count.saturating_sub(1)) as u32
}

/// Split a target list into those that may be messaged and those that may not,
/// so the refusal can name the numbers instead of failing opaquely.
pub(crate) fn partition<'a>(
    targets: &'a [String],
    allowlist: &[String],
) -> (Vec<&'a String>, Vec<&'a String>) {
    targets
        .iter()
        .partition(|target| is_allowed(target, allowlist))
}

/// WhatsApp's own default status background (dark green), as 0xAARRGGBB.
/// A status must carry one; picking the client's own default keeps a
/// bot-posted status visually indistinguishable from a hand-typed one.
pub(crate) const STATUS_BACKGROUND_ARGB: u32 = 0xFF_1E_6E_4F;

/// Turn an allowlist entry into a JID.
///
/// Entries are written by a human in config.toml, so they arrive as
/// `+1 (555) 123-4567` as often as `+15551234567`. `None` means the entry
/// held no digits at all, which is a config typo worth refusing loudly
/// rather than silently skipping.
pub(crate) fn target_jid(target: &str) -> Option<wacore_binary::jid::Jid> {
    let digits = normalize(target);
    if digits.is_empty() {
        return None;
    }
    format!("{digits}@s.whatsapp.net").parse().ok()
}
