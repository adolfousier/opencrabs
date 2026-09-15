//! Remember what each rich message was last edited to, so we stop asking
//! Telegram to replace a message with itself.
//!
//! The rich edit path re-sends identical content on every refresh. Telegram
//! answers `400: message is not modified`, which the caller logged as a send
//! failure. In one evening of ordinary group use that produced dozens of WARN
//! lines, each one a round trip whose only purpose was to be told nothing had
//! changed, spent against the same budget that was already backing off
//! elsewhere in the session (#1443).
//!
//! The fingerprint covers the reply markup as well as the text, because that
//! is the pair Telegram compares. Hashing the text alone would skip an edit
//! that only swaps a keyboard, which is a real change.
//!
//! A fingerprint is recorded only once an edit has actually landed. Recording
//! at attempt time would let a failed edit convince the next identical attempt
//! that it was redundant, and the message would keep whatever it had.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Mutex, OnceLock};

/// Bound on remembered messages. Far above the number of cards live in a busy
/// chat, and cheap to rebuild: a miss costs one edit that Telegram answers
/// `not modified`, which is exactly the pre-fix behaviour for that one call.
const MAX_TRACKED: usize = 512;

type Key = (i64, i32);

fn tracked() -> &'static Mutex<HashMap<Key, u64>> {
    static TRACKED: OnceLock<Mutex<HashMap<Key, u64>>> = OnceLock::new();
    TRACKED.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The `(chat_id, message_id, fingerprint)` an edit body addresses, or `None`
/// when the body is not an addressed edit.
///
/// The fingerprint spans the rich payload and the reply markup together, so
/// changing either one is a different edit.
pub(crate) fn fingerprint(body: &serde_json::Value) -> Option<(i64, i32, u64)> {
    let chat_id = body.get("chat_id")?.as_i64()?;
    let message_id = i32::try_from(body.get("message_id")?.as_i64()?).ok()?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    body.get("rich_message")
        .map(ToString::to_string)
        .hash(&mut hasher);
    body.get("reply_markup")
        .map(ToString::to_string)
        .hash(&mut hasher);
    Some((chat_id, message_id, hasher.finish()))
}

/// Would this edit leave the message exactly as it already is?
pub(crate) fn is_redundant(chat_id: i64, message_id: i32, fingerprint: u64) -> bool {
    tracked()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&(chat_id, message_id))
        == Some(&fingerprint)
}

/// Record the state a message is now in, after an edit that landed.
///
/// Also called when Telegram reports the content was already identical: that
/// answer is itself confirmation of the message's state, and recording it is
/// what stops the next refresh asking again.
pub(crate) fn remember(chat_id: i64, message_id: i32, fingerprint: u64) {
    let mut map = tracked().lock().unwrap_or_else(|e| e.into_inner());
    if map.len() >= MAX_TRACKED && !map.contains_key(&(chat_id, message_id)) {
        // Nothing here is worth an eviction policy: dropping the lot costs one
        // redundant edit per live message, once.
        map.clear();
    }
    map.insert((chat_id, message_id), fingerprint);
}

/// Telegram's way of saying the edit was a no-op.
pub(crate) fn is_not_modified(description: &str) -> bool {
    description.contains("message is not modified")
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn clear_for_test() {
    tracked().lock().unwrap_or_else(|e| e.into_inner()).clear();
}
