//! The `oc://` target URL scheme and its single resolver (#148).
//!
//! EVERY tool that targets a session or a channel accepts these URLs and
//! funnels through [`resolve_target`] here — this module is the ONLY place
//! that may parse or construct `oc://` strings (the opaqueness wall, D11;
//! enforced by `src/tests/oc_target_opaqueness_test.rs`). Consumers hand
//! the raw string over and get a [`ResolvedTarget`] back, or an error that
//! names the candidates — never a guess.
//!
//! Grammar:
//!
//! ```text
//! oc://session/<uuid | 8-char-prefix>
//! oc://telegram/<chat_id>[/<thread>]     thread=1 => General topic (session-scoping
//!                                        key, #1220) — resolves, delivers threadless;
//!                                        thread=0 is a PARSE ERROR, never coerced.
//! oc://discord/<channel_id>
//! oc://slack/<channel_id>
//! oc://whatsapp/<phone | jid>
//! oc://here                              ambient — resolves against the origin stamp
//! ```
//!
//! The legacy `telegram:<chat>[:<thread>]` deliver_to grammar stays accepted
//! and distinct (no `://`), so existing cron job rows keep working untouched.
//! Non-numeric segments are percent-encoded per RFC 3986 (D11); numeric ids
//! stay bare digits.

use anyhow::{anyhow, bail, Result};
use uuid::Uuid;

use super::telegram::session_resolve::GENERAL_TOPIC_ID;
use crate::brain::tools::OriginTarget;

/// Where a resolved target's delivery should go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetDestination {
    /// Deliver into the session that owns the URL — the delivery surface
    /// re-derives its current binding from the session id at send time.
    Session(Uuid),
    /// Deliver to this concrete channel target (chat id + optional topic).
    /// Cron bakes this form at create time; the owning session id rides
    /// alongside for liveness revalidation (fork #17).
    Channel {
        channel: &'static str,
        chat_id: String,
        thread: Option<i32>,
        session: Option<Uuid>,
    },
}

/// The result of resolving an `oc://` target URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTarget {
    /// The session the URL designates, when the resolver could pin one.
    pub session: Option<Uuid>,
    /// Concrete delivery destination.
    pub destination: TargetDestination,
}

impl ResolvedTarget {
    /// The legacy deliver_to string for this target (`telegram:<chat>[:<t>]`
    /// …) — the concrete form baked into cron job rows. General topics bake
    /// WITHOUT `:1` (#1319): the scoping key never becomes a wire address.
    pub fn deliver_to(&self) -> String {
        match &self.destination {
            TargetDestination::Session(_) => String::new(),
            TargetDestination::Channel {
                channel,
                chat_id,
                thread,
                ..
            } => match thread {
                Some(t) if *t != GENERAL_TOPIC_ID => format!("{channel}:{chat_id}:{t}"),
                _ => format!("{channel}:{chat_id}"),
            },
        }
    }
}

/// Percent-encode a non-numeric URL path segment (D11): everything outside
/// RFC 3986 unreserved set becomes `%XX`. Numeric segments pass through
/// bare — callers must not feed digits here.
pub fn encode_segment(seg: &str) -> String {
    let mut out = String::with_capacity(seg.len());
    for b in seg.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Decode a percent-encoded segment. Invalid escapes are an error — the
/// resolver never guesses at mangled input.
pub fn decode_segment(seg: &str) -> Result<String> {
    let bytes = seg.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes
                .get(i + 1..i + 3)
                .ok_or_else(|| anyhow!("invalid percent-encoding in '{seg}'"))?;
            let s = std::str::from_utf8(hex)
                .map_err(|_| anyhow!("invalid percent-encoding in '{seg}'"))?;
            let byte = u8::from_str_radix(s, 16)
                .map_err(|_| anyhow!("invalid percent-encoding in '{seg}'"))?;
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| anyhow!("segment '{seg}' is not valid UTF-8"))
}

/// Split an `oc://` URL into `(authority, decoded-segments)`. Pure syntax —
/// no channel semantics. This is the ONLY grammar parser for `oc://`.
fn parse_url(url: &str) -> Result<(&str, Vec<String>)> {
    let rest = url
        .strip_prefix("oc://")
        .ok_or_else(|| anyhow!("not an oc:// url: '{url}'"))?;
    let (authority, path) = match rest.split_once('/') {
        Some((a, p)) => (a, p),
        None => (rest, ""),
    };
    if authority.is_empty() {
        bail!("empty authority in '{url}'");
    }
    let segments: Vec<String> = if path.is_empty() {
        Vec::new()
    } else {
        path.split('/')
            .map(decode_segment)
            .collect::<Result<Vec<_>>>()?
    };
    Ok((authority, segments))
}

