//! Tests for rebuild outcome delivery (#304/#305): the background build's
//! completion and failure notices must reach whoever asked — a chat via the
//! cron `deliver_to` path, the TUI and channel sessions via the gated
//! session route in the rebuild completion hook (#1748).
//! Forum-topic origin rides the #1451 grammar (`telegram:chat:thread`),
//! captured automatically from the asking turn (#1457).

use crate::brain::tools::rebuild::rebuild_deliver_target;
use crate::db::Database;
use crate::db::repository::PendingRequestRepository;

// ── deliver_to mapping (#305) ────────────────────────────────────────────

#[test]
fn channel_chats_map_to_delivery_targets() {
    assert_eq!(
        rebuild_deliver_target("telegram", Some("123456789"), None).as_deref(),
        Some("telegram:123456789")
    );
    assert_eq!(
        rebuild_deliver_target("discord", Some("987654321"), None).as_deref(),
        Some("discord:987654321")
    );
    assert_eq!(
        rebuild_deliver_target("slack", Some("C0123ABC"), None).as_deref(),
        Some("slack:C0123ABC")
    );
}

#[test]
fn telegram_topic_origin_rides_the_1451_grammar() {
    // #1457: a rebuild fired from forum topic 249 must report back into
    // topic 249, not the chat default. Negative chat ids (supergroups) stay
    // intact — the scheduler's parse_telegram_target splits left-to-right.
    assert_eq!(
        rebuild_deliver_target("telegram", Some("-1004428873948"), Some("249")).as_deref(),
        Some("telegram:-1004428873948:249")
    );
    // Empty/whitespace thread degrades to the plain chat target, never to
    // "telegram:chat:" (which #1451's parser would reject loudly).
    assert_eq!(
        rebuild_deliver_target("telegram", Some("123"), Some("")).as_deref(),
        Some("telegram:123")
    );
    assert_eq!(
        rebuild_deliver_target("telegram", Some("123"), Some("  ")).as_deref(),
        Some("telegram:123")
    );
    // Discord/Slack ignore the thread component — no grammar for it there.
    assert_eq!(
        rebuild_deliver_target("discord", Some("987"), Some("249")).as_deref(),
        Some("discord:987")
    );
}

#[test]
fn tui_and_unsupported_channels_map_to_none() {
    // TUI has its own notifier; whatsapp has no cron delivery arm.
    assert!(rebuild_deliver_target("tui", None, None).is_none());
    assert!(rebuild_deliver_target("tui", Some("ignored"), None).is_none());
    assert!(rebuild_deliver_target("whatsapp", Some("15551234567"), None).is_none());
}

#[test]
fn missing_or_empty_chat_id_maps_to_none() {
    assert!(rebuild_deliver_target("telegram", None, None).is_none());
    assert!(rebuild_deliver_target("telegram", Some(""), None).is_none());
    assert!(rebuild_deliver_target("telegram", Some("   "), None).is_none());
}

// ── pending-request lookup (#305 source of channel/chat/thread) ─────────

#[tokio::test]
async fn find_latest_for_session_returns_the_current_turn_row() {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let repo = PendingRequestRepository::new(db.pool().clone());

    let session_id = uuid::Uuid::new_v4();
    repo.insert(
        uuid::Uuid::new_v4(),
        session_id,
        "please rebuild yourself",
        "telegram",
        Some("123456789"),
        None,
        "user",
    )
    .await
    .unwrap();

    let row = repo
        .find_latest_for_session(session_id)
        .await
        .unwrap()
        .expect("in-flight row must be found");
    assert_eq!(row.channel, "telegram");
    assert_eq!(row.channel_chat_id.as_deref(), Some("123456789"));

    // End-to-end mapping: this is the deliver_to the tool will schedule.
    assert_eq!(
        rebuild_deliver_target(
            &row.channel,
            row.channel_chat_id.as_deref(),
            row.channel_thread_id.as_deref()
        )
        .as_deref(),
        Some("telegram:123456789")
    );
}

#[tokio::test]
async fn rebuild_from_a_topic_targets_that_topic_end_to_end() {
    // The #1457 occurrence: rebuild triggered in OC Dev (thread 249),
    // completion landed in General. The row now carries the topic and the
    // scheduled target names it — landing IN the topic is #1451's parser.
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let repo = PendingRequestRepository::new(db.pool().clone());

    let session_id = uuid::Uuid::new_v4();
    repo.insert(
        uuid::Uuid::new_v4(),
        session_id,
        "rebuild from OC Dev",
        "telegram",
        Some("-1004428873948"),
        Some("249"),
        "user",
    )
    .await
    .unwrap();

    let row = repo
        .find_latest_for_session(session_id)
        .await
        .unwrap()
        .expect("in-flight row must be found");
    assert_eq!(
        rebuild_deliver_target(
            &row.channel,
            row.channel_chat_id.as_deref(),
            row.channel_thread_id.as_deref()
        )
        .as_deref(),
        Some("telegram:-1004428873948:249")
    );
}

#[tokio::test]
async fn find_latest_for_session_none_when_no_row() {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let repo = PendingRequestRepository::new(db.pool().clone());

    let row = repo
        .find_latest_for_session(uuid::Uuid::new_v4())
        .await
        .unwrap();
    assert!(row.is_none());
}
