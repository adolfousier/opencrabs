//! Which of a sender's two addresses to key on (#1533).
//!
//! WhatsApp addresses a message either by phone number (PN, `@s.whatsapp.net`)
//! or by an opaque privacy id (LID, `@lid`), and the parser hands back the
//! OTHER one in `sender_alt`. From `wacore/src/messages.rs:539`:
//!
//! ```text
//! let sender_alt = if participant.server.is_pn_family() {
//!     attrs.optional_jid("participant_lid")   // sender is PN, alt is the LID
//! } else if participant.server.is_lid_family() {
//!     attrs.optional_jid("participant_pn")    // sender is LID, alt is the PN
//! }
//! ```
//!
//! So `sender_alt` is whichever identity the sender is NOT. That makes
//! "prefer `sender_alt`" a trap: it reads as "prefer the phone number" and is
//! only that for a LID-addressed sender. For a PN-addressed one it prefers the
//! LID, which is the opposite of the intent, and it looks correct in any test
//! where the sender happens to arrive as a LID.
//!
//! Anything used as a MAP KEY has to pick one of the pair and pick the same one
//! every time, or a contact whose addressing mode changes between two stanzas
//! gets two entries: two sessions, an approval that can never be resolved, a
//! photo album that never batches. The rule below keys on the server family
//! rather than on which field the value arrived in.

use wacore_binary::jid::Jid;

/// The sender's phone-number identity when the pair has one, else the sender.
///
/// A PN is preferred because it is the identity everything else in the channel
/// is written in: `allowed_phones`, `bot_owner`, the blocklist, and the session
/// label a human reads. A LID is only ever a fallback for a sender who has no
/// PN twin in the stanza, where the choice is that LID or nothing.
///
/// Both families are asked rather than one being assumed as the negation of the
/// other: `is_pn_family` covers `Pn | Hosted` and `is_lid_family` covers
/// `Lid | HostedLid`, so a two-branch `if/else` on either one alone sorts the
/// hosted variants into the wrong half. Servers in neither family (group,
/// broadcast, newsletter) are not identity pairs at all and keep their own user
/// part.
///
/// Takes the user part only, never the device: `Jid::user` already excludes the
/// `:34` linked-device suffix, so the same person on phone and desktop keys the
/// same way.
pub(crate) fn canonical_user(sender: &Jid, sender_alt: Option<&Jid>) -> String {
    if sender.server.is_pn_family() {
        return sender.user.to_string();
    }
    if sender.server.is_lid_family()
        && let Some(alt) = sender_alt
        && alt.server.is_pn_family()
        && !alt.user.is_empty()
    {
        return alt.user.to_string();
    }
    sender.user.to_string()
}