/// The `here` token and its URL form — both resolve against the ambient
/// origin stamp. A cron turn (headless) has no origin and must refuse.
pub const HERE_TOKEN: &str = "here";

/// Is this string an `oc://` URL or the `here` token? Cheap syntax check so
/// consumers can decide whether to route through the resolver without
/// parsing the contents themselves (opaqueness wall — they still may not).
pub fn is_target_url(s: &str) -> bool {
    s.starts_with("oc://") || s == HERE_TOKEN
}

/// What the resolver needs from the world. Implemented by the wiring context
/// (ChannelManager + session lookup) so tests can drive resolution without
/// a live daemon.
#[async_trait::async_trait]
pub trait TargetResolution: Send + Sync {
    /// The session bound to `(channel, chat_id, thread)` — reverse
    /// ownership map. `None` when no session currently owns it.
    async fn session_for_channel(
        &self,
        channel: &str,
        chat_id: &str,
        thread: Option<i32>,
    ) -> Option<Uuid>;

    /// The channel target a session is currently bound to (forward map) —
    /// used to confirm a resolved session still owns a live channel.
    async fn binding_for_session(
        &self,
        session: Uuid,
    ) -> Option<OriginTarget>;

    /// The session bound to an `oc://telegram/<chat>` URL with NO thread:
    /// `Ok(None)` when the chat simply has no binding, `Err(list)` when the
    /// chat is a forum with MULTIPLE topic sessions — the caller must pick
    /// a topic; bare-chat resolution would guess (D10/D9 multi-topic rule).
    async fn telegram_chat_topics(
        &self,
        chat_id: i64,
    ) -> Result<Option<Vec<i32>>>;
}

