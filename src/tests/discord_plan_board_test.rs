//! The Discord plan card (#1912, Option A): what it paints, how often, and
//! which chat it is allowed to touch.
//!
//! Option A is the full persistent card, and the two things that make it safe
//! to share a channel with the 4 s flow bubble are the signature dedupe (an
//! unchanged checklist must cost ZERO API calls) and the governor's surface
//! classes (#1910: a live repaint is droppable chrome, a completing paint is
//! content that holds). These tests pin exactly that behaviour, plus the
//! Discord-specific rendering: markdown escaping instead of Telegram's HTML,
//! the 2000 code point cap, and the rule that a row belonging to another chat
//! is never edited or overwritten from here.
//!
//! Chat, message and session ids are synthetic and carry no user identifiers.

use std::sync::Arc;

use crate::channels::discord::DiscordState;
use crate::channels::discord::governor::{Governor, Surface, test_support};
use crate::channels::discord::plan_card::{
    PaintTarget, card_is_ours, decide_paint, escape_markdown, render_plan_card, should_paint,
    status_word, surface_for, untracks_the_card,
};
use crate::config::Config;
use crate::db::Database;
use crate::db::repository::{PlanCard, PlanCardRepository};
use crate::tui::plan::{PlanDocument, PlanStatus, PlanTask};

/// Install a config mirror with the governor knobs this suite expects, run
/// inside the shared clock guard, and hand back the previous mirror so the
/// test can put it back. Parsing `config.toml.example` is deliberate: it keeps
/// the shipped example loadable, which is the same seam the #1910 suite uses.
macro_rules! governor_config {
    ($($field:ident : $value:expr),* $(,)?) => {{
        let _guard = test_support::registry_guard().await;
        test_support::reset(0);
        let prev = Config::current();
        let mut cfg: Config = toml::from_str(include_str!("../../config.toml.example"))
            .expect("embedded config.toml.example must parse");
        $(cfg.channels.discord.governor.$field = $value;)*
        Config::set_current(cfg);
        (prev, _guard)
    }};
}

const SESSION: &str = "0d1b7c00-0000-4000-8000-000000000192";
const CHANNEL: u64 = 1_234_567_890_123_456;
const FOREIGN_CHANNEL: u64 = 9_876_543_210_987_654;

async fn repo() -> PlanCardRepository {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    PlanCardRepository::new(db.pool().clone())
}

fn task(title: &str, status: crate::tui::plan::TaskStatus) -> PlanTask {
    let mut t = PlanTask::new(
        1,
        title.to_string(),
        String::new(),
        crate::tui::plan::TaskType::Other(String::new()),
    );
    t.status = status;
    t
}

fn plan(title: &str, status: PlanStatus, tasks: Vec<PlanTask>) -> PlanDocument {
    PlanDocument {
        title: title.to_string(),
        status,
        tasks,
        ..PlanDocument::new(SESSION.parse().unwrap(), title.to_string())
    }
}

#[test]
fn a_task_title_cannot_ping_the_guild() {
    // The card body is model-authored, and Discord renders markdown and pings
    // in ordinary messages: `@everyone` in a checklist row would notify every
    // member of the guild. Telegram escapes the same content as HTML; here
    // each control character must come back prefixed with a backslash.
    let escaped = escape_markdown("deploy @everyone and @here <@1234567890> *bold* _under_");
    assert!(
        !escaped.contains("@everyone") || escaped.contains("\\@everyone"),
        "got {escaped}"
    );
    assert!(escaped.contains("\\@here"), "got {escaped}");
    assert!(escaped.contains("\\<\\@1234567890\\>"), "got {escaped}");
    assert!(escaped.contains("\\*bold\\*"), "got {escaped}");
    assert!(escaped.contains("\\_under\\_"), "got {escaped}");
}

#[test]
fn the_rendered_card_has_no_raw_markdown_or_tags() {
    let p = plan(
        "Ship <b>now</b> @everyone *immediately*",
        PlanStatus::Active,
        vec![task(
            "run `cargo test` <@999>",
            crate::tui::plan::TaskStatus::Pending,
        )],
    );
    let card = render_plan_card(&p);
    assert!(!card.contains("<b>"), "raw HTML survived: {card}");
    // `\@everyone` still CONTAINS the substring `@everyone`: the backslash
    // guards it, it does not remove it. So the raw-ping check looks only at
    // the occurrences that are not backslash-guarded.
    assert!(
        !card.replace("\\@", "").contains("@everyone"),
        "raw ping survived: {card}"
    );
    assert!(!card.contains("<@999>"), "raw mention survived: {card}");
    assert!(card.contains("\\@everyone"));
    assert!(card.contains("\\`cargo test\\`"));
}

