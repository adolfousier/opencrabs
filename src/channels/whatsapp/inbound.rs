//! Structured extracts for inbound message types the handler used to drop
//! silently (#1410 video, #1483 sticker / location / contact / reaction).
//!
//! Every one of these arrived, matched none of the image/audio/document arms,
//! and fell through to `return` - so sending the bot a pin of where you are
//! produced nothing at all. The functions here turn each proto into a short
//! line of text the agent can actually reason about. They are pure: no
//! downloads, no client, no I/O, so the shape of what the agent sees is
//! pinned by tests rather than by a live WhatsApp account.

use waproto::whatsapp::message::{
    ContactMessage, LiveLocationMessage, LocationMessage, ReactionMessage,
};

/// Render a location pin as coordinates plus whatever labels came with it.
///
/// Coordinates lead because they are the part an agent can act on; the name
/// and address are frequently absent (a dropped pin has neither).
pub(crate) fn describe_location(loc: &LocationMessage) -> String {
    let lat = loc.degrees_latitude.unwrap_or_default();
    let lng = loc.degrees_longitude.unwrap_or_default();
    let mut out = format!("[location] {lat:.6}, {lng:.6}");
    if let Some(name) = loc.name.as_deref().filter(|n| !n.trim().is_empty()) {
        out.push_str(&format!(" ({})", name.trim()));
    }
    if let Some(addr) = loc.address.as_deref().filter(|a| !a.trim().is_empty()) {
        out.push_str(&format!(" - {}", addr.trim()));
    }
    out
}

/// Render a live-location share. Flagged as live because the coordinates are
/// a snapshot that keeps moving, which changes what the agent should do with
/// them.
pub(crate) fn describe_live_location(loc: &LiveLocationMessage) -> String {
    let lat = loc.degrees_latitude.unwrap_or_default();
    let lng = loc.degrees_longitude.unwrap_or_default();
    let mut out = format!("[live location] {lat:.6}, {lng:.6}");
    if let Some(caption) = loc.caption.as_deref().filter(|c| !c.trim().is_empty()) {
        out.push_str(&format!(" - {}", caption.trim()));
    }
    out
}

/// Pull the first phone number out of a vCard.
///
/// WhatsApp writes `TEL;type=CELL;waid=15551234567:+1 555 123 4567`, but the
/// parameter soup before the colon varies by client, so the split is on the
/// last colon of a `TEL` line rather than on any fixed prefix.
pub(crate) fn phone_from_vcard(vcard: &str) -> Option<String> {
    vcard
        .lines()
        .map(str::trim)
        .find(|line| line.to_ascii_uppercase().starts_with("TEL"))
        .and_then(|line| line.rsplit(':').next())
        .map(str::trim)
        .filter(|phone| !phone.is_empty())
        .map(str::to_string)
}

/// Render a shared contact card as a name plus a number when one is present.
pub(crate) fn describe_contact(contact: &ContactMessage) -> String {
    let name = contact
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or("unnamed");
    match contact.vcard.as_deref().and_then(phone_from_vcard) {
        Some(phone) => format!("[contact] {name} - {phone}"),
        None => format!("[contact] {name}"),
    }
}

/// Render an inbound reaction on one of our messages.
///
/// Returns `None` when the emoji is empty, which is how WhatsApp encodes a
/// reaction being REMOVED. Surfacing that as a reaction with no emoji would
/// read to the agent as approval that was in fact taken back.
pub(crate) fn describe_reaction(reaction: &ReactionMessage) -> Option<String> {
    let emoji = reaction.text.as_deref().map(str::trim).unwrap_or("");
    if emoji.is_empty() {
        return None;
    }
    let target = reaction
        .key
        .as_ref()
        .and_then(|k| k.id.as_deref())
        .unwrap_or("unknown message");
    Some(format!("[reaction] {emoji} on message {target}"))
}
