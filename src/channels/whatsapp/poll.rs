//! Inbound poll votes (#1482).
//!
//! Outbound polls have shipped for a while; votes were never decoded, so the
//! bot could create a poll and then had no idea who picked what. Two things
//! were missing, and neither is the crypto: the lib already implements the
//! whole vote path in `wacore::poll`.
//!
//! 1. Votes reference options by SHA-256 hash, never by name. Turning a vote
//!    back into a label needs the poll's own option list, so the options are
//!    remembered per poll message here.
//! 2. `message_secret` is needed to decrypt, and that part was already solved:
//!    `MsgSecretStore` is implemented against SQLite in `store/msgsecret.rs`,
//!    so the secret is persisted and survives a restart with no new work.

use std::collections::HashMap;

use tokio::sync::Mutex;
use wacore::poll::compute_option_hash;

/// Turn the hashes in a decrypted vote back into the option labels.
///
/// Hashes that match no option are dropped rather than rendered as raw bytes:
/// they mean the voter used "add option" on a poll we did not author, and a
/// wall of hex helps nobody. The order follows the POLL's option order, not
/// the vote's, so two voters picking the same pair read identically.
pub(crate) fn resolve_options(selected: &[Vec<u8>], options: &[String]) -> Vec<String> {
    options
        .iter()
        .filter(|option| {
            let hash = compute_option_hash(option);
            selected.iter().any(|s| s.as_slice() == hash.as_slice())
        })
        .cloned()
        .collect()
}

/// Render a vote for the agent. `None` when nothing resolved, which is a vote
/// being CLEARED (the voter deselected everything) and reads as noise.
pub(crate) fn describe_vote(voter: &str, chosen: &[String]) -> Option<String> {
    if chosen.is_empty() {
        return None;
    }
    Some(format!("[poll vote] {voter} chose: {}", chosen.join(", ")))
}

/// Option lists for polls this bot created, keyed by the poll's message id.
///
/// Bounded the same way as the recent-message window: a poll older than this
/// cannot have its votes labelled, which is reported honestly rather than
/// guessed at.
pub(crate) const POLL_CAPACITY: usize = 100;

#[derive(Default)]
struct Inner {
    order: Vec<String>,
    options: HashMap<String, Vec<String>>,
}

#[derive(Default)]
pub(crate) struct PollOptions {
    inner: Mutex<Inner>,
}

impl PollOptions {
    /// Remember the options of a poll we just sent.
    pub async fn remember(&self, message_id: impl Into<String>, options: Vec<String>) {
        let id = message_id.into();
        if id.is_empty() || options.is_empty() {
            return;
        }
        let mut inner = self.inner.lock().await;
        if inner.options.insert(id.clone(), options).is_some() {
            inner.order.retain(|existing| existing != &id);
        }
        inner.order.push(id);
        while inner.order.len() > POLL_CAPACITY {
            let evicted = inner.order.remove(0);
            inner.options.remove(&evicted);
        }
    }

    /// The options of a poll, if it is still inside the window.
    pub async fn get(&self, message_id: &str) -> Option<Vec<String>> {
        self.inner.lock().await.options.get(message_id).cloned()
    }

    /// How many polls are tracked. Exists for the tests that pin eviction.
    #[cfg(test)]
    pub async fn len(&self) -> usize {
        self.inner.lock().await.options.len()
    }
}

/// Decode one inbound poll vote into a line for the agent (#1482).
///
/// `None` covers every "nothing useful to say" case, and each one is logged
/// at debug rather than swallowed: the poll is outside the tracked window,
/// the `message_secret` is not in the store, the ciphertext does not open, or
/// the voter deselected everything. A vote that cannot be labelled is worth
/// less than silence, because a wall of hashes reads to the agent as data.
pub(crate) async fn decode_vote(
    client: &whatsapp_rust::client::Client,
    state: &super::WhatsAppState,
    update: &waproto::whatsapp::message::PollUpdateMessage,
    chat_jid: &wacore_binary::jid::Jid,
    voter_jid: &wacore_binary::jid::Jid,
    voter_label: &str,
) -> Option<String> {
    let key = update.poll_creation_message_key.as_ref()?;
    let poll_id = key.id.as_deref()?;

    let Some(options) = state.polls.get(poll_id).await else {
        tracing::debug!(
            target: "whatsapp",
            poll_id,
            "poll vote for a poll outside the tracked window; cannot label the options"
        );
        return None;
    };

    let vote = update.vote.as_ref()?;
    let (payload, iv) = (vote.enc_payload.as_deref()?, vote.enc_iv.as_deref()?);

    // The secret is keyed on the poll's own chat and author. We only track
    // polls WE created, so the author is this account.
    let chat = chat_jid.to_non_ad().to_string();
    let creator = key
        .remote_jid
        .clone()
        .unwrap_or_else(|| chat_jid.to_string());
    let secret = match client
        .persistence_manager()
        .backend()
        .get_msg_secret(&chat, &creator, poll_id)
        .await
    {
        Ok(Some(secret)) => secret,
        Ok(None) => {
            tracing::debug!(
                target: "whatsapp",
                poll_id,
                "no message secret stored for this poll; vote cannot be decrypted"
            );
            return None;
        }
        Err(e) => {
            tracing::warn!(
                target: "whatsapp",
                error = %e,
                poll_id,
                "message-secret lookup failed; poll vote dropped"
            );
            return None;
        }
    };

    let creator_jid = key
        .remote_jid
        .as_deref()
        .and_then(|j| j.parse::<wacore_binary::jid::Jid>().ok())
        .unwrap_or_else(|| chat_jid.clone());
    let selected = match client
        .polls()
        .decrypt_vote(
            wacore::poll::PollVoteCiphertext {
                enc_payload: payload,
                enc_iv: iv,
            },
            &secret,
            poll_id,
            &creator_jid,
            voter_jid,
        )
        .await
    {
        Ok(selected) => selected,
        Err(e) => {
            tracing::warn!(
                target: "whatsapp",
                error = %e,
                poll_id,
                "poll vote failed to decrypt"
            );
            return None;
        }
    };

    describe_vote(voter_label, &resolve_options(&selected, &options))
}
