//! Plan cards must be readable by the agent that posted them (#1684).
//!
//! Every ordinary bot bubble is recorded in `channel_messages` by
//! `persist_telegram_messages` before it is ever relayed. The plan card was the
//! one exception: `plan_card.rs` posts it directly, so no row ever existed for
//! its message id. The consequence is the reply-context branch
//! `handler.rs:unrecoverable_bot_reply` answering "no stored message for id"
//! about a card sitting there on screen, no reaction attribution, and no
//! `channel_search` hit for the agent's own plan.
//!
//! These tests drive the real repository through the real record/forget
//! helpers. Chat, topic and message ids are synthetic and carry no user
//! identifiers.

use crate::channels::telegram::TelegramState;
use crate::channels::telegram::plan_card::{forget_card, record_card};
use crate::db::Database;
use crate::db::models::ChannelMessage;
use crate::db::repository::channel_message::ChannelMessageRepository;
use std::sync::Arc;
use teloxide::types::{ChatId, MessageId, ThreadId};

/// A supergroup, a forum topic and a message id, with no Telegram involved.
/// `MessageId` is `i32` and `ThreadId` wraps a `MessageId` in teloxide 0.13,
/// while `ChatId` is `i64`: the types here are the library's, not preferences.
const CHAT: i64 = -1_004_428_873_948;
const TOPIC: i32 = 3471;
const CARD_MESSAGE: i32 = 4261;

async fn state_with_store() -> (Arc<TelegramState>, ChannelMessageRepository) {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let repo = ChannelMessageRepository::new(db.pool().clone());
    let state = Arc::new(TelegramState::new());
    state.set_channel_message_store(repo.clone()).await;
    (state, repo)
}

fn chat() -> ChatId {
    ChatId(CHAT)
}

fn thread() -> Option<ThreadId> {
    Some(ThreadId(MessageId(TOPIC)))
}

fn card_id() -> MessageId {
    MessageId(CARD_MESSAGE)
}

/// Every stored row claiming this card's message id. One is the only legal
/// answer while the card is alive: the bug is a card with no row at all, and
/// its mirror image is a card that grew one row per refresh.
async fn card_rows(repo: &ChannelMessageRepository) -> Vec<ChannelMessage> {
    let want = CARD_MESSAGE.to_string();
    repo.recent(Some("telegram"), &CHAT.to_string(), 50, None, None)
        .await
        .unwrap()
        .into_iter()
        .filter(|m| m.platform_message_id.as_deref() == Some(want.as_str()))
        .collect()
}

#[tokio::test]
async fn a_card_render_writes_one_row_a_reply_can_read() {
    let (state, repo) = state_with_store().await;
    let rendered = "<b>Wave 3</b>\nland the queue fix";

    record_card(&state, chat(), thread(), card_id(), rendered).await;

    let rows = card_rows(&repo).await;
    assert_eq!(
        rows.len(),
        1,
        "a plan card render must leave exactly one channel_messages row"
    );
    let row = &rows[0];
    assert_eq!(
        row.sender_id, "bot:opencrabs",
        "the card must be attributed to the bot, the way delivery.rs does it"
    );
    assert_eq!(
        row.thread_id.as_deref(),
        Some(TOPIC.to_string().as_str()),
        "a card in a forum topic must store its thread_id or reactions attribute to nothing"
    );

    // The exact read that failed in the incident: a reply to the card resolving
    // its own content instead of `unrecoverable_bot_reply`.
    let content = repo
        .content_by_platform_message_id("telegram", &CHAT.to_string(), &CARD_MESSAGE.to_string())
        .await
        .unwrap()
        .unwrap_or_else(|| {
            panic!("replying to plan card {CARD_MESSAGE} reads no stored message: that is #1684")
        });
    assert!(
        content.contains("land the queue fix"),
        "the row must hold what the card shows, got {content:?}"
    );
    assert!(
        !content.contains("<b>"),
        "the row stores readable text, not the HTML wire form: {content:?}"
    );
}

