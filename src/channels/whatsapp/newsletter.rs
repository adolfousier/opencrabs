//! WhatsApp newsletter poller (#1529).
//!
//! Opt-in channels from `channels.whatsapp.newsletters` are polled on a
//! 15-minute tick spawned per connection. Posts are **never** injected as
//! inbound agent turns — that is #1542's anti-pattern (spoofed inbound) by
//! another name; instead each tick's new posts become one digest message to
//! the owner, and the per-channel cursor advances only after the digest was
//! accepted by the send path.
//!
//! First observation of a channel baselines silently: the cursor is set to
//! the newest post and nothing is sent, so flipping config on never replays
//! historical posts at the owner.
//!
//! The pure decision helpers below are tested in
//! `src/tests/whatsapp_newsletter_test.rs`; the IO leg (`poll_once`) composes
//! them around crate calls the compiler types for us.

use crate::channels::whatsapp::WhatsAppState;
use crate::db::ChannelMessageRepository;
use std::sync::Arc;
use wacore_binary::jid::Jid;

/// The cursor stored per channel: newest digested post's monotonic server id
/// plus its Unix seconds (kept for `before` pagination when a page is ever
/// exhausted).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cursor {
    pub last_server_id: i64,
    pub last_ts: i64,
}

/// A fetched post reduced to what the poll decision needs.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PostRef {
    pub server_id: u64,
    pub ts: u64,
}

/// Posts strictly newer than the cursor, oldest first (digests read
/// chronologically). An absent cursor means "first sight": baseline, nothing
/// new.
pub(crate) fn select_new(cur: Option<Cursor>, posts: &[PostRef]) -> Vec<PostRef> {
    let Some(cur) = cur else { return Vec::new() };
    let mut new: Vec<PostRef> = posts
        .iter()
        .copied()
        .filter(|p| p.server_id as i64 > cur.last_server_id)
        .collect();
    new.sort_by_key(|p| p.server_id);
    new
}

/// The cursor to persist after a tick.
///
/// - Existing cursor: advance to the newest post seen this page (monotonic
///   id + its ts), never regress.
/// - Absent cursor: baseline on the newest post, or an explicit zero cursor
///   when the channel has no posts at all (so the next empty page does not
///   re-baseline forever).
pub(crate) fn next_cursor(cur: Option<Cursor>, posts: &[PostRef]) -> Cursor {
    let newest = posts.iter().max_by_key(|p| p.server_id).copied();
    match (cur, newest) {
        (Some(c), Some(p)) => Cursor {
            last_server_id: c.last_server_id.max(p.server_id as i64),
            last_ts: p.ts as i64,
        },
        (Some(c), None) => c,
        (None, Some(p)) => Cursor {
            last_server_id: p.server_id as i64,
            last_ts: p.ts as i64,
        },
        (None, None) => Cursor {
            last_server_id: 0,
            last_ts: 0,
        },
    }
}

/// Owner-facing digest body. Names the feature and the channel so the
/// message is self-describing without any client-side rendering assumptions.
pub(crate) fn format_digest(channel_name: &str, posts: &[PostRef], texts: &[String]) -> String {
    let mut out = format!(
        "📰 WhatsApp newsletter digest — {channel_name} ({} new)\n",
        posts.len()
    );
    for (p, text) in posts.iter().zip(texts.iter()) {
        let when = chrono::DateTime::<chrono::Utc>::from_timestamp(p.ts as i64, 0)
            .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| format!("{}s", p.ts));
        out.push_str(&format!("\n[{when}] {text}\n"));
    }
    out
}

// ---------------------------------------------------------------------------
// IO leg
// ---------------------------------------------------------------------------

/// Poll spacing per the decision doc (15 minutes).
pub(crate) const POLL_INTERVAL_SECS: u64 = 15 * 60;

/// Posts fetched per page, newest first.
pub(crate) const POLL_PAGE_SIZE: u32 = 30;

/// Pages followed within one tick while the whole page stays unseen. A
/// newsletter posting >300 items inside one interval loses the overflow with
/// a warn — documented, bounded, and logged rather than an unbounded fetch
/// loop (decision doc's "no unbounded" clause).
pub(crate) const POLL_MAX_PAGES: usize = 10;

/// The poller loop: one task per process session, spawned on first connect.
/// The first tick waits a full interval — the connect window already carries
/// the greeting and any history-sync requests, and the cursor makes a later
/// first poll complete anyway.
pub(crate) async fn run_poller(
    wa_state: Arc<WhatsAppState>,
    repo: ChannelMessageRepository,
    owner: String,
    channels: Vec<String>,
) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(POLL_INTERVAL_SECS)).await;
        let Some(client) = wa_state.client().await else {
            continue; // disconnected window: next tick retries
        };
        for ch in &channels {
            if let Err(e) = poll_once(&client, &repo, ch, &owner).await {
                tracing::warn!("whatsapp newsletter: poll for {ch} failed: {e}");
            }
        }
    }
}

