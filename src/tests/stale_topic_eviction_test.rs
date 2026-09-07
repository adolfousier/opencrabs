//! Stale-topic auto-route eviction (#116).
//!
//! A remembered forum topic can be deleted on Telegram's side while we hold
//! its address in `channel_messages` / the session-topic map. Every send
//! routed to it then fails with `400 Bad Request: message thread not found`,
//! and BEFORE the fix the plain fallback re-used the same poisoned thread and
//! failed identically — a permanent failure loop for the chat. Measured on
//! prod 2026-09-06 05:29Z (owner DM, thread 30134): rich 400 → HTML 400 →
//! plain 400 → tool error.
//!
//! The contract under test:
//! 1. `clear_thread_for_chat` clears ONLY that chat's rows carrying the dead
//!    thread (chat-scoped, other chats untouched);
//! 2. `send_markdown_outbox` on the thread-not-found error evicts the dead
//!    address and retries the send ONCE unthreaded (General/DM = absence of a
//!    thread, #1319) instead of dropping the message;
//! 3. any other rich failure still falls through to the HTML ladder with the
//!    thread intact (no eviction, no behavior change).

use crate::db::Database;
use crate::db::models::ChannelMessage;
use crate::db::repository::ChannelMessageRepository;
use teloxide::types::{ChatId, ThreadId};

const CHAT: i64 = 133_526_395;
const OTHER_CHAT: i64 = 999_111;
const DEAD_TOPIC: i32 = 30_134;

async fn seeded_repo() -> ChannelMessageRepository {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let repo = ChannelMessageRepository::new(db.pool().clone());
    let mk = |chat: i64, mid: &str, thread: Option<i32>| {
        let cm = ChannelMessage::new(
            "telegram".into(),
            chat.to_string(),
            None,
            "u1".into(),
            "alice".into(),
            format!("msg {mid}"),
            "text".into(),
            Some(mid.into()),
        );
        match thread {
            Some(t) => cm.with_thread(Some(t.to_string()), None),
            None => cm,
        }
    };
    // The poisoned chat: two rows on the dead topic, one General row.
    repo.insert(&mk(CHAT, "m-1", Some(DEAD_TOPIC)))
        .await
        .unwrap();
    repo.insert(&mk(CHAT, "m-2", Some(DEAD_TOPIC)))
        .await
        .unwrap();
    repo.insert(&mk(CHAT, "m-3", None)).await.unwrap();
    // A different chat that happens to use the SAME topic id must stay
    // untouched — eviction is chat-scoped, never global.
    repo.insert(&mk(OTHER_CHAT, "m-4", Some(DEAD_TOPIC)))
        .await
        .unwrap();
    repo
}

#[tokio::test]
async fn eviction_clears_only_the_dead_chat_rows() {
    let repo = seeded_repo().await;

    let evicted = repo
        .clear_thread_for_chat("telegram", &CHAT.to_string(), &DEAD_TOPIC.to_string())
        .await
        .expect("eviction");

    assert_eq!(evicted, 2, "both dead-topic rows of the chat are cleared");

    // The poisoned chat now resolves to its General row (no thread), never
    // the dead topic again.
    let rows = repo
        .recent(Some("telegram"), &CHAT.to_string(), 10, None, None)
        .await
        .expect("recent after eviction");
    assert!(
        rows.iter().all(|r| r.thread_id.as_deref() != Some("30134")),
        "no row of the chat may still carry the dead topic"
    );

    // The other chat using the same topic id is untouched.
    let other = repo
        .recent(
            Some("telegram"),
            &OTHER_CHAT.to_string(),
            10,
            Some(&DEAD_TOPIC.to_string()),
            None,
        )
        .await
        .expect("other chat rows");
    assert_eq!(
        other.len(),
        1,
        "a different chat using the same topic id must NOT be evicted"
    );
}

