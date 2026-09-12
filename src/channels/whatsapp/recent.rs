//! Bounded memory of recently seen messages (#1484).
//!
//! `Client::forward_message` takes the original `wa::Message`, not an id: it
//! rebuilds the body with the forward flags set and relays media from the same
//! CDN blob rather than re-uploading. So forwarding needs the proto we saw,
//! and the channel kept none of them - inbound messages were read for their
//! text and dropped.
//!
//! This keeps the last N, oldest evicted first. Media protos carry URLs and
//! decryption keys, never the blob itself, so the entries stay small. A
//! forward request for something older than the window fails with a clear
//! message instead of silently sending nothing.

use std::collections::{HashMap, VecDeque};

use tokio::sync::Mutex;
use waproto::whatsapp::Message;

/// How many messages to keep. Large enough that "forward the photo from
/// earlier" works across a normal conversation, small enough to be invisible
/// in memory.
pub(crate) const DEFAULT_CAPACITY: usize = 200;

#[derive(Default)]
struct Inner {
    /// Insertion order, for eviction.
    order: VecDeque<String>,
    by_id: HashMap<String, Message>,
}

/// The last `capacity` messages the channel saw, keyed by message id.
pub(crate) struct RecentMessages {
    inner: Mutex<Inner>,
    capacity: usize,
}

impl Default for RecentMessages {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }
}

impl RecentMessages {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            // A zero capacity would make every insert evict itself, which
            // reads as "forward is broken" rather than "forward is off".
            capacity: capacity.max(1),
        }
    }

    /// Remember a message, evicting the oldest once the window is full.
    ///
    /// Re-remembering an id refreshes the stored proto and moves it to the
    /// newest position, so a message that is still being referenced does not
    /// age out while in use.
    pub async fn remember(&self, id: impl Into<String>, message: Message) {
        let id = id.into();
        if id.is_empty() {
            return;
        }
        let mut inner = self.inner.lock().await;
        if inner.by_id.insert(id.clone(), message).is_some() {
            inner.order.retain(|existing| existing != &id);
        }
        inner.order.push_back(id);
        while inner.order.len() > self.capacity {
            if let Some(evicted) = inner.order.pop_front() {
                inner.by_id.remove(&evicted);
            }
        }
    }

    /// The stored proto for an id, if it is still inside the window.
    pub async fn get(&self, id: &str) -> Option<Message> {
        self.inner.lock().await.by_id.get(id).cloned()
    }

    /// How many messages are held. Exists for the tests that pin eviction.
    #[cfg(test)]
    pub async fn len(&self) -> usize {
        self.inner.lock().await.by_id.len()
    }
}
