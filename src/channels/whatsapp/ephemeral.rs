//! Disappearing-message TTL resolution (#1487).
//!
//! `SendOptions::ephemeral_expiration` sets `contextInfo.expiration` on an
//! outgoing message, which is how WhatsApp makes it disappear on both ends.
//! The channel has always left it unset. A caller can now ask for a TTL per
//! message, or the channel can carry a default for every send.

/// WhatsApp's own disappearing-message presets, in seconds. Anything else is
/// accepted too; these are named so a caller does not have to remember them.
pub(crate) const TTL_24_HOURS: u32 = 86_400;
pub(crate) const TTL_7_DAYS: u32 = 604_800;
pub(crate) const TTL_90_DAYS: u32 = 7_776_000;

/// Longest TTL WhatsApp accepts. A larger value is clamped rather than
/// rejected: the caller asked for "as long as possible" and refusing the send
/// over it would be worse than honouring the maximum.
pub(crate) const TTL_MAX: u32 = TTL_90_DAYS;

/// Resolve the TTL for one outbound message.
///
/// `requested` is the per-call value from the tool payload, `default_ttl` the
/// channel-wide setting. An explicit `0` means "not disappearing" and beats
/// the channel default, which is why this is not a plain `or`.
pub(crate) fn resolve(requested: Option<u64>, default_ttl: Option<u32>) -> Option<u32> {
    match requested {
        Some(0) => None,
        Some(seconds) => Some(seconds.min(u64::from(TTL_MAX)) as u32),
        None => match default_ttl {
            Some(0) | None => None,
            Some(seconds) => Some(seconds.min(TTL_MAX)),
        },
    }
}

/// Human-readable form for the tool result, so the agent can report what it
/// actually sent rather than echoing a raw second count.
pub(crate) fn describe(ttl: u32) -> String {
    match ttl {
        TTL_24_HOURS => "24 hours".to_string(),
        TTL_7_DAYS => "7 days".to_string(),
        TTL_90_DAYS => "90 days".to_string(),
        s if s % 86_400 == 0 => format!("{} days", s / 86_400),
        s if s % 3_600 == 0 => format!("{} hours", s / 3_600),
        s if s % 60 == 0 => format!("{} minutes", s / 60),
        s => format!("{s} seconds"),
    }
}