#[tokio::test]
async fn refreshes_of_one_card_update_in_place() {
    let (state, repo) = state_with_store().await;

    record_card(&state, chat(), thread(), card_id(), "9/12 done").await;
    record_card(&state, chat(), thread(), card_id(), "10/12 done").await;
    record_card(&state, chat(), thread(), card_id(), "11/12 done").await;

    let rows = card_rows(&repo).await;
    assert_eq!(
        rows.len(),
        1,
        "three refreshes of one card must not become three rows: got {}",
        rows.len()
    );

    let (content, thread_id) = repo
        .bot_message_with_thread("telegram", &CHAT.to_string(), &CARD_MESSAGE.to_string())
        .await
        .unwrap()
        .expect("the refreshed card must still resolve for the reaction path");
    assert_eq!(
        content, "11/12 done",
        "latest-wins: the row holds the render"
    );
    assert_eq!(
        thread_id.as_deref(),
        Some(TOPIC.to_string().as_str()),
        "an in-place update must not lose the topic"
    );
}

#[tokio::test]
async fn a_deleted_card_takes_its_row_with_it() {
    let (state, repo) = state_with_store().await;
    record_card(&state, chat(), thread(), card_id(), "completed plan").await;
    assert_eq!(card_rows(&repo).await.len(), 1);

    forget_card(&state, chat(), card_id()).await;

    assert_eq!(
        card_rows(&repo).await.len(),
        0,
        "a card deleted for real must not stay readable in history"
    );
}

#[tokio::test]
async fn a_card_is_findable_by_history_search() {
    let (state, repo) = state_with_store().await;
    record_card(
        &state,
        chat(),
        thread(),
        card_id(),
        "plan: land the queue fix then the card fix",
    )
    .await;

    // The agent's read path: what did I say in this chat? Before #1684 the card
    // never appeared, so a quote of it was invisible to the agent itself.
    let hits = repo
        .search_history("telegram", &CHAT.to_string(), "land the queue fix", 0, 10)
        .await
        .unwrap();
    assert!(
        hits.iter().any(|(content, _, _)| content.contains("plan:")),
        "channel_search must find the agent's own plan card, got {hits:?}"
    );
}

#[test]
fn every_tracked_card_render_in_refresh_plan_card_records_a_row() {
    let src = include_str!("../channels/telegram/plan_card.rs");
    let start = src
        .find("async fn refresh_plan_card(")
        .expect("the refresh body");
    let end = start
        + src[start..]
            .find("\n}\n")
            .expect("the body ends at its closing brace");
    let body = &src[start..end];

    let sets: Vec<usize> = body
        .match_indices(".set_plan_card(")
        .map(|(i, _)| i)
        .collect();
    assert!(
        !sets.is_empty(),
        "the refresh body has no card-tracking sites; this sentinel guards nothing"
    );
    for at in &sets {
        // The tracker write and the row write describe the same bubble, so
        // between one `.set_plan_card(` and the next site (or the end of the
        // body) there must be a `record_card(`.
        let next = sets
            .iter()
            .find(|n| **n > *at)
            .copied()
            .unwrap_or(body.len());
        let region = &body[*at..next];
        assert!(
            region.contains("record_card("),
            "a tracked card render at offset {at} records no channel_messages row"
        );
    }
}

#[test]
fn both_card_removal_sites_forget_the_row() {
    let src = include_str!("../channels/telegram/plan_card.rs");
    let deletes = src.match_indices("bot.delete_message(").count();
    // Count the CALLS, not the definition: `async fn forget_card(` also
    // contains "forget_card(", so matching the bare name overcounts by one.
    let forgets = src
        .lines()
        .filter(|l| l.contains("forget_card(state, chat, mid)"))
        .count();
    assert_eq!(deletes, forgets);
    assert_eq!(
        deletes, 2,
        "expected exactly two card removal sites (finalize stale delete + remove_plan_card_locked)"
    );
}

#[test]
fn the_startup_wiring_hands_the_card_module_a_message_store() {
    let src = include_str!("../cli/ui.rs");
    assert!(
        src.contains("set_channel_message_store"),
        "the plan card's channel_messages writer is wired nowhere, so record_card \
         silently no-ops and #1684 is back"
    );
}