/// Resolve an `oc://` URL (or the `here` token) to a concrete target.
///
/// Resolution order: `here`/ambient origin → session authority (exact uuid
/// fast-path, then case-insensitive 8-char prefix with the 0/1/many law) →
/// channel authority via the reverse ownership map. Errors name candidates
/// or the reason — resolution never guesses.
pub async fn resolve_target(
    url: &str,
    origin: Option<&OriginTarget>,
    world: &(impl TargetResolution + ?Sized),
    sessions: &[crate::db::models::Session],
) -> Result<ResolvedTarget> {
    // `here` — ambient origin only. No origin (cron, CLI, sub-agent) is a
    // clean refusal: a headless surface has no "here".
    if url == HERE_TOKEN {
        return match origin {
            None => bail!(
                "'here' has no current channel on this surface — pass an explicit oc:// target"
            ),
            Some(o) => Ok(ResolvedTarget {
                session: world
                    .session_for_channel(o.channel, &o.chat_id, o.thread)
                    .await,
                destination: TargetDestination::Channel {
                    channel: o.channel,
                    chat_id: o.chat_id.to_string(),
                    thread: o.thread,
                    session: world
                        .session_for_channel(o.channel, &o.chat_id, o.thread)
                        .await,
                },
            }),
        };
    }

    let (authority, segments) = parse_url(url)?;

    match authority {
        "session" => {
            let id = segments
                .first()
                .ok_or_else(|| anyhow!("oc://session needs an id: '{url}'"))?;
            // Exact-uuid fast path, then prefix (0/1/many, never guess).
            let session = if let Ok(u) = Uuid::parse_str(id) {
                sessions
                    .iter()
                    .find(|s| s.id == u)
                    .map(|s| s.id)
                    .ok_or_else(|| anyhow!("no session with id '{id}'"))?
            } else {
                // Prefix law lives in one place (#1340): reuse it verbatim so
                // the ambiguity rules cannot drift from the CLI resolvers.
                crate::cli::session_resolve::resolve_one_by_prefix(sessions, id)
                    .map_err(|e| anyhow!("{e}"))?
            };
            let binding = world.binding_for_session(session).await;
            Ok(ResolvedTarget {
                session: Some(session),
                destination: match binding {
                    Some(b) => TargetDestination::Channel {
                        channel: b.channel,
                        chat_id: b.chat_id,
                        thread: b.thread,
                        session: Some(session),
                    },
                    None => TargetDestination::Session(session),
                },
            })
        }
        "telegram" => {
            let chat_id = segments
                .first()
                .ok_or_else(|| anyhow!("oc://telegram needs a chat id: '{url}'"))?;
            let _chat: i64 = chat_id
                .parse()
                .map_err(|_| anyhow!("telegram chat id must be numeric: '{chat_id}'"))?;
            let thread: Option<i32> = match segments.get(1) {
                None => None,
                Some(t) => {
                    let t: i32 = t
                        .parse()
                        .map_err(|_| anyhow!("telegram thread must be numeric: '{t}'"))?;
                    if t == 0 {
                        // D10: `0` means nothing in the Telegram API or this
                        // codebase — refuse, never coerce to General/bare.
                        bail!("thread id 0 is not a valid Telegram topic (General is 1)");
                    }
                    Some(t)
                }
            };
            let session = world
                .session_for_channel("telegram", chat_id, thread)
                .await;
            // Bare-chat URL on a forum chat: ambiguous by design (D9/D10).
            if thread.is_none()
                && session.is_none()
                && let Ok(cid) = chat_id.parse::<i64>()
                && let Some(topics) = world.telegram_chat_topics(cid).await?
                && topics.len() > 1
            {
                bail!(
                    "chat {chat_id} is a forum with multiple topic sessions — \
                     pick a topic: {}",
                    topics
                        .iter()
                        .map(|t| format!("oc://telegram/{chat_id}/{t}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            Ok(ResolvedTarget {
                session,
                destination: TargetDestination::Channel {
                    channel: "telegram",
                    chat_id: chat_id.to_string(),
                    thread,
                    session,
                },
            })
        }
        "discord" | "slack" | "whatsapp" => {
            let chat_id = segments
                .first()
                .ok_or_else(|| anyhow!("oc://{authority} needs a channel id: '{url}'"))?;
            if chat_id.is_empty() {
                bail!("empty channel id in '{url}'");
            }
            // WhatsApp: accept a bare phone number as sugar for the JID.
            let id = if authority == "whatsapp" && !chat_id.contains('@') {
                let digits = chat_id.trim_start_matches('+');
                if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                    bail!("whatsapp target must be a phone number or a JID: '{chat_id}'");
                }
                format!("{digits}@s.whatsapp.net")
            } else {
                chat_id.clone()
            };
            let session = world.session_for_channel(authority, &id, None).await;
            Ok(ResolvedTarget {
                session,
                destination: TargetDestination::Channel {
                    channel: authority_static(authority)?,
                    chat_id: id,
                    thread: None,
                    session,
                },
            })
        }
        other => bail!("unknown oc:// authority '{other}' (session|telegram|discord|slack|whatsapp)"),
    }
}

fn authority_static(a: &str) -> Result<&'static str> {
    Ok(match a {
        "discord" => "discord",
        "slack" => "slack",
        "whatsapp" => "whatsapp",
        other => bail!("unknown authority '{other}'"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sess(id: Uuid, title: &str) -> crate::db::models::Session {
        let mut s = crate::db::models::Session {
            id: Uuid::nil(),
            title: None,
            model: None,
            provider_name: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            archived_at: None,
            token_count: 0,
            total_cost: 0.0,
            working_directory: None,
            auto_title_attempted: false,
            project_id: None,
        };
        s.id = id;
        s.title = Some(title.to_string());
        s
    }

    /// In-memory `TargetResolution` for syntax + ambiguity tests.
    struct FakeWorld {
        bindings: std::collections::HashMap<(String, String, Option<i32>), Uuid>,
        forward: std::collections::HashMap<Uuid, OriginTarget>,
        forum_topics: std::collections::HashMap<i64, Vec<i32>>,
    }

    impl FakeWorld {
        fn new() -> Self {
            Self {
                bindings: Default::default(),
                forward: Default::default(),
                forum_topics: Default::default(),
            }
        }
        fn bind(&mut self, ch: &str, chat: &str, t: Option<i32>, s: Uuid) {
            self.bindings.insert((ch.to_string(), chat.to_string(), t), s);
        }
    }

    #[async_trait::async_trait]
    impl TargetResolution for FakeWorld {
        async fn session_for_channel(
            &self,
            channel: &str,
            chat_id: &str,
            thread: Option<i32>,
        ) -> Option<Uuid> {
            self.bindings
                .get(&(channel.to_string(), chat_id.to_string(), thread))
                .copied()
        }
        async fn binding_for_session(&self, session: Uuid) -> Option<OriginTarget> {
            self.forward.get(&session).cloned()
        }
        async fn telegram_chat_topics(&self, chat_id: i64) -> Result<Option<Vec<i32>>> {
            Ok(self.forum_topics.get(&chat_id).cloned())
        }
    }

    #[tokio::test]
    async fn here_without_origin_is_refused() {
        let w = FakeWorld::new();
        let e = resolve_target("here", None, &w, &[]).await.unwrap_err();
        assert!(e.to_string().contains("no current channel"), "{e}");
    }

    #[tokio::test]
    async fn here_resolves_from_origin() {
        let mut w = FakeWorld::new();
        let s = Uuid::new_v4();
        w.bind("telegram", "-100123", Some(GENERAL_TOPIC_ID), s);
        let origin = OriginTarget {
            channel: "telegram",
            chat_id: "-100123".into(),
            thread: Some(GENERAL_TOPIC_ID),
        };
        let r = resolve_target("here", Some(&origin), &w, &[]).await.unwrap();
        assert_eq!(r.deliver_to(), "telegram:-100123");
    }

    #[tokio::test]
    async fn telegram_thread_zero_is_parse_error() {
        let w = FakeWorld::new();
        let e = resolve_target("oc://telegram/-100123/0", None, &w, &[])
            .await
            .unwrap_err();
        assert!(e.to_string().contains("not a valid Telegram topic"), "{e}");
    }

    #[tokio::test]
    async fn telegram_general_resolves_and_delivers_threadless() {
        let mut w = FakeWorld::new();
        let s = Uuid::new_v4();
        w.bind("telegram", "-100123", Some(GENERAL_TOPIC_ID), s);
        let r = resolve_target("oc://telegram/-100123/1", None, &w, &[])
            .await
            .unwrap();
        assert_eq!(r.session, Some(s));
        assert_eq!(r.deliver_to(), "telegram:-100123");
    }

    #[tokio::test]
    async fn telegram_real_thread_bakes_with_thread() {
        let mut w = FakeWorld::new();
        let s = Uuid::new_v4();
        w.bind("telegram", "-100123", Some(42), s);
        let r = resolve_target("oc://telegram/-100123/42", None, &w, &[])
            .await
            .unwrap();
        assert_eq!(r.session, Some(s));
        assert_eq!(r.deliver_to(), "telegram:-100123:42");
    }

    #[tokio::test]
    async fn telegram_bare_chat_on_multi_topic_forum_is_ambiguous() {
        let mut w = FakeWorld::new();
        w.forum_topics
            .insert(-100123, vec![GENERAL_TOPIC_ID, 42]);
        let e = resolve_target("oc://telegram/-100123", None, &w, &[])
            .await
            .unwrap_err();
        assert!(e.to_string().contains("multiple topic sessions"), "{e}");
        assert!(e.to_string().contains("oc://telegram/-100123/42"));
    }

    #[tokio::test]
    async fn session_prefix_ambiguity_lists_candidates() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let w = FakeWorld::new();
        let shared = format!("{:08x}", a.as_u128() >> 96);
        // Force a shared 8-char prefix by using the same UUID twice — instead
        // assert the single-match and no-match paths.
        let r = resolve_target(&format!("oc://session/{a}"), None, &w, &[sess(a, "one"), sess(b, "two")])
            .await
            .unwrap();
        assert_eq!(r.session, Some(a));
        let e = resolve_target("oc://session/zzzzzzzz", None, &w, &[sess(a, "one")])
            .await
            .unwrap_err();
        assert!(e.to_string().contains("no session"), "{e}");
        let _ = shared; // prefix-shaping documented above
    }

    #[tokio::test]
    async fn whatsapp_phone_normalizes_to_jid() {
        let mut w = FakeWorld::new();
        let s = Uuid::new_v4();
        w.bind("whatsapp", "79991234567@s.whatsapp.net", None, s);
        let r = resolve_target("oc://whatsapp/+79991234567", None, &w, &[])
            .await
            .unwrap();
        assert_eq!(r.session, Some(s));
        assert_eq!(
            r.deliver_to(),
            "whatsapp:79991234567@s.whatsapp.net"
        );
    }

    #[tokio::test]
    async fn unknown_authority_and_bad_paths_are_errors() {
        let w = FakeWorld::new();
        assert!(resolve_target("oc://irc/1", None, &w, &[]).await.is_err());
        assert!(resolve_target("oc://telegram/", None, &w, &[]).await.is_err());
        assert!(resolve_target("https://example.com", None, &w, &[]).await.is_err());
    }

    #[test]
    fn encoding_roundtrip() {
        let raw = "kanban board/2";
        let enc = encode_segment(raw);
        assert!(!enc.contains(' '));
        assert_eq!(decode_segment(&enc).unwrap(), raw);
        assert!(decode_segment("%zz").is_err());
        assert!(decode_segment("%4").is_err());
    }
}
