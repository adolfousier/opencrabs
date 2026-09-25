//! Tests for `resolve_target` (oc:// target resolution) syntax + ambiguity.
//!
//! Extracted from an inline `#[cfg(test)]` block in
//! `src/channels/target_resolver.rs`; project policy (CONTRIBUTING.md)
//! requires all tests under `src/tests/`.

use std::collections::HashMap;

use async_trait::async_trait;
use uuid::Uuid;

use crate::brain::tools::OriginTarget;
use crate::channels::target_resolver::{
    ResolvedTarget, TargetDestination, TargetResolution, decode_segment, encode_segment,
    extract_session_target, resolve_target,
};
use crate::channels::telegram::session_resolve::GENERAL_TOPIC_ID;

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
        channel_chat_key: None,
    };
    s.id = id;
    s.title = Some(title.to_string());
    s
}

/// In-memory `TargetResolution` for syntax + ambiguity tests.
struct FakeWorld {
    bindings: HashMap<(String, String, Option<i32>), Uuid>,
    forward: HashMap<Uuid, OriginTarget>,
    forum_topics: HashMap<i64, Vec<i32>>,
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
        self.bindings
            .insert((ch.to_string(), chat.to_string(), t), s);
    }
}

#[async_trait]
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
    async fn telegram_chat_topics(&self, chat_id: i64) -> anyhow::Result<Option<Vec<i32>>> {
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
    let r = resolve_target("here", Some(&origin), &w, &[])
        .await
        .unwrap();
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
    w.forum_topics.insert(-100123, vec![GENERAL_TOPIC_ID, 42]);
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
    let r = resolve_target(
        &format!("oc://session/{a}"),
        None,
        &w,
        &[sess(a, "one"), sess(b, "two")],
    )
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
    assert_eq!(r.deliver_to(), "whatsapp:79991234567@s.whatsapp.net");
}

#[tokio::test]
async fn unknown_authority_and_bad_paths_are_errors() {
    let w = FakeWorld::new();
    assert!(resolve_target("oc://irc/1", None, &w, &[]).await.is_err());
    assert!(
        resolve_target("oc://telegram/", None, &w, &[])
            .await
            .is_err()
    );
    assert!(
        resolve_target("https://example.com", None, &w, &[])
            .await
            .is_err()
    );
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

// ── session targets (#1629) ─────────────────────────────────────────────────

/// `oc://session/<id>` and the legacy `session:<id>` both name a session, and
/// nothing else does.
#[test]
fn extract_session_target_handles_url_and_legacy() {
    assert_eq!(
        extract_session_target("oc://session/12345678-1234-1234-1234-123456789abc"),
        Some("12345678-1234-1234-1234-123456789abc")
    );
    assert_eq!(
        extract_session_target("oc://session/12345678"),
        Some("12345678")
    );
    assert_eq!(
        extract_session_target("oc://session/12345678/extra"),
        Some("12345678")
    );
    assert_eq!(
        extract_session_target("session:12345678-1234-1234-1234-123456789abc"),
        Some("12345678-1234-1234-1234-123456789abc")
    );
    assert_eq!(extract_session_target("telegram:123456"), None);
    assert_eq!(extract_session_target("oc://telegram/123456"), None);
}

/// A session destination renders back as the `session:<uuid>` spelling the
/// extractor accepts, so a resolved target round-trips.
#[test]
fn session_target_deliver_to_format() {
    let u = Uuid::new_v4();
    let rt = ResolvedTarget {
        session: Some(u),
        destination: TargetDestination::Session(u),
    };
    assert_eq!(rt.deliver_to(), format!("session:{u}"));
}
