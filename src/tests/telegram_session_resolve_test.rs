//! Integration tests for Telegram session title + label drift (issue #121).

use crate::channels::telegram::TelegramState;
use crate::channels::telegram::session_resolve::{
    ResolveSource, build_session_title, chat_id_suffix, choose_resolve_source,
    session_idle_expired, should_refresh_label, topic_session_id,
};
use crate::db::Database;
use crate::db::models::Session;
use crate::db::repository::SessionRepository;
use crate::services::{ServiceContext, SessionService};
use uuid::Uuid;

async fn fresh_repo() -> (Database, SessionRepository) {
    let db = Database::connect_in_memory()
        .await
        .expect("in-memory DB connect");
    db.run_migrations().await.expect("migrations");
    let repo = SessionRepository::new(db.pool().clone());
    (db, repo)
}

#[test]
fn resolve_policy_prefers_chat_bound_over_suffix_winner() {
    let bound = Uuid::new_v4();
    let suffix = Uuid::new_v4();
    assert_eq!(
        choose_resolve_source(Some(bound), false, Some(suffix)),
        ResolveSource::ChatBound
    );
}

#[tokio::test]
async fn telegram_state_chat_map_survives_suffix_competition() {
    let state = TelegramState::new();
    let chat_id = 4242_i64;
    let bound = Uuid::new_v4();
    let suffix_winner = Uuid::new_v4();
    state.register_session_chat(bound, chat_id, None).await;
    assert_eq!(state.chat_session(chat_id, None).await, Some(bound));
    assert_eq!(
        choose_resolve_source(
            state.chat_session(chat_id, None).await,
            false,
            Some(suffix_winner)
        ),
        ResolveSource::ChatBound
    );
}

#[test]
fn should_not_clobber_auto_titled_dm_title() {
    let auto = "Telegram: Fix deploy pipeline [chat:133526395]";
    let template = build_session_title(true, "Alexey", 133526395, "", 133526395, None, None);
    assert!(
        !should_refresh_label(auto, &template),
        "auto-titled DM must not revert to default template"
    );
}

#[test]
fn group_rename_still_refreshes() {
    let old = "Telegram: Old Group [chat:-5246593256]";
    let new = "Telegram: New Group [chat:-5246593256]";
    assert!(should_refresh_label(old, new));
}

#[tokio::test]
async fn suffix_lookup_after_switch_touch_picks_switched_row() {
    let (_db, repo) = fresh_repo().await;
    let chat_id = 42_i64;
    let suffix = chat_id_suffix(chat_id, None);
    let title = build_session_title(true, "U", 1, "", chat_id, None, None);

    let older = Session::new(Some(title.clone()), None, None);
    repo.create(&older).await.expect("create older");

    let mut newer = Session::new(Some(title), None, None);
    newer.updated_at = older.updated_at + chrono::Duration::seconds(1);
    repo.create(&newer).await.expect("create newer");

    // Simulate /sessions switch to older session (touch updated_at)
    let mut switched = older.clone();
    switched.updated_at = newer.updated_at + chrono::Duration::seconds(1);
    repo.update(&switched).await.expect("touch older");

    let hit = repo
        .find_by_title_suffix(&suffix)
        .await
        .expect("query")
        .expect("hit");
    assert_eq!(hit.id, older.id);
}

#[tokio::test]
async fn auto_titled_title_survives_should_refresh_check() {
    let template = build_session_title(true, "Alice", 1, "", 99, None, None);
    let auto_titled = format!("Telegram: Deploy fix {}", chat_id_suffix(99, None));
    assert!(!should_refresh_label(&auto_titled, &template));
}

/// Mirrors handler chat-bound idle branch: archive stale bound row, create replacement.
/// Guest /sessions switch only needs register_session_chat (extra_sessions map removed).
#[tokio::test]
async fn register_session_chat_binds_guest_dm() {
    let state = TelegramState::new();
    let guest_chat_id = 9988_i64;
    let session_id = Uuid::new_v4();
    state
        .register_session_chat(session_id, guest_chat_id, None)
        .await;
    assert_eq!(
        state.chat_session(guest_chat_id, None).await,
        Some(session_id)
    );
    assert_eq!(
        choose_resolve_source(state.chat_session(guest_chat_id, None).await, false, None),
        ResolveSource::ChatBound
    );
}

#[tokio::test]
async fn archived_chat_map_entry_uses_suffix_not_bound() {
    let bound = Uuid::new_v4();
    let suffix = Uuid::new_v4();
    assert_eq!(
        choose_resolve_source(Some(bound), true, Some(suffix)),
        ResolveSource::Suffix
    );
}