#[tokio::test]
async fn eviction_of_an_unknown_topic_is_a_noop() {
    let repo = seeded_repo().await;
    let evicted = repo
        .clear_thread_for_chat("telegram", &CHAT.to_string(), "424242")
        .await
        .expect("eviction");
    assert_eq!(evicted, 0, "an unknown topic clears nothing");
}

/// The seam contract end-to-end: rich fails with the thread-not-found
/// signature on the FIRST call (threaded) and succeeds on the SECOND
/// (unthreaded). The wire must show the unthreaded retry — not a drop, and
/// not a second threaded attempt.
#[tokio::test]
async fn outbox_retries_unthreaded_after_thread_not_found() {
    use crate::channels::telegram::send::send_markdown_outbox;

    // Pin the process-wide config mirror for this test: the rich-arm choice
    // reads Config::current(), and parallel tests in this binary leave the
    // mirror in arbitrary states. Without the pin, the send may ride the
    // HTML ladder (teloxide client) and the sendRichMessage mocks below see
    // zero requests — a nondeterministic RED that has nothing to do with
    // the eviction contract under test. Guard serializes the swap against
    // the governor tests that mutate the same mirror.
    let _guard = crate::channels::telegram::governor::test_support::registry_guard().await;
    let mut pinned: crate::config::Config =
        toml::from_str(include_str!("../../config.toml.example"))
            .expect("embedded config.toml.example must parse");
    // The example file ships rich_messages COMMENTED OUT (default false) —
    // a pinned parse alone leaves the flag off and the send rides the HTML
    // ladder, never reaching the sendRichMessage mocks below. Force the
    // native-rich arm on; that is the path the eviction contract lives on.
    pinned.channels.telegram.rich_messages = true;
    crate::config::Config::set_current(pinned);

    let mut server = mockito::Server::new_async().await;
    // Register order matters. mockito 1.7 dispatches each request to the
    // FIRST matching mock that still has unmet expected hits (creation
    // order), not most-recent-first. So the selective `dead` mock is
    // registered FIRST: it matches only while the request carries the dead
    // thread id (PartialJson on message_thread_id). On the retry the thread
    // key is OMITTED entirely (unthreaded address), so `dead` no longer
    // matches and execution falls to the catch-all `healed` registered
    // SECOND — the assert below only passes if the wire really went
    // unthreaded.
    let dead = server
        .mock("POST", "/botTESTTOKEN/sendRichMessage")
        .match_body(mockito::Matcher::PartialJson(
            serde_json::json!({"message_thread_id": DEAD_TOPIC}),
        ))
        .with_status(400)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{"ok":false,"error_code":400,"description":"Bad Request: message thread not found"}"#,
        )
        .expect(1)
        .create_async()
        .await;
    // Catch-all registered SECOND: never reached while the dead mock still
    // matches (first-match-with-missing-hits wins), so it only serves the
    // unthreaded retry.
    let healed = server
        .mock("POST", "/botTESTTOKEN/sendRichMessage")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{"ok":true,"result":{"message_id":77,"date":1757166400,"chat":{"id":133526395,"type":"private"},"text":"| a | b |\n|---|---|\n| 1 | 2 |"}}"#,
        )
        .expect(1)
        .create_async()
        .await;

    let bot = teloxide::Bot::with_client(
        "TESTTOKEN",
        reqwest_teloxide::Client::builder().build().unwrap(),
    )
    .set_api_url(server.url().parse().unwrap());

    // Table-shaped so the message takes the native rich arm.
    let md = "| a | b |\n|---|---|\n| 1 | 2 |";
    let sent = send_markdown_outbox(
        &bot,
        ChatId(CHAT),
        Some(ThreadId(teloxide::types::MessageId(DEAD_TOPIC))),
        md,
        "tool",
        "send",
        None,
    )
    .await
    .expect("the send must be healed by the unthreaded retry, not dropped");

    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, 77);

    dead.assert_async().await;
    healed.assert_async().await;
}
