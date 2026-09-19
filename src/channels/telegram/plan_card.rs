//! Persistent per-session plan card (#580): a single Telegram message that
//! shows the plan title + checklist and the Approve/Discard keyboard, edited in
//! place across the creation/execution/completion turns instead of re-rendered
//! inside each per-turn flow block. Tracked cross-turn on [`TelegramState`], so
//! there is exactly one card at a time rather than one checklist per turn.

use super::TelegramState;
use super::flow_chrome::{
    GoalSection, PlanKb, ProseSection, load_goal_section, load_plan_prose, load_plan_sections,
};
use super::handler::escape_html;
use super::send::message_in_thread;
use crate::brain::agent::AgentService;
use crate::config::Config;
use crate::utils::truncate_chars;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{MessageId, ParseMode, ThreadId};
use uuid::Uuid;

/// Total character budget for prose bodies on the classic card. The card
/// carries the title, checklist rows, and keyboard inside Telegram's 4096-char
/// message cap; sections past the budget are dropped (full prose via
/// /show-plan). The rich path (`sendRichMessage`, 32K chars) needs no budget.
const CARD_PROSE_BUDGET: usize = 2400;

/// Goal text budget (chars) on the classic card. The goal renders as a
/// collapsed expandable (ADR 0005 Decision 12), so the cap only trims the
/// expanded body, never the visible chrome.
const GOAL_TEXT_CAP: usize = 600;

/// Collapsible wrapper style for prose sections and goals.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum CollapsibleStyle {
    /// Classic `sendMessage` (4096 chars): `<blockquote expandable>`,
    /// prose truncated to `CARD_PROSE_BUDGET`.
    BlockquoteExpandable,
    /// Rich `sendRichMessage` (32K chars): `<details><summary>`, no truncation.
    DetailsSummary,
}

/// One section of the card, before it is committed to a target's HTML dialect.
///
/// Sections describe WHAT they are; the serializer decides how they are
/// separated. Letting each section pick its own line break is what collapsed
/// the checklist into a single line (#941): prose was converted to `<p>` for
/// the rich target and the title, checklist rows and goal separator were left
/// on bare `\n`, which the rich renderer treats as ordinary whitespace.
enum CardBlock {
    /// Inline content that must occupy its own line (the title, a checklist
    /// row). The serializer supplies whatever the target needs to break here.
    Line(String),
    /// Already block-level markup (`<details>`, `<blockquote>`, `<p>`-wrapped
    /// prose) that must not be wrapped again — `<p>` cannot contain `<details>`.
    Block(String),
    /// A blank line before the goal on the classic card. The rich serializer
    /// ignores it: block-level elements already space themselves.
    ClassicGap,
}

/// Commit the card's sections to one target's HTML dialect.
///
/// This is the ONLY place a line-break convention is chosen. Adding a section
/// cannot get it wrong, because sections no longer decide.
fn serialize_card(style: CollapsibleStyle, blocks: &[CardBlock]) -> String {
    let mut out = String::new();
    match style {
        // Classic `ParseMode::Html` is Telegram's limited dialect: a bare
        // newline IS the line break and there is no `<p>`. One newline between
        // blocks; `ClassicGap` contributes the second.
        CollapsibleStyle::BlockquoteExpandable => {
            for b in blocks {
                match b {
                    CardBlock::Line(s) | CardBlock::Block(s) => {
                        if !out.is_empty() {
                            out.push('\n');
                        }
                        out.push_str(s);
                    }
                    CardBlock::ClassicGap => {
                        if !out.is_empty() {
                            out.push('\n');
                        }
                    }
                }
            }
        }
        // `sendRichMessage` renders real HTML, where a newline is whitespace
        // and collapses. Every standalone line needs its own block-level
        // wrapper; the tags provide the spacing, so blocks join with nothing.
        CollapsibleStyle::DetailsSummary => {
            for b in blocks {
                match b {
                    CardBlock::Line(s) => {
                        out.push_str("<p>");
                        out.push_str(s);
                        out.push_str("</p>");
                    }
                    CardBlock::Block(s) => out.push_str(s),
                    CardBlock::ClassicGap => {}
                }
            }
        }
    }
    out
}

