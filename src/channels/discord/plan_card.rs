//! Persistent per-session plan card for Discord (#1912, Option A).
//!
//! Telegram renders the live plan as one message it edits in place
//! (`telegram/plan_card.rs`, #580). Discord had no plan surface at all: a
//! member watching a long turn could see the tool bubbles and the clock and
//! nothing about the checklist the agent is working through.
//!
//! Option A is the full card, and the constraint that comes with it is edit
//! traffic: the flow ticker (#1843) already rewrites the flow bubble every
//! 4 s, so a second live surface on the same channel must not become a second
//! edit storm. Two things bound it, and both are load-bearing:
//!
//! - **Signature dedupe.** The card's signature is its rendered text, so an
//!   unchanged checklist paints NOTHING: no edit call, no round trip. A tick
//!   over an unchanged plan costs zero writes, which is what makes sharing the
//!   ticker's cadence safe. #809 persists the same signature so a restart does
//!   not lose that dedupe.
//! - **The governor's surface classes (#1910).** A live repaint is CHROME: the
//!   next tick restates the same checklist, so a refusal is free and the write
//!   is dropped instead of held. A completing paint is FINAL: it is the last
//!   word this card will ever get, so it holds until it lands. The first paint
//!   of a session's card is a SEND, because nothing else in the channel
//!   carries the checklist yet.
//!
//! State is the channel-agnostic `PlanCardRepository` (#809) with an
//! in-memory mirror beside it, exactly like Telegram's `plan_cards` map: the
//! map is the fast path, the row is what survives a restart. The row is keyed
//! by session and carries a chat id, so a row belonging to another chat is
//! never edited or overwritten here: a session that ran a card on Telegram
//! keeps it, and Discord declines to double-own the same row.
//!
//! No Approve/Discard keyboard. Telegram's card carries inline buttons because
//! its plan mode gates on them; Discord gates approval through `/execute` and
//! the command surface, so this card is display-only.

use std::sync::Arc;

use serenity::builder::{CreateMessage, EditMessage};
use serenity::model::id::{ChannelId, MessageId};
use uuid::Uuid;

use super::governor::{self, Surface};
use super::state::DiscordState;
use crate::db::repository::PlanCard;
use crate::tui::plan::{PlanDocument, PlanStatus, status_mark};

/// Per-row text cap. A task title is a sentence, not a paragraph; a wrapped
/// row eats the space a later task would have used.
const ROW_TEXT_CAP: usize = 90;

/// Title cap on the card header.
const TITLE_CAP: usize = 120;

/// Discord counts 2000 code points per message. The card stays under that
/// with headroom for the dropped-row tail it appends itself.
const CARD_CHAR_CAP: usize = 1900;

/// Escape the markdown and mention syntax a plan title or task can carry.
///
/// Card text is model-authored, and Discord renders markdown in ordinary
/// messages: an unescaped `*foo*` turns a task title into bold noise, and
/// `@everyone`, `@here` or `<@123456789>` in a title would ping a whole guild
/// from a checklist row. Backslash is Discord's own escape; Telegram solves
/// the same problem with `escape_html`.
pub(crate) fn escape_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for c in text.chars() {
        match c {
            '*' | '_' | '~' | '`' | '>' | '<' | '|' | '@' => {
                out.push('\\');
                out.push(c);
            }
            other => out.push(other),
        }
    }
    out
}

/// The plan's own status word. `PlanStatus` serializes by hand and has no
/// `Display`, so the card names the two states here rather than printing a
/// `Debug` form no user should read.
pub(crate) fn status_word(status: &PlanStatus) -> &'static str {
    match status {
        PlanStatus::Editing => "Editing",
        PlanStatus::Active => "Active",
    }
}

/// Which paint this refresh owes the channel.
///
/// `finalizing` is the completing settle: the checklist is archived and this
/// is the last thing the card will ever say, so it may not be dropped.
pub(crate) fn surface_for(finalizing: bool) -> Surface {
    if finalizing {
        Surface::Final
    } else {
        Surface::Chrome
    }
}

/// Whether a repaint is needed at all. The same signature means the card on
/// screen already says exactly this, so the answer is zero API calls.
pub(crate) fn should_paint(previous: Option<&str>, rendered: &str) -> bool {
    previous.is_none_or(|prev| prev != rendered)
}

