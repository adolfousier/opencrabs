//! Local mirror of the WhatsApp blocklist (#1487).
//!
//! The server already stops delivering messages from a blocked contact, so
//! this is a second line rather than the only one. It exists because the tool
//! can block a number mid-session and the inbound guard must react on the very
//! next message without a round trip per message: `Blocking::is_blocked` calls
//! `get_blocklist`, which is a server query, and running that on every inbound
//! stanza would be absurd.
//!
//! The mirror is refreshed from the server on connect and updated in place on
//! every block/unblock the bot performs. It is deliberately keyed on the user
//! part of the JID: a block applies to the whole account, never to one device.

use std::collections::HashSet;

use tokio::sync::RwLock;

/// The user part of a JID, which is what a block actually applies to.
/// `15551234@s.whatsapp.net/3` and `15551234@s.whatsapp.net` are the same
/// account, and a device suffix must not smuggle a blocked contact through.
pub(crate) fn user_part(jid: &str) -> &str {
    let jid = jid.split('/').next().unwrap_or(jid);
    jid.split('@').next().unwrap_or(jid)
}

/// Blocked accounts, by JID user part.
#[derive(Default)]
pub(crate) struct Blocklist {
    users: RwLock<HashSet<String>>,
}

impl Blocklist {
    /// Replace the mirror wholesale, as after a `get_blocklist` at connect.
    pub async fn replace(&self, jids: impl IntoIterator<Item = String>) {
        let fresh: HashSet<String> = jids
            .into_iter()
            .map(|j| user_part(&j).to_string())
            .collect();
        *self.users.write().await = fresh;
    }

    /// Record a block the bot just performed.
    pub async fn insert(&self, jid: &str) {
        self.users.write().await.insert(user_part(jid).to_string());
    }

    /// Record an unblock the bot just performed.
    pub async fn remove(&self, jid: &str) {
        self.users.write().await.remove(user_part(jid));
    }

    /// Is this sender blocked? Device suffixes are ignored.
    pub async fn contains(&self, jid: &str) -> bool {
        self.users.read().await.contains(user_part(jid))
    }

    /// How many accounts are mirrored. Exists for the tests that assert
    /// `replace` swaps the set rather than merging into it.
    #[cfg(test)]
    pub async fn len(&self) -> usize {
        self.users.read().await.len()
    }
}