/// Unified plan card renderer. The two surfaces (classic sendMessage and rich
/// sendRichMessage) share title, checklist, and goal logic; only the
/// collapsible tag pair, the truncation budget, and the HTML dialect differ.
///
/// Returns `None` when the session has no plan content (no title and no
/// checklist) — the caller removes the card in that case.
///
/// Async because the rich arms render prose through the mermaid-aware
/// converter (`markdown_to_html_mermaid_p`), so a mermaid fence in plan
/// prose resolves to an embedded image exactly like one in a final reply
/// (#1142). The classic arms stay on the sync converter: classic
/// `sendMessage` HTML has no `<img>` tag, so a resolved image would get the
/// whole card rejected — a fence there stays a readable source code block.
async fn render_plan_card(
    style: CollapsibleStyle,
    title: Option<&str>,
    checklist: Option<&[String]>,
    prose: Option<&[ProseSection]>,
    goal: Option<&GoalSection>,
) -> Option<String> {
    let mut blocks: Vec<CardBlock> = Vec::new();

    // Title: identical for both styles.
    if let Some(t) = title.map(str::trim).filter(|t| !t.is_empty()) {
        blocks.push(CardBlock::Line(format!("📋 <b>{}</b>", escape_html(t))));
    }

    // Prose: style-dependent collapsible tags + truncation budget.
    // Locked order: title, prose expandables, checklist rows, goal.
    if let Some(sections) = prose.filter(|s| !s.is_empty()) {
        let mut budget: Option<usize> = match style {
            CollapsibleStyle::BlockquoteExpandable => Some(CARD_PROSE_BUDGET),
            CollapsibleStyle::DetailsSummary => None,
        };
        for sec in sections {
            if budget == Some(0) {
                break;
            }
            // Truncate raw text BEFORE HTML conversion so the collapsible
            // tags are always well-formed (truncating rendered HTML can
            // cut mid-tag, causing Telegram to strip rich formatting).
            let (body, chars_used) = match budget {
                Some(remaining) => {
                    let truncated = truncate_chars(&sec.body, remaining);
                    (truncated, truncated.chars().count())
                }
                None => (sec.body.as_str(), 0),
            };
            budget = budget.map(|b| b.saturating_sub(chars_used));
            // Every arm yields block-level markup: a collapsible element, or
            // prose the renderer has already wrapped for its target. The rich
            // arms go through the mermaid-aware converter (#1142); the classic
            // arms stay sync (classic HTML cannot embed <img>).
            blocks.push(CardBlock::Block(match (&sec.heading, style) {
                (Some(h), CollapsibleStyle::BlockquoteExpandable) => format!(
                    "<blockquote expandable><b>{}</b>\n{}</blockquote>",
                    escape_html(h),
                    super::rich::markdown_to_html(body),
                ),
                (Some(h), CollapsibleStyle::DetailsSummary) => format!(
                    "<details><summary><b>{}</b></summary>{}</details>",
                    escape_html(h),
                    super::rich::markdown_to_html_mermaid_p(body).await,
                ),
                (None, CollapsibleStyle::DetailsSummary) => {
                    super::rich::markdown_to_html_mermaid_p(body).await
                }
                (None, CollapsibleStyle::BlockquoteExpandable) => {
                    super::rich::markdown_to_html(body)
                }
            }));
        }
    }

    // Checklist: identical for both styles. Each row is its own line, and the
    // serializer decides what that means for the target — these rows rendering
    // as one run-on paragraph is the bug this structure prevents (#941).
    if let Some(rows) = checklist {
        for row in rows {
            blocks.push(CardBlock::Line(escape_html(row)));
        }
    }

    // Goal: style-dependent wrapper + truncation on classic only.
    if let Some(g) = goal {
        let text = g.text.trim();
        if !text.is_empty() {
            let has_prose = prose.is_some_and(|p| !p.is_empty());
            if checklist.is_some() || has_prose {
                blocks.push(CardBlock::ClassicGap);
            }
            blocks.push(CardBlock::Block(match style {
                CollapsibleStyle::BlockquoteExpandable => {
                    let capped = escape_html(truncate_chars(text, GOAL_TEXT_CAP));
                    format!(
                        "<blockquote expandable>{} {capped}</blockquote>",
                        g.prefix(true)
                    )
                }
                CollapsibleStyle::DetailsSummary => format!(
                    "<details><summary>{} goal</summary>\n{}</details>",
                    g.prefix(true),
                    escape_html(text)
                ),
            }));
        }
    }

    let out = serialize_card(style, &blocks);
    (!out.is_empty()).then_some(out)
}