/// Does this tracked row belong to the channel we are about to write to?
///
/// The `plan_cards` table is keyed by session, not by platform, and one
/// session id can be seen by more than one channel. Editing a foreign message
/// id here would fail anyway, and overwriting the row would leave the other
/// platform's card with no tracker to edit or delete it.
pub(crate) fn card_is_ours(stored_chat_id: i64, channel: u64) -> bool {
    stored_chat_id == channel as i64
}

/// What this refresh should render, and what it must never resurrect.
///
/// The #809 rule, kept from Telegram verbatim in spirit: there is NO archive
/// fallback. "No live plan" stays true forever once a plan completes, so
/// deriving the final state from absence would repaint a finished checklist on
/// every later refresh, contaminating a chat that moved on. `may_finalize` is
/// the caller's `finalizing && a card is still tracked`, so exactly one refresh
/// per completed plan can ever read the archive: the paint that uses it
/// releases the tracker.
#[derive(Debug, Clone)]
pub(crate) enum PaintTarget {
    /// Nothing to show: no live plan, and this is not the completing settle.
    Skip,
    /// Paint the live plan.
    Live(PlanDocument),
    /// Paint the plan that just archived, exactly once.
    Finalizing(PlanDocument),
}

pub(crate) fn decide_paint(
    live: Option<PlanDocument>,
    may_finalize: bool,
    archived: Option<PlanDocument>,
) -> PaintTarget {
    if let Some(plan) = live {
        return PaintTarget::Live(plan);
    }
    if may_finalize && let Some(plan) = archived {
        return PaintTarget::Finalizing(plan);
    }
    PaintTarget::Skip
}

/// Whether a failed card edit means the card is GONE (untrack it, so the next
/// refresh posts a new one) or merely refused (keep tracking, retry later).
///
/// Untracking on a transient error would post a second card and strand the
/// first uneditable and undeletable, which is the #822 failure this module
/// exists to avoid; keeping a tracker on a deleted message would edit nothing
/// forever. So only "that message does not exist any more" (or we may not
/// touch it) releases the tracker.
pub(crate) fn untracks_the_card(error: &str) -> bool {
    let text = error.to_lowercase();
    text.contains("unknown message")
        || text.contains("unknown channel")
        || text.contains("10008")
        || text.contains("10003")
        || text.contains("404")
        || text.contains("403")
}

/// Render the card: header, progress line, one row per task.
pub(crate) fn render_plan_card(plan: &PlanDocument) -> String {
    let total = plan.tasks.len();
    let done = plan
        .tasks
        .iter()
        .filter(|t| matches!(t.status, crate::tui::plan::TaskStatus::Completed))
        .count();

    let mut out = String::new();
    if !plan.title.trim().is_empty() {
        out.push_str(&format!(
            "**📋 {}**\n",
            escape_markdown(crate::utils::truncate_str(plan.title.trim(), TITLE_CAP))
        ));
    }
    out.push_str(&format!(
        "`{done}/{total} done` · {}\n\n",
        status_word(&plan.status)
    ));

    if total == 0 {
        out.push_str("_No checklist yet._");
        return out;
    }

    let mut rows = String::new();
    let mut painted = 0usize;
    for task in &plan.tasks {
        let mut row = format!(
            "{} {}",
            status_mark(&task.status),
            escape_markdown(crate::utils::truncate_str(task.title.trim(), ROW_TEXT_CAP))
        );
        row.push_str(&crate::tui::plan::quality_glyph_suffix(task));
        if let Some(verdict) = task.verification.as_ref() {
            row.push_str(&format!(" {}", verdict.badge()));
        }
        if out.chars().count() + rows.chars().count() + row.chars().count() + 1 > CARD_CHAR_CAP {
            break;
        }
        rows.push_str(&row);
        rows.push('\n');
        painted += 1;
    }
    out.push_str(&rows);
    if painted < total {
        out.push_str(&format!("… +{} more", total - painted));
    } else {
        out.pop(); // the last row's newline is not needed
    }
    out
}

fn repository() -> Option<crate::db::repository::PlanCardRepository> {
    let pool = crate::db::database::global_pool()?.clone();
    Some(crate::db::repository::PlanCardRepository::new(pool))
}