#[test]
fn the_card_shows_status_marks_and_the_machine_badge() {
    // The marks come from the shared `status_mark` helper (#1154) and the
    // badge from #1133's machine-written verdict, so the Discord card and the
    // TUI widget can never drift apart.
    let mut verified = task("gate the change", crate::tui::plan::TaskStatus::InProgress);
    verified.verification = Some(crate::tui::plan::VerificationVerdict::Verified);
    let skipped = task("old lane", crate::tui::plan::TaskStatus::Skipped);
    let p = plan(
        "Batch",
        PlanStatus::Active,
        vec![
            task("done thing", crate::tui::plan::TaskStatus::Completed),
            verified,
            skipped,
        ],
    );
    let card = render_plan_card(&p);
    assert!(card.contains('☑'), "got {card}");
    assert!(card.contains('▶'), "got {card}");
    assert!(card.contains('⏭'), "got {card}");
    assert!(card.contains('🛡'), "badge missing: {card}");
    // "done" counts Completed only, the same convention as the TUI widget's
    // progress_percentage: a Skipped row shows ⏭ but is not a done row.
    assert!(card.contains("1/3 done"), "progress line missing: {card}");
    assert!(card.contains(status_word(&PlanStatus::Active)));
}

#[test]
fn a_long_checklist_is_clipped_not_sent_whole() {
    // Discord counts 2000 code points per message. A 200-task plan must not
    // produce a rejected send: rows past the cap are replaced by a tail that
    // says how many are missing.
    let tasks: Vec<PlanTask> = (0..200)
        .map(|i| {
            task(
                &format!("task {i}: {}", "w".repeat(120)),
                crate::tui::plan::TaskStatus::Pending,
            )
        })
        .collect();
    let card = render_plan_card(&plan("Huge", PlanStatus::Active, tasks));
    assert!(
        card.chars().count() < 2000,
        "card is {} code points",
        card.chars().count()
    );
    assert!(card.ends_with(" more"), "no tail: {card}");
    assert!(card.contains("… +"), "tail has no count: {card}");
}

#[test]
fn an_unchanged_card_is_never_repainted() {
    // The dedupe that makes sharing the ticker's 4 s cadence safe: the same
    // signature means zero API calls.
    assert!(!should_paint(Some("☑ a\n☐ b"), "☑ a\n☐ b"));
    assert!(should_paint(Some("☑ a\n☐ b"), "☑ a\n▶ b"));
    assert!(
        should_paint(None, "☑ a"),
        "a first paint must not be silenced by the dedupe"
    );
}

#[test]
fn a_live_repaint_is_chrome_and_a_completing_paint_is_final() {
    // #1910's classification, stated once for the whole module: the ticker's
    // paint is restated by the next tick, so it may be dropped; the finalizing
    // paint is the card's last word, so it must hold.
    assert!(!surface_for(false).is_content());
    assert_eq!(surface_for(true), Surface::Final);
    assert!(surface_for(true).is_content());
}

#[test]
fn an_absent_plan_is_never_resurrected() {
    // #809's zombie rule carried over verbatim: "no live plan" stays true once
    // a plan completes, so only the settle that archived it may paint the
    // archive. Every other refresh must do nothing.
    let live = plan("Live", PlanStatus::Active, vec![]);
    let archived = plan("Gone", PlanStatus::Editing, vec![]);
    let got = decide_paint(Some(live.clone()), false, None);
    assert!(
        matches!(got, PaintTarget::Live(ref p) if p.title == "Live"),
        "{got:?}"
    );
    let got = decide_paint(None, true, Some(archived.clone()));
    assert!(
        matches!(got, PaintTarget::Finalizing(ref p) if p.title == "Gone"),
        "{got:?}"
    );
    // A plan that is merely absent must not be read out of the archive, even
    // when one is sitting there: the archive is only for the settle that
    // produced it.
    let got = decide_paint(None, false, Some(live.clone()));
    assert!(matches!(got, PaintTarget::Skip), "{got:?}");
    let got = decide_paint(None, false, None);
    assert!(matches!(got, PaintTarget::Skip), "{got:?}");
}