/// Classic sendMessage card: `<blockquote expandable>` collapsibles, 4096-char
/// budget with per-section prose truncation.
pub(crate) async fn render_plan_card_html(
    title: Option<&str>,
    checklist: Option<&[String]>,
    prose: Option<&[ProseSection]>,
    goal: Option<&GoalSection>,
) -> Option<String> {
    render_plan_card(
        CollapsibleStyle::BlockquoteExpandable,
        title,
        checklist,
        prose,
        goal,
    )
    .await
}

/// Rich `sendRichMessage` card: `<details><summary>` collapsibles, 32K-char
/// limit, no truncation — prose renders in full.
pub(crate) async fn render_plan_card_rich_html(
    title: Option<&str>,
    checklist: Option<&[String]>,
    prose: Option<&[ProseSection]>,
    goal: Option<&GoalSection>,
) -> Option<String> {
    render_plan_card(
        CollapsibleStyle::DetailsSummary,
        title,
        checklist,
        prose,
        goal,
    )
    .await
}

/// Result of a plan card edit attempt.
enum EditOutcome {
    /// Card saved successfully (or content unchanged).
    Saved,
    /// Rate-limited: card writes suppressed for a duration.
    Suppressed,
    /// Card gone/unusable: caller should try creating fresh.
    Gone,
}

/// Classify a plan card edit failure and take the appropriate state action.
/// Handles "message is not modified" (silent success) and rate-limiting
/// (suppress future writes). Returns `Gone` when the card needs recreating.
async fn handle_edit_failure(
    error: &str,
    state: &TelegramState,
    session_id: Uuid,
    chat: ChatId,
    thread_id: Option<ThreadId>,
    signature: &str,
    mid: MessageId,
) -> EditOutcome {
    if error.contains("message is not modified") {
        state
            .set_plan_card(session_id, chat, thread_id, mid, signature.to_string())
            .await;
        return EditOutcome::Saved;
    }
    if let Some(wait) = super::rate_limit::parse_retry_after(error) {
        // A 429 learned here is a process-wide fact, not a per-card one: the
        // server is throttling this bot, and every other chat is about to hit
        // the same wall. Suppressing card writes locally teaches them nothing.
        super::rate_limit::record_global_429(wait);
        tracing::warn!(
            "Telegram plan card edit throttled for session {session_id}: {error} — \
             pausing card writes for {}s",
            wait.as_secs()
        );
        state
            .suppress_plan_card(session_id, wait + super::rate_limit::RETRY_MARGIN)
            .await;
        return EditOutcome::Suppressed;
    }
    tracing::debug!("Telegram plan card edit failed ({mid:?}): {error} — recreating");
    state.take_plan_card(session_id).await;
    EditOutcome::Gone
}

/// Classify a plan card create failure. Suppresses future writes on rate-limit,
/// warns on other errors.
async fn handle_create_failure(error: &str, state: &TelegramState, session_id: Uuid) {
    if let Some(wait) = super::rate_limit::parse_retry_after(error) {
        // Same as the edit path: record it globally before suppressing locally.
        super::rate_limit::record_global_429(wait);
        tracing::warn!(
            "Telegram plan card create throttled for session {session_id}: {error} — \
             pausing card writes for {}s",
            wait.as_secs()
        );
        state
            .suppress_plan_card(session_id, wait + super::rate_limit::RETRY_MARGIN)
            .await;
    } else {
        tracing::warn!("Telegram plan card create failed: {error}");
    }
}