/// Refresh (or first-post) this session's plan card in `channel`.
///
/// Called from the flow ticker while a turn is live and from a settling turn
/// with `finalizing = true`. Everything is serialized per session (#822, same
/// reason Telegram holds the lock across its API calls): two concurrent
/// refreshes that both see no card both post one, and the second id
/// overwrites the first in the tracker, leaving a card nobody can ever edit or
/// delete.
pub(crate) async fn refresh_plan_card(
    dstate: &DiscordState,
    http: &serenity::http::Http,
    channel: ChannelId,
    session_id: Uuid,
    finalizing: bool,
) {
    let card_lock = dstate.plan_card_lock(session_id).await;
    let _guard = card_lock.lock().await;

    // The tracker is read first because it doubles as the one-shot guard: a
    // card still on screen can be finalized exactly once, and finalizing
    // releases the tracker, so nothing later can finalize it again.
    let repo = repository();
    let tracked = dstate
        .plan_card(session_id, channel.get(), repo.as_ref())
        .await;

    let live = crate::utils::plan_files::load_plan(session_id).await;
    // #809's anti-resurrection rule, carried over from Telegram: the archive is
    // read ONLY by the settle that completes a card which is still tracked.
    // Nothing tracked means it was either finalized already or never posted,
    // and repainting an old archive in either case contaminates a chat that has
    // moved on. The on-disk `just_archived` stamp is deliberately NOT the guard
    // here: no call site in this repo writes it any more (`mark_plan_just_archived`
    // has zero callers), so a finalize keyed on it could never fire. That dead
    // stamp is filed as its own issue rather than being propped up from here.
    let may_finalize = finalizing && tracked.is_some();
    let archived = if live.is_none() && may_finalize {
        crate::utils::plan_files::latest_archived_plan(session_id).await
    } else {
        None
    };
    let plan = match decide_paint(live, may_finalize, archived) {
        PaintTarget::Skip => return,
        PaintTarget::Live(plan) | PaintTarget::Finalizing(plan) => plan,
    };

    let rendered = render_plan_card(&plan);
    if !should_paint(tracked.as_ref().map(|(_, sig)| sig.as_str()), &rendered) {
        return;
    }

    // Where this paint goes, decided once: no tracker means a new message, a
    // finalizing settle means the never-dropped path, anything else means
    // droppable chrome.
    let surface = match &tracked {
        None => Surface::Send,
        Some(_) => surface_for(finalizing),
    };
    let target = match (tracked, surface) {
        (None, _) => Paint::Post,
        (Some((mid, _)), Surface::Final) => Paint::Final(mid),
        (Some((mid, _)), _) => Paint::Repaint(mid),
    };
    let message_id = match target {
        Paint::Post => {
            let builder = CreateMessage::new().content(rendered.clone());
            match governor::send_content(&dstate.governor, channel, http, Surface::Send, builder)
                .await
            {
                Ok(sent) => sent.id.get(),
                Err(e) => {
                    tracing::warn!("Discord: plan card post failed: {e}");
                    return;
                }
            }
        }
        Paint::Repaint(mid) => {
            let edit = EditMessage::new().content(rendered.clone());
            match governor::edit_chrome(&dstate.governor, channel, http, MessageId::new(mid), edit)
                .await
            {
                None => return, // dropped: the next tick restates this checklist
                Some(Err(e)) => {
                    // Either way this paint is lost: the signature is NOT
                    // updated, so the next tick retries the same text. If the
                    // card itself is gone, release_tracker drops the tracker
                    // first so the retry posts a fresh card instead of editing
                    // an id that never resolves.
                    release_tracker(dstate, session_id, &e).await;
                    return;
                }
                Some(Ok(_)) => mid,
            }
        }
        Paint::Final(mid) => {
            let edit = EditMessage::new().content(rendered.clone());
            match governor::edit_content(
                &dstate.governor,
                channel,
                http,
                MessageId::new(mid),
                Surface::Final,
                edit,
            )
            .await
            {
                Ok(_) => mid,
                Err(e) => {
                    release_tracker(dstate, session_id, &e).await;
                    return;
                }
            }
        }
    };

    let row = PlanCard {
        session_id: session_id.to_string(),
        chat_id: channel.get() as i64,
        thread_id: None,
        message_id: message_id as i64,
        signature: rendered.clone(),
    };
    if finalizing {
        // Last word said. Release the tracker and its row: the next settle
        // finds nothing tracked, takes the `may_finalize` branch off, and
        // leaves this finished checklist alone for good.
        dstate.forget_plan_card(session_id, repo.as_ref()).await;
        return;
    }
    dstate
        .remember_plan_card(
            session_id,
            channel.get(),
            message_id,
            rendered,
            row,
            repo.as_ref(),
        )
        .await;
}