#[test]
fn only_a_gone_card_releases_its_tracker() {
    // Untracking on a transient error would post a SECOND card and strand the
    // first uneditable (#822). Keeping a tracker on a deleted message would
    // edit nothing forever. So only "that message is gone" releases it.
    assert!(untracks_the_card("HTTP 404: Unknown Message"));
    assert!(untracks_the_card("error 10008: Unknown Message"));
    assert!(untracks_the_card("403: Missing Permissions"));
    assert!(!untracks_the_card("429: You are being rate limited."));
    assert!(!untracks_the_card("connection reset by peer"));
}

#[test]
fn a_row_belonging_to_another_chat_is_not_ours() {
    assert!(card_is_ours(CHANNEL as i64, CHANNEL));
    assert!(!card_is_ours(FOREIGN_CHANNEL as i64, CHANNEL));
    // Telegram rows are keyed by i64 chat ids that can be negative
    // (supergroups); the cast must not make one look like a Discord channel.
    assert!(!card_is_ours(-1_004_428_873_948, CHANNEL));
}

#[tokio::test]
async fn a_card_survives_a_restart_and_is_rehydrated_from_the_row() {
    // The tracker map is process-local; the row is what outlives a restart. A
    // fresh state must find the same message id instead of posting a second
    // card beside the survivor.
    let repo = repo().await;
    repo.set(PlanCard {
        session_id: SESSION.to_string(),
        chat_id: CHANNEL as i64,
        thread_id: None,
        message_id: 555,
        signature: "☑ a\n☐ b".to_string(),
    })
    .await
    .unwrap();

    let state = DiscordState::new();
    let session: uuid::Uuid = SESSION.parse().unwrap();
    let hit = state
        .plan_card(session, CHANNEL, Some(&repo))
        .await
        .expect("the persisted row must rehydrate the card");
    assert_eq!(hit.0, 555);
    assert_eq!(hit.1, "☑ a\n☐ b");

    // The same row must never be edited from a channel that does not own it.
    assert!(
        state
            .plan_card(session, FOREIGN_CHANNEL, Some(&repo))
            .await
            .is_none(),
        "a foreign chat would overwrite another platform's card"
    );
}

#[tokio::test]
async fn a_painted_card_persists_its_signature_for_the_next_process() {
    let repo = repo().await;
    let state = DiscordState::new();
    let session: uuid::Uuid = SESSION.parse().unwrap();
    state
        .remember_plan_card(
            session,
            CHANNEL,
            777,
            "☑ a".to_string(),
            PlanCard {
                session_id: SESSION.to_string(),
                chat_id: CHANNEL as i64,
                thread_id: None,
                message_id: 777,
                signature: "☑ a".to_string(),
            },
            Some(&repo),
        )
        .await;

    // In memory now, and on disk for the next process: a restart with the same
    // rendered text must still dedupe.
    let row = repo.get(SESSION).await.unwrap().expect("row written");
    assert_eq!(row.message_id, 777);
    assert_eq!(row.signature, "☑ a");

    let fresh = DiscordState::new();
    let (mid, sig) = fresh
        .plan_card(session, CHANNEL, Some(&repo))
        .await
        .expect("rehydrated");
    assert_eq!(mid, 777);
    assert!(!should_paint(Some(&sig), "☑ a"), "restart must dedupe");
}

#[tokio::test]
async fn a_burnt_bucket_refuses_the_repaint_instead_of_queueing_behind_it() {
    // The card shares the channel's ONE budget with the flow bubble (#1912's
    // constraint). A repaint that cannot be afforded must be refused cheaply:
    // if it queued, the card would sit in front of the bubble's next tick and
    // both surfaces would drift. The bucket, not the spacing floor, is what
    // refuses here, so the floor is switched off.
    let (prev, _guard) = governor_config!(
        writes_per_minute: 3u32,
        burst: 3u32,
        chrome_min_spacing_ms: 0u64,
    );
    let governor = Governor::default();
    for i in 1..=3 {
        assert!(
            governor.chrome_admits(CHANNEL),
            "write {i} of a burst of 3 must be admitted"
        );
    }
    assert!(
        !governor.chrome_admits(CHANNEL),
        "the fourth paint on a burnt bucket must be refused, not queued"
    );
    if let Ok(cfg) = Arc::try_unwrap(prev) {
        Config::set_current(cfg);
    }
}