/// Create or update the session's plan card to reflect the live plan state,
/// carrying `plan_kb`. Removes the card when the plan is gone.
///
/// When `rich_messages` is enabled, the card is sent via `sendRichMessage`
/// (32K char limit, native `<details><summary>` collapsibles). On any rich
/// API failure, falls back to the classic HTML `sendMessage` path (4096 chars,
/// `<blockquote expandable>`).
pub(crate) async fn refresh_plan_card(
    bot: &Bot,
    chat: ChatId,
    thread_id: Option<ThreadId>,
    state: &Arc<TelegramState>,
    agent: &AgentService,
    session_id: Uuid,
    plan_kb: PlanKb,
) {
    // Telegram asked us to wait. The card is chrome, so skipping an update
    // beats renewing the flood-control window on every refresh (#814).
    // Checked BEFORE taking the lock, so a throttled session releases waiters
    // immediately instead of queueing them behind a write that will not happen.
    if state.plan_card_suppressed(session_id).await {
        return;
    }

    // Serialise everything below (#822). The sequence is check-whether-a-card-
    // is-tracked, decide edit-or-post, record the id — and with nothing held
    // across it two concurrent refreshes both saw no card, both posted, and the
    // second id overwrote the first. The loser was left visible in the chat but
    // untracked, so it could never be edited or deleted again.
    //
    // Held across the API calls, not just the map reads: releasing before the
    // create is exactly what leaves the window open.
    let card_lock = state.plan_card_lock(session_id).await;
    let _guard = card_lock.lock().await;
    let (title, checklist) = load_plan_sections(session_id).await;
    let prose = load_plan_prose(session_id).await;
    let goal = if checklist.is_some() {
        load_goal_section(agent, session_id)
            .await
            .map(|(text, completed)| GoalSection { text, completed })
    } else {
        None
    };
    let use_rich = Config::current().channels.telegram.rich_messages;

    // Try rich path first when enabled: sendRichMessage (32K, native
    // <details><summary> collapsibles) with reply_markup for the keyboard.
    if use_rich
        && let Some(rich_html) = render_plan_card_rich_html(
            title.as_deref(),
            checklist.as_deref(),
            prose.as_deref(),
            goal.as_ref(),
        )
        .await
    {
        let kb_val = plan_kb
            .keyboard()
            .and_then(|m| serde_json::to_value(m).ok());
        let rich_sig = format!("rich:{rich_html}\u{1}{plan_kb:?}");
        if let Some((mid, last_sig)) = state.plan_card(session_id).await {
            if last_sig == rich_sig {
                return;
            }
            // G2 flood governor (#1211): plan-card refreshes are FINAL class —
            // never dropped. When the edit bucket is empty the payload queues
            // latest-wins and the governor's drainer lands it on refill; the
            // tracked signature is saved now so identical later refreshes skip
            // (a permanently failed queue drain self-heals on the next
            // differing-content plan change). Media and keyboard ride the
            // queued final via edit_admission_media_kb (owner law: extend,
            // never bypass the governor, #155, #229).
            let admitted = super::governor::edit_admission_media_kb(
                bot,
                chat,
                mid,
                super::governor::EditClass::Final,
                rich_html.clone(),
                true,
                Vec::new(),
                kb_val.clone(),
                super::governor::FinalDialect::Html,
            )
            .await;
            if !admitted {
                state
                    .set_plan_card(session_id, chat, thread_id, mid, rich_sig)
                    .await;
                return;
            }
            match super::rich::api::edit_rich_html(
                bot.api_url().as_str(),
                bot.token(),
                chat.0,
                mid.0,
                &rich_html,
                kb_val.as_ref(),
                "turn",
                "-",
            )
            .await
            {
                Ok(()) => {
                    state
                        .set_plan_card(session_id, chat, thread_id, mid, rich_sig)
                        .await;
                    return;
                }
                Err(e) => {
                    let outcome = handle_edit_failure(
                        &e.to_string(),
                        state,
                        session_id,
                        chat,
                        thread_id,
                        &rich_sig,
                        mid,
                    )
                    .await;
                    match outcome {
                        EditOutcome::Saved | EditOutcome::Suppressed => return,
                        EditOutcome::Gone => { /* fall through to create */ }
                    }
                }
            }
        }
        // No live card or edit failed: create fresh via rich API.
        // G3 send pacing (#1211): a fresh card is a full message post.
        super::governor::pace_send(chat).await;
        match super::rich::api::send_rich_html_id(
            bot.api_url().as_str(),
            bot.token(),
            chat.0,
            thread_id,
            &rich_html,
            kb_val.as_ref(),
            "turn",
            "-",
        )
        .await
        {
            Ok(mid) => {
                state
                    .set_plan_card(session_id, chat, thread_id, MessageId(mid), rich_sig)
                    .await;
                return;
            }
            Err(e) => {
                tracing::warn!("Rich plan card create failed: {e} — falling back to HTML");
            }
        }
    }

    // Classic HTML path (sendMessage, 4096 chars, <blockquote expandable>).
    let Some(html) = render_plan_card_html(
        title.as_deref(),
        checklist.as_deref(),
        prose.as_deref(),
        goal.as_ref(),
    )
    .await
    else {
        // No live plan. If THIS settle just archived the plan, the completion
        // arrived on a path that didn't run the handler settle gate (a
        // flood-delayed settle via resume.rs/stream_loop drives refresh_plan_card
        // directly). Finalize the completed card and re-stick it to the bottom
        // instead of silently deleting it (#1231). Otherwise it's a genuine
        // plan-gone/discard removal.
        if crate::utils::plan_files::peek_plan_just_archived(session_id).await {
            // Finalize consumes the flag itself once the notice lands (#16);
            // a failed finalize leaves it in place for the next settle's retry.
            finalize_plan_card_locked(bot, chat, thread_id, state, session_id).await;
        } else {
            remove_plan_card_locked(bot, chat, state, session_id).await;
        }
        return;
    };
    let kb = plan_kb.keyboard();
    let signature = format!("{html}\u{1}{plan_kb:?}");

    if let Some((mid, last_sig)) = state.plan_card(session_id).await {
        if last_sig == signature {
            return;
        }
        // G2 flood governor (#1211): same FINAL contract as the rich path —
        // queue latest-wins when the edit bucket is empty, never drop.
        let admitted = super::governor::edit_admission(
            bot,
            chat,
            mid,
            super::governor::EditClass::Final,
            html.clone(),
            false,
        )
        .await;
        if !admitted {
            state
                .set_plan_card(session_id, chat, thread_id, mid, signature)
                .await;
            return;
        }
        let mut req = bot
            .edit_message_text(chat, mid, html.clone())
            .parse_mode(ParseMode::Html);
        if let Some(ref k) = kb {
            req = req.reply_markup(k.clone());
        }
        match req.await {
            Ok(_) => {
                state
                    .set_plan_card(session_id, chat, thread_id, mid, signature)
                    .await;
                return;
            }
            Err(e) => {
                let outcome = handle_edit_failure(
                    &e.to_string(),
                    state,
                    session_id,
                    chat,
                    thread_id,
                    &signature,
                    mid,
                )
                .await;
                match outcome {
                    EditOutcome::Saved | EditOutcome::Suppressed => return,
                    EditOutcome::Gone => { /* fall through to create */ }
                }
            }
        }
    }

    // No live card (or it was unusable): post a fresh one at the bottom.
    // G3 send pacing (#1211): a fresh card is a full message post.
    super::governor::pace_send(chat).await;
    let mut req = message_in_thread(bot, chat, thread_id, html).parse_mode(ParseMode::Html);
    if let Some(ref k) = kb {
        req = req.reply_markup(k.clone());
    }
    match req.await {
        Ok(m) => {
            state
                .set_plan_card(session_id, chat, thread_id, m.id, signature)
                .await
        }
        Err(e) => {
            handle_create_failure(&e.to_string(), state, session_id).await;
        }
    }
}