/// Where a decided paint goes.
enum Paint {
    Post,
    Repaint(u64),
    Final(u64),
}

/// A failed edit that proves the card is gone releases the tracker so the next
/// refresh posts a new one; a transient refusal keeps it. Returns whether the
/// tracker was released.
async fn release_tracker(dstate: &DiscordState, session_id: Uuid, error: &serenity::Error) -> bool {
    if !untracks_the_card(&error.to_string()) {
        tracing::warn!("Discord: plan card edit refused, keeping the tracker: {error}");
        return false;
    }
    tracing::warn!("Discord: plan card is gone, untracking it: {error}");
    dstate
        .forget_plan_card(session_id, repository().as_ref())
        .await;
    true
}

/// Ticker seam: the session bound to this channel gets its card refreshed.
/// Puts the plan surface on the SAME cadence and the SAME per-channel budget
/// the flow bubble already uses, instead of adding a second timer.
pub(crate) async fn refresh_for_channel(
    dstate: &DiscordState,
    http: &serenity::http::Http,
    channel: ChannelId,
) {
    let Some(session_id) = dstate.session_owner_by_channel(channel.get()).await else {
        return;
    };
    refresh_plan_card(dstate, http, channel, session_id, false).await;
}

impl DiscordState {
    /// The tracker for this session's card in this channel, if it is ours.
    ///
    /// On a miss, rehydrate from the persisted row (#809): after a restart the
    /// map is empty while the message still sits in the channel, and posting a
    /// second card next to the survivor is exactly the duplicate-card bug the
    /// rehydrate exists to prevent. A row owned by another chat is left
    /// untouched and reported as absent, so neither the foreign message nor the
    /// foreign row is ever overwritten from here.
    pub(crate) async fn plan_card(
        &self,
        session_id: Uuid,
        channel: u64,
        repo: Option<&crate::db::repository::PlanCardRepository>,
    ) -> Option<(u64, String)> {
        if let Some((card_channel, message_id, signature)) =
            self.plan_cards.lock().await.get(&session_id).cloned()
        {
            return card_is_ours(card_channel as i64, channel).then_some((message_id, signature));
        }
        let repo = repo?;
        let Ok(Some(row)) = repo.get(&session_id.to_string()).await else {
            return None;
        };
        if !card_is_ours(row.chat_id, channel) {
            return None;
        }
        tracing::info!(
            "Discord: rehydrated plan card for session {} (message {})",
            session_id,
            row.message_id
        );
        let hit = (channel, row.message_id as u64, row.signature.clone());
        self.plan_cards.lock().await.insert(session_id, hit);
        Some((row.message_id as u64, row.signature))
    }

    /// Per-session card lock, created on demand (#822).
    pub(crate) async fn plan_card_lock(&self, session_id: Uuid) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.plan_card_locks.lock().await;
        locks
            .entry(session_id)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    /// Record a successful paint: the in-memory tracker, plus the DB row that
    /// lets the next process dedupe instead of reposting.
    pub(crate) async fn remember_plan_card(
        &self,
        session_id: Uuid,
        channel: u64,
        message_id: u64,
        signature: String,
        row: PlanCard,
        repo: Option<&crate::db::repository::PlanCardRepository>,
    ) {
        self.plan_cards
            .lock()
            .await
            .insert(session_id, (channel, message_id, signature));
        // A write failure here is not fatal to the card: the in-memory tracker
        // already prevents the duplicate for this process, and the next
        // successful paint retries. Losing it entirely (no pool) just means a
        // restart posts a fresh card, which is the pre-#809 behaviour.
        if let Some(repo) = repo
            && let Err(e) = repo.set(row).await
        {
            tracing::warn!("Discord: plan card persistence failed for {session_id}: {e}");
        }
    }

    /// Drop the tracker, and the row with it, after an edit proved the card is
    /// gone. The next refresh then posts a fresh card instead of editing an id
    /// that no longer resolves.
    pub(crate) async fn forget_plan_card(
        &self,
        session_id: Uuid,
        repo: Option<&crate::db::repository::PlanCardRepository>,
    ) {
        self.plan_cards.lock().await.remove(&session_id);
        if let Some(repo) = repo
            && let Err(e) = repo.delete(&session_id.to_string()).await
        {
            tracing::warn!("Discord: plan card row delete failed for {session_id}: {e}");
        }
    }
}
