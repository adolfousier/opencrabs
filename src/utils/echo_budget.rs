//! One home for the #1221 echo-card body budgets, per wire leg (#490).
//!
//! Two layers need the same answer and must never disagree:
//!
//! * the Telegram card render (`channels::telegram::resume`), which applies
//!   the budget of the wire it actually chose, and
//! * the sender-facing delivery verdict (`brain::tools::subagent::notify`),
//!   which has to TELL the sending lane its payload was shortened. Before
//!   #490 it read a bare `Delivered` while the tail was already gone, so the
//!   sending lane could not learn that its closing blocks never arrived.
//!
//! Deliberately NOT under `channels::telegram`: the notify tool compiles with
//! the `telegram` feature off, so it cannot reach into a feature-gated channel
//! module — a budget the verdict quotes has to live somewhere unconditional.
//!
//! The counts come from [`crate::utils::string::truncate_chars_tail_preserving`]
//! itself, never from a second copy of its arithmetic: a re-derived formula
//! drifts the moment either end changes, and the whole point of the signal is
//! that the warning MATCHES the cut the reader will see.

use crate::utils::string::truncate_chars_tail_preserving;

/// Body budget for the **classic** wire. Classic `sendMessage` caps a message
/// at 4096 chars; header, tags and Telegram's own margin eat the rest. This
/// governs the classic fallback only — a body bound for a rich wire takes
/// [`ECHO_BODY_CAP_CHARS_RICH`] instead (#490).
pub const ECHO_BODY_CAP_CHARS: usize = 3200;

/// Body budget for the **rich** wire (#490). `sendRichMessage` carries ~32K
/// chars — eight times the classic cap — so the classic 3200 budget was cutting
/// the tail (the Disclosures and What-now/next blocks) off notify cards
/// Telegram would have accepted whole. Sized just under the rich ceiling to
/// leave room for the card's own `<details>`/`<summary>` chrome, matching the
/// `flow` chrome's own 30000 guard.
pub const ECHO_BODY_CAP_CHARS_RICH: usize = 30_000;

/// What each wire leg would drop from one body, beside the body's own size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverCap {
    /// The body's length in characters — the unit the budgets are written in.
    pub total: usize,
    /// Characters the rich leg would drop; `0` when the body fits whole.
    pub rich_dropped: usize,
    /// Characters the classic fallback would drop; `0` when it fits whole.
    pub classic_dropped: usize,
}

impl OverCap {
    /// True when NO leg shortens the body — a verdict then stays bare.
    pub fn fits_everywhere(&self) -> bool {
        self.rich_dropped == 0 && self.classic_dropped == 0
    }
}

/// Measure one body against both wire budgets.
pub fn over_cap(body: &str) -> OverCap {
    OverCap {
        total: body.chars().count(),
        rich_dropped: truncate_chars_tail_preserving(body, ECHO_BODY_CAP_CHARS_RICH).1,
        classic_dropped: truncate_chars_tail_preserving(body, ECHO_BODY_CAP_CHARS).1,
    }
}

/// The sender-facing over-cap signal for a delivery verdict (#490), or an
/// empty string when neither leg would drop anything.
///
/// Empty-on-fit is deliberate: every body that fits keeps the verdict it had,
/// and the warning appears exactly when a sending lane would otherwise be
/// relying on material that is gone. Both legs are named because the leg is
/// not known at verdict time — the card takes the rich wire and falls back to
/// the classic one only if that call fails, so the honest report is what each
/// leg WOULD do with this body.
pub fn sender_over_cap_signal(body: &str) -> String {
    let cap = over_cap(body);
    if cap.fits_everywhere() {
        return String::new();
    }
    format!(
        " Payload {} chars — rich leg ({} budget) drops {}; classic fallback ({} budget) drops {}.",
        cap.total,
        ECHO_BODY_CAP_CHARS_RICH,
        cap.rich_dropped,
        ECHO_BODY_CAP_CHARS,
        cap.classic_dropped
    )
}