/// Delete the session's plan card and stop tracking it. Used both as terminal
/// removal (discard / plan gone) and — followed by a later [`refresh_plan_card`]
/// — as a re-stick so the next card posts fresh at the bottom of the
/// conversation, keeping exactly one card visible as it follows the turns down.
/// Completed-plan finalization (#1158, #1231): when a turn settles right after
/// the session's plan was archived, turn the card into its completed form and
/// RE-STICK IT TO THE BOTTOM of the thread: ✅ header, final checklist, keyboard
/// stripped (explicit EMPTY `inline_keyboard`; omitting `reply_markup` leaves
/// stale buttons attached per Bot API semantics), footer noting the archive.
/// The buried tracked card is deleted and a fresh completed card is posted at
/// the conversation's current position, so the finished plan lands where the
/// user is reading, not far up in history.
///
/// Flood-safe: the fresh card is posted FIRST (paced through G3); only if the
/// post fails do we fall back to editing the tracked card in place, so the
/// completed form is never lost on a flood-controlled settle. A successful
/// post then deletes the old card best-effort — a delete failure leaves the
/// stale card visible, never a duplicate (the new one is what users see).
///
/// One-shot (#809 lesson): success UNTRACKS the card, so no later refresh or
/// restart ever re-renders an archived plan as live. The "just archived THIS
/// settle" stamp is a durable flag; tool_loop archives at EVERY settling
/// plan-turn, so without that stamp a later settle would wrongly finalize an
/// unrelated archive forever.
///
/// Outcome-gated consumption (#16): finalize itself consumes the flag — but
/// only AFTER the completed card's post/edit is confirmed landed (or is
/// terminally impossible). A failed finalize leaves the flag in place, so
/// the NEXT settle retries instead of losing the completion notice forever.
/// Pre-#16 the gate site consumed the flag BEFORE calling finalize; when a
/// G3 pacing hold aborted the settle mid-await, finalize died before any API
/// call and any failure branch — zero logs, flag gone, notice lost. Returns
/// `true` when the notice landed or was terminally settled, `false` when the
/// flag was kept for retry.
pub(crate) async fn finalize_plan_card(
    bot: &Bot,
    chat: ChatId,
    thread_id: Option<ThreadId>,
    state: &Arc<TelegramState>,
    session_id: Uuid,
) -> bool {
    // Same lock discipline as refresh/remove (#822): held across API calls.
    let card_lock = state.plan_card_lock(session_id).await;
    let _guard = card_lock.lock().await;
    finalize_plan_card_locked(bot, chat, thread_id, state, session_id).await
}