#[tokio::test]
async fn suffix_path_when_chat_map_empty() {
    let suffix = Uuid::new_v4();
    assert_eq!(
        choose_resolve_source(None, false, Some(suffix)),
        ResolveSource::Suffix
    );
    assert_eq!(
        choose_resolve_source(None, false, None),
        ResolveSource::Create
    );
}

#[tokio::test]
async fn chat_bound_idle_archives_and_creates_new_session() {
    let (db, repo) = fresh_repo().await;
    let ctx = ServiceContext::new(db.pool().clone());
    let svc = SessionService::new(ctx.clone());
    let chat_id = 77_i64;
    let title = build_session_title(true, "U", 1, "", chat_id, None, None);

    let mut bound = Session::new(Some(title.clone()), None, None);
    bound.updated_at = chrono::Utc::now() - chrono::Duration::hours(48);
    repo.create(&bound).await.expect("create bound");
    assert!(session_idle_expired(bound.updated_at, Some(1.0)));

    repo.archive(bound.id).await.expect("archive idle bound");
    let new_session = svc
        .create_session(Some(title))
        .await
        .expect("create replacement");

    assert_ne!(new_session.id, bound.id);
    let archived = svc.get_session(bound.id).await.expect("get").expect("row");
    assert!(archived.is_archived());
}

// ── Forum-topic session isolation (#215) ──────────────────────────────────

#[test]
fn chat_id_suffix_topic_vs_base_format() {
    // Base chats keep the bare suffix; a forum topic carries :topic:<id>.
    assert_eq!(chat_id_suffix(-100, None), "[chat:-100]");
    assert_eq!(chat_id_suffix(-100, Some(42)), "[chat:-100:topic:42]");
}

#[test]
fn build_session_title_carries_topic_suffix() {
    let base = build_session_title(false, "", 0, "Build", -100, None, None);
    let topic = build_session_title(false, "", 0, "Build", -100, Some(42), None);
    assert_eq!(base, "Telegram: Build [chat:-100]");
    assert_eq!(topic, "Telegram: Build [chat:-100:topic:42]");
}

#[test]
fn build_session_title_shows_topic_name_keeps_numeric_suffix() {
    // The topic NAME goes in the display so the session reads as "Devops",
    // but the resolution suffix stays the numeric `:topic:2` form.
    let named = build_session_title(false, "", 0, "Build", -100, Some(2), Some("Devops"));
    assert_eq!(named, "Telegram: Build / Devops [chat:-100:topic:2]");
    assert!(
        named.ends_with("[chat:-100:topic:2]"),
        "suffix must stay numeric"
    );

    // A blank/whitespace name falls back to the plain title (no trailing slash).
    let blank = build_session_title(false, "", 0, "Build", -100, Some(2), Some("  "));
    assert_eq!(blank, "Telegram: Build [chat:-100:topic:2]");

    // A name with no topic_id (non-forum) is ignored.
    let no_topic = build_session_title(false, "", 0, "Build", -100, None, Some("Devops"));
    assert_eq!(no_topic, "Telegram: Build [chat:-100]");
}

#[test]
fn refresh_label_promotes_id_titled_topic_session_to_named() {
    // A topic session created before we learned the name (id-titled) gets
    // refreshed to the named form on the next message — rename-on-learn.
    let id_titled = "Telegram: Build [chat:-100:topic:2]";
    let named = build_session_title(false, "", 0, "Build", -100, Some(2), Some("Devops"));
    assert!(
        should_refresh_label(id_titled, &named),
        "id-titled topic session should be promoted to the named form"
    );
}

#[test]
fn topic_session_id_gates_on_is_topic_message() {
    // A real forum-topic message isolates...
    assert_eq!(topic_session_id(true, Some(7)), Some(7));
    // ...but a plain reply-thread (thread_id present, NOT a topic) must not,
    // nor a DM / non-forum group / General topic (is_topic_message == false).
    assert_eq!(topic_session_id(false, Some(7)), None);
    assert_eq!(topic_session_id(false, None), None);
    assert_eq!(topic_session_id(true, None), None);
}

/// #1220: General-topic normalization for known forums.
#[test]
fn normalize_topic_gives_general_its_own_bucket_in_known_forums() {
    use crate::channels::telegram::session_resolve::{GENERAL_TOPIC_ID, normalize_topic};

    // Raw topic resolution always wins — normalization only fills the gap.
    assert_eq!(normalize_topic(Some(5), true), Some(5));
    assert_eq!(normalize_topic(Some(5), false), Some(5));

    // Known forum + no explicit topic = the General topic: own bucket.
    assert_eq!(normalize_topic(None, true), Some(GENERAL_TOPIC_ID));

    // Unknown chat (cold start) or genuinely topicless chat: legacy None.
    assert_eq!(normalize_topic(None, false), None);
}