/// Poll one channel once. The cursor advances only after the digest send was
/// accepted, so a failed send re-surfaces the posts on the next tick.
pub(crate) async fn poll_once(
    client: &Arc<whatsapp_rust::client::Client>,
    repo: &ChannelMessageRepository,
    channel: &str,
    owner: &str,
) -> anyhow::Result<()> {
    let jid: Jid = channel
        .parse()
        .map_err(|e| anyhow::anyhow!("bad newsletter jid {channel}: {e:?}"))?;
    let owner_jid: Jid = owner
        .parse()
        .map_err(|e| anyhow::anyhow!("bad owner jid {owner}: {e:?}"))?;
    let cur = repo
        .newsletter_cursor(channel)
        .await?
        .map(|(sid, ts)| Cursor {
            last_server_id: sid,
            last_ts: ts,
        });

    if cur.is_none() {
        // First sight: baseline silently on the newest post. Opting in never
        // replays whatever was already there at the phone.
        let fetched = client
            .newsletter()
            .get_messages(jid.clone(), POLL_PAGE_SIZE, None)
            .await
            .map_err(|e| anyhow::anyhow!("get_messages({channel}): {e:?}"))?;
        let refs: Vec<PostRef> = fetched
            .iter()
            .map(|m| PostRef {
                server_id: m.server_id,
                ts: m.timestamp,
            })
            .collect();
        let next = next_cursor(None, &refs);
        repo.set_newsletter_cursor(channel, next.last_server_id, next.last_ts)
            .await?;
        tracing::debug!("whatsapp newsletter: baselined {channel}");
        return Ok(());
    }
    let cur = cur.expect("checked above");

    // Follow pages with `before` while the whole newest page is unseen; the
    // first page that touches known territory proves there is no more below.
    let mut digested: Vec<(PostRef, String)> = Vec::new();
    let mut before: Option<u64> = None;
    let mut overflow = true;
    for _ in 0..POLL_MAX_PAGES {
        let fetched = client
            .newsletter()
            .get_messages(jid.clone(), POLL_PAGE_SIZE, before)
            .await
            .map_err(|e| anyhow::anyhow!("get_messages({channel}): {e:?}"))?;
        let refs: Vec<PostRef> = fetched
            .iter()
            .map(|m| PostRef {
                server_id: m.server_id,
                ts: m.timestamp,
            })
            .collect();
        let new = select_new(Some(cur), &refs);
        if new.is_empty() {
            overflow = false;
            break;
        }
        for p in &new {
            let text = fetched
                .iter()
                .find(|m| m.server_id == p.server_id)
                .and_then(|m| m.message.as_ref())
                .map(post_body)
                .unwrap_or_else(|| "[post without text]".to_string());
            digested.push((*p, text));
        }
        if new.len() == refs.len() && refs.len() as u32 == POLL_PAGE_SIZE {
            before = refs.iter().map(|p| p.ts).min();
            continue;
        }
        overflow = false;
        break;
    }
    if overflow {
        tracing::warn!(
            "whatsapp newsletter: {channel} exceeded {POLL_MAX_PAGES} full unseen pages in one \
             tick; older posts beyond the cap are skipped"
        );
    }

    if digested.is_empty() {
        return Ok(());
    }
    digested.sort_by_key(|(p, _)| p.server_id);

    let name = match client.newsletter().get_metadata(&jid).await {
        Ok(m) => m.name,
        Err(_) => channel.to_string(),
    };
    let posts: Vec<PostRef> = digested.iter().map(|(p, _)| *p).collect();
    let texts: Vec<String> = digested.into_iter().map(|(_, t)| t).collect();
    let digest = format_digest(&name, &posts, &texts);

    let msg = waproto::whatsapp::Message {
        conversation: Some(digest.clone()),
        ..Default::default()
    };
    client
        .send_message(owner_jid, msg)
        .await
        .map_err(|e| anyhow::anyhow!("digest send to owner: {e:?}"))?;

    // Stored copy: the digest stays answerable from the search tool even if
    // the message is later deleted on the phone side.
    let row = crate::db::models::ChannelMessage {
        created_at: chrono::Utc::now(),
        ..crate::db::models::ChannelMessage::new(
            "whatsapp".into(),
            channel.to_string(),
            Some(name.clone()),
            owner.to_string(),
            name,
            digest,
            "newsletter".into(),
            None,
        )
    };
    if let Err(e) = repo.insert(&row).await {
        tracing::warn!("whatsapp newsletter: digest store row failed: {e}");
    }

    let next = next_cursor(Some(cur), &posts);
    repo.set_newsletter_cursor(channel, next.last_server_id, next.last_ts)
        .await?;
    Ok(())
}

/// Best-effort text of a newsletter post: plain, extended, then captioned
/// media; everything else gets a typed placeholder so the digest line never
/// disappears silently.
fn post_body(m: &waproto::whatsapp::Message) -> String {
    if let Some(t) = m.conversation.as_deref().filter(|t| !t.is_empty()) {
        return t.to_string();
    }
    if let Some(t) = m
        .extended_text_message
        .as_ref()
        .and_then(|e| e.text.as_deref())
        .filter(|t| !t.is_empty())
    {
        return t.to_string();
    }
    if let Some(c) = m
        .image_message
        .as_ref()
        .and_then(|i| i.caption.as_deref())
        .filter(|c| !c.is_empty())
    {
        return format!("📷 {c}");
    }
    if let Some(c) = m
        .video_message
        .as_ref()
        .and_then(|i| i.caption.as_deref())
        .filter(|c| !c.is_empty())
    {
        return format!("🎥 {c}");
    }
    "[media post]".to_string()
}