/// Drop-guard WARN for a mid-flight finalize abort (#16): when the settle
/// task is cancelled while finalize awaits (the G3 pacing hold never
/// resolving before the settle context drops was the 2026-08-28 incident),
/// the future is dropped and NO code below the await runs — the abort was
/// forensic-silent. Drop is the one hook that still fires; a normal exit
/// disarms the guard first.
struct FinalizeAbortGuard {
    session_id: Uuid,
    armed: bool,
}

impl Drop for FinalizeAbortGuard {
    fn drop(&mut self) {
        if self.armed {
            tracing::warn!(
                "Telegram plan card finalize aborted mid-flight for session {} \
                 (task cancelled while awaiting — e.g. unresolved G3 pacing hold); \
                 just-archived flag retained, next settle retries",
                self.session_id
            );
        }
    }
}

/// Finalization body, for callers already holding the per-session card lock.
///
/// `refresh_plan_card` runs the same lock and, on its no-live-plan path (the
/// plan was just archived there), finalizes instead of deleting — calling the
/// lock-taking `finalize_plan_card` from inside that locked scope would
/// deadlock (the lock is not reentrant; same split as
/// `remove_plan_card_locked`).
async fn finalize_plan_card_locked(
    bot: &Bot,
    chat: ChatId,
    thread_id: Option<ThreadId>,
    state: &Arc<TelegramState>,
    session_id: Uuid,
) -> bool {
    let Some((mid, _sig)) = state.plan_card(session_id).await else {
        // Nothing tracked: finalized once already, or never posted (card
        // tracking is in-memory — a restart empties it). Either way
        // deliberately NOT reposting is what kills resurrection. Consume
        // the flag (#16) so a stale stamp cannot gate every later settle.
        // Logged (#16 round 2): this consume drops the completion notice
        // permanently — it must never be forensic-silent again.
        tracing::warn!(
            "Telegram plan card finalize for session {session_id}: no tracked \
             card (already finalized or never posted) — consuming just-archived \
             flag, completion notice NOT posted"
        );
        crate::utils::plan_files::take_plan_just_archived(session_id).await;
        return true;
    };
    let Some(doc) = crate::utils::plan_files::latest_archived_plan(session_id).await else {
        // No archived document to render from: terminal, consume (#16).
        // Logged (#16 round 2): the stem-drift bug routed EVERY finalize
        // through this branch (reader prefix never matched the dot-less
        // archive names) — it must never be forensic-silent again.
        tracing::warn!(
            "Telegram plan card finalize for session {session_id}: no archived \
             plan document found — consuming just-archived flag, completion \
             notice NOT posted"
        );
        crate::utils::plan_files::take_plan_just_archived(session_id).await;
        return true;
    };
    // #16: armed across every await below; a normal exit disarms it. If the
    // enclosing task is cancelled mid-await instead, its Drop logs the abort
    // the old code never could.
    let mut abort_guard = FinalizeAbortGuard {
        session_id,
        armed: true,
    };
    let (title, checklist) = super::flow_chrome::plan_document_sections(&doc);
    let empty_kb = serde_json::json!({ "inline_keyboard": [] });

    let use_rich = Config::current().channels.telegram.rich_messages;

    // Completed forms, mirrored dual-path as in refresh_plan_card.
    let rich = if use_rich {
        render_plan_card_rich_html(title.as_deref(), checklist.as_deref(), None, None)
            .await
            .map(|mut r| {
                r = r.replacen("📋", "✅", 1);
                r.push_str("\n<i>Plan completed and archived.</i>");
                r
            })
    } else {
        None
    };
    let mut html = render_plan_card_html(title.as_deref(), checklist.as_deref(), None, None)
        .await
        .unwrap_or_else(|| "<b>Plan</b>".to_string());
    html = html.replacen("📋", "✅", 1);
    html.push_str("\n<i>Plan completed and archived.</i>");

    // Restick-to-bottom (#1231): post the completed card fresh at the bottom
    // FIRST, so a post failure can fall back to the card already present — the
    // completed form is never lost.
    let mut posted: Option<MessageId> = None;
    if use_rich && let Some(rich) = &rich {
        // G3 send pacing (#1211): a fresh card is a full message post.
        super::governor::pace_send(chat).await;
        match super::rich::api::send_rich_html_id(
            bot.api_url().as_str(),
            bot.token(),
            chat.0,
            thread_id,
            rich,
            Some(&empty_kb),
            "turn",
            "-",
        )
        .await
        {
            Ok(mid) => posted = Some(MessageId(mid)),
            Err(e) => tracing::warn!("Telegram plan card rich restick failed: {e}"),
        }
    }
    if posted.is_none() {
        // G3 send pacing (#1211): a fresh card is a full message post.
        super::governor::pace_send(chat).await;
        let req = message_in_thread(bot, chat, thread_id, html.clone()).parse_mode(ParseMode::Html);
        match req.await {
            Ok(m) => posted = Some(m.id),
            Err(e) => tracing::warn!("Telegram plan card restick post failed ({mid:?}): {e}"),
        }
    }

    match posted {
        Some(new_mid) => {
            // The fresh completed card is at the bottom: the completion
            // notice has LANDED — consume the flag now, not before (#16).
            crate::utils::plan_files::take_plan_just_archived(session_id).await;
            tracing::info!(
                "Telegram plan card finalized for session {session_id}: \
                 completed card posted ({new_mid:?})"
            );
            // Delete the buried tracked card best-effort; a delete failure
            // leaves a stale card visible, never a duplicate. Both outcomes
            // are logged — a vanished card must stay forensic (#16).
            if let Some((mid, _)) = state.plan_card(session_id).await
                && new_mid != mid
            {
                match bot.delete_message(chat, mid).await {
                    Ok(_) => {
                        tracing::info!("Telegram plan card restick deleted stale card ({mid:?})")
                    }
                    Err(e) => {
                        tracing::warn!("Telegram plan card restick delete failed ({mid:?}): {e}")
                    }
                }
            }
            state.take_plan_card(session_id).await;
            abort_guard.armed = false;
            true
        }
        None => {
            // Post failed (flood/API): fall back to editing the tracked card
            // in place so the completed form is still shown.
            let Some((mid, _)) = state.plan_card(session_id).await else {
                // Tracked card vanished mid-finalize: nothing left to edit,
                // and reposting from here would resurrect a deliberately
                // removed card. Terminal — consume the flag (#16). Logged
                // (#16 round 2): a vanished card plus a dropped notice must
                // stay forensic.
                tracing::warn!(
                    "Telegram plan card finalize for session {session_id}: \
                     tracked card vanished mid-finalize — consuming \
                     just-archived flag, completion notice NOT posted"
                );
                crate::utils::plan_files::take_plan_just_archived(session_id).await;
                abort_guard.armed = false;
                return true;
            };
            let mut edited = false;
            if use_rich && let Some(rich) = &rich {
                match super::rich::api::edit_rich_html(
                    bot.api_url().as_str(),
                    bot.token(),
                    chat.0,
                    mid.0,
                    rich,
                    Some(&empty_kb),
                    "turn",
                    "-",
                )
                .await
                {
                    Ok(()) => edited = true,
                    Err(e) => tracing::warn!(
                        "Telegram plan card rich finalize edit failed ({mid:?}): {e}"
                    ),
                }
            }
            if !edited {
                match bot
                    .edit_message_text(chat, mid, html.clone())
                    .parse_mode(teloxide::types::ParseMode::Html)
                    .reply_markup(super::suggest_options::empty_keyboard())
                    .await
                {
                    Ok(_) => edited = true,
                    Err(e) => {
                        tracing::warn!("Telegram plan card finalize edit failed ({mid:?}): {e}")
                    }
                }
            }
            abort_guard.armed = false;
            if edited {
                // In-place edit LANDED: the completion notice is visible —
                // consume the flag and untrack (#16).
                crate::utils::plan_files::take_plan_just_archived(session_id).await;
                tracing::info!(
                    "Telegram plan card finalized in place ({mid:?}) for session \
                     {session_id} after post failure"
                );
                state.take_plan_card(session_id).await;
                true
            } else {
                // Post AND edit failed: keep the flag so the NEXT settle
                // retries finalize (#16). The stamp is durable; this WARN is
                // the visible retry trail until one lands.
                tracing::warn!(
                    "Telegram plan card finalize FAILED for session {session_id} \
                     (post + edit both failed) — just-archived flag retained, \
                     next settle retries"
                );
                false
            }
        }
    }
}