/// #1220: evidence-based forum detection on TelegramState, gated per #1708.
#[tokio::test]
async fn thread_evidence_marks_forum_once_and_only_from_topic_flagged_msgs() {
    let state = TelegramState::new();
    let chat = 777_i64;

    // No evidence yet — cold start keeps legacy behaviour.
    assert!(!state.is_known_forum(chat).await);

    // #1708: a bare thread id WITHOUT is_topic_message is an ordinary reply
    // chain in a non-forum group (production: OpenCrabbify produced 215 then
    // 220 from plain replies). It must stay inert — trusting it flipped the
    // chat to "forum" permanently and orphaned its session.
    state.note_thread_evidence(chat, false, Some(42)).await;
    assert!(
        !state.is_known_forum(chat).await,
        "#1708: a bare reply-chain thread id is NOT forum evidence"
    );

    // A thread-scoped message Telegram itself flagged proves forum-ness...
    state.note_thread_evidence(chat, true, Some(42)).await;
    assert!(state.is_known_forum(chat).await);

    // ...and later topicless traffic never un-proves it.
    state.note_thread_evidence(chat, true, None).await;
    assert!(state.is_known_forum(chat).await);

    // A different chat stays unknown.
    assert!(!state.is_known_forum(888).await);
}

#[tokio::test]
async fn chat_map_isolates_topic_from_base_session() {
    // The reverse map keys on (chat_id, topic_id): the same supergroup chat
    // resolves to different sessions for the base/General vs a forum topic,
    // and an unbound topic returns None rather than leaking the base session.
    let state = TelegramState::new();
    let chat_id = -1009_i64;
    let base = Uuid::new_v4();
    let topic = Uuid::new_v4();

    state.register_session_chat(base, chat_id, None).await;
    state.register_session_chat(topic, chat_id, Some(5)).await;

    assert_eq!(state.chat_session(chat_id, None).await, Some(base));
    assert_eq!(state.chat_session(chat_id, Some(5)).await, Some(topic));
    assert_eq!(state.chat_session(chat_id, Some(99)).await, None);
}

#[tokio::test]
async fn base_and_topic_titles_do_not_cross_resolve() {
    // The suffix LIKE '%suffix' lookup must keep base and topic sessions
    // separate: a base title never ends with :topic:<n>], and a topic title
    // never ends with the bare [chat:<id>], so neither suffix matches the
    // other's row.
    let (_db, repo) = fresh_repo().await;
    let chat_id = -100_i64;

    let base_title = build_session_title(false, "", 0, "G", chat_id, None, None);
    let topic_title = build_session_title(false, "", 0, "G", chat_id, Some(7), None);
    let base = Session::new(Some(base_title), None, None);
    let topic = Session::new(Some(topic_title), None, None);
    repo.create(&base).await.expect("create base");
    repo.create(&topic).await.expect("create topic");

    let base_hit = repo
        .find_by_title_suffix(&chat_id_suffix(chat_id, None))
        .await
        .expect("query base")
        .expect("base hit");
    assert_eq!(base_hit.id, base.id, "base suffix must not match topic row");

    let topic_hit = repo
        .find_by_title_suffix(&chat_id_suffix(chat_id, Some(7)))
        .await
        .expect("query topic")
        .expect("topic hit");
    assert_eq!(
        topic_hit.id, topic.id,
        "topic suffix must not match base row"
    );
}

#[tokio::test]
async fn service_update_session_title_preserves_suffix() {
    let db = Database::connect_in_memory().await.expect("connect");
    db.run_migrations().await.expect("migrations");
    let ctx = ServiceContext::new(db.pool().clone());
    let svc = SessionService::new(ctx);

    let title = build_session_title(true, "U", 1, "", 77, None, None);
    let session = svc
        .create_session(Some(title.clone()))
        .await
        .expect("create");

    let new_title = format!("Telegram: Custom topic {}", chat_id_suffix(77, None));
    svc.update_session_title(session.id, Some(new_title.clone()))
        .await
        .expect("rename");

    let loaded = svc
        .get_session(session.id)
        .await
        .expect("get")
        .expect("row");
    assert_eq!(loaded.title.as_deref(), Some(new_title.as_str()));
    assert!(
        loaded.title.as_ref().unwrap().ends_with("[chat:77]"),
        "suffix must remain for lookup"
    );
}