pub(crate) async fn remove_plan_card(
    bot: &Bot,
    chat: ChatId,
    state: &Arc<TelegramState>,
    session_id: Uuid,
) {
    // Same lock as refresh (#822). Removal clears tracking, so a refresh
    // interleaving here is guaranteed to see no card and post one, which is
    // the widest form of the race.
    //
    // Callers that ALREADY hold the lock must use remove_plan_card_locked
    // instead: the lock is not reentrant, so re-acquiring it deadlocks.
    let card_lock = state.plan_card_lock(session_id).await;
    let _guard = card_lock.lock().await;
    remove_plan_card_locked(bot, chat, state, session_id).await;
}

/// Settle-path plan-card tail (#69): the #62 re-stick block shared by BOTH
/// delivery paths. The normal settle runs it after final delivery; the
/// interrupt/replacement teardown (handle_message's cancel_token guard) runs
/// the SAME tail, because a preempted turn's `return Ok(())` used to skip the
/// re-stick entirely — during rapid-fire discussion the card stayed buried at
/// a stale position for as long as the interrupt chain lasted (#69 live
/// observation: zero normal settles for 2+ hours, resticks only from
/// stream_loop flow-burial). Same gates as the normal path, unchanged:
/// finalize-on-archive first, then the tracked-card claim on the shared
/// sticky-stack budget (#1150) — a cardless settle spends nothing and retries
/// on the next settle — then the in-place refresh riding G2, fresh post
/// riding G3 (#1211). No cooldown: the #814 legacy stays deleted.
///
/// `plan_kb` is passed by the caller: the normal path forwards the value its
/// settle render just computed (bit-identical to the pre-#69 block); the
/// interrupt path recomputes it for a finished turn (`load_plan_state_section`
/// with `turn_active == false`, mirroring the settle path's `refresh_sections`)
/// so the Approve/Discard keyboard rides the resticked card.
pub(crate) async fn restick_plan_card_after_turn(
    bot: &Bot,
    chat: ChatId,
    thread_id: Option<ThreadId>,
    state: &Arc<TelegramState>,
    agent: &AgentService,
    session_id: Uuid,
    plan_kb: PlanKb,
) {
    if crate::utils::plan_files::peek_plan_just_archived(session_id).await {
        // Plan completed THIS settle (#1158, #1231): finalize the tracked
        // card (✅ header, keyboard stripped, one-shot untrack) and re-stick
        // the completed card to the bottom of the thread instead of
        // re-sticking or refreshing a now-archived plan. Finalize consumes
        // the flag only after the notice LANDED (#16): a flood-aborted
        // finalize leaves it in place, so the next settle retries instead
        // of losing the completion notice forever.
        finalize_plan_card(bot, chat, thread_id, state, session_id).await;
    } else {
        // Gate the claim on a tracked card (in-memory only — cheap): a
        // cardless settle must not spend the sticky budget, or the
        // flow-block restick starves for 15s after EVERY settle (#62).
        if state.plan_card_cached(session_id).await.is_some()
            && state.claim_sticky_action(chat.0, TelegramState::STICKY_STACK_MIN_INTERVAL)
        {
            remove_plan_card(bot, chat, state, session_id).await;
        }
        refresh_plan_card(bot, chat, thread_id, state, agent, session_id, plan_kb).await;
    }
}

/// Removal body, for callers already holding the per-session card lock.
///
/// Split out because `refresh_plan_card` takes the lock and then needs to
/// remove on its no-content path. Calling the lock-taking version there
/// deadlocked the Telegram handler outright.
async fn remove_plan_card_locked(
    bot: &Bot,
    chat: ChatId,
    state: &Arc<TelegramState>,
    session_id: Uuid,
) {
    if let Some(mid) = state.take_plan_card(session_id).await {
        // #16: deleteMessage success used to be silent — a vanished card was
        // undetectable without cross-referencing a user report. Log both
        // outcomes so a removal is always forensic.
        match bot.delete_message(chat, mid).await {
            Ok(_) => {
                tracing::info!("Telegram plan card deleted ({mid:?}) for session {session_id}")
            }
            Err(e) => tracing::warn!("Telegram plan card delete failed ({mid:?}): {e}"),
        }
    }
}
