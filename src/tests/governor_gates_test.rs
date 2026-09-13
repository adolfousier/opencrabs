//! Integration suite for the proactive flood governors (#1211).
//!
//! Per the #1211 test brief this drives the REAL gates end-to-end with every
//! slow thing mocked, nothing slept for real:
//!
//! - **Mocked Telegram server** — `mockito` fakes the Bot API; the queued-
//!   finals drainer must land its `editMessageText` over HTTP against it,
//!   proving the wire shape and delivery accounting, not just bookkeeping.
//! - **Mocked agent sessions** — concurrent fake turn tasks hammer
//!   [`admit_chat_action`] for one forum topic, reproducing the multi-session
//!   split-budget storm that motivated #1211.
//! - **Mocked passage of time** — every gate reads the injectable `gate_now`
//!   seam; tests advance the virtual clock by hand (`ts::advance`) instead of
//!   sleeping, and run under tokio's paused runtime so even the internal
//!   backoff sleeps cost zero wall time.
//!
//! The reactive backstop (`rate_limit::wait_out`) is intentionally NOT
//! exercised here — these tests assert governors NEVER let a call through
//! that would need it.

use std::time::Duration;

use teloxide::Bot;
use teloxide::types::{ChatId, MessageId};

use crate::channels::telegram::governor;
use crate::channels::telegram::governor::test_support as ts;
use crate::config::Config;

/// Replace the process-wide config mirror with `config.toml.example` plus
/// per-test `rate_limiter` mutations. Parsed fresh per test so knob changes
/// cannot leak sideways; the [`ts::registry_guard`] serializes the swap.
macro_rules! rl_config {
    ($($field:ident : $value:expr),* $(,)?) => {{
        let mut cfg: Config = toml::from_str(include_str!("../../config.toml.example"))
            .expect("embedded config.toml.example must parse");
        $(cfg.channels.telegram.rate_limiter.$field = $value;)*
        Config::set_current(cfg);
    }};
}

#[tokio::test(start_paused = true)]
async fn dm_chat_ids_bypass_all_three_governors() {
    let _guard = ts::registry_guard().await;
    ts::reset(1_000);
    rl_config!(enabled: true);

    // Positive chat ids are DMs: every gate admits instantly and must NOT
    // create peer state, whatever the volumes look like.
    for _ in 0..25 {
        assert!(governor::admit_chat_action(ChatId(777), None).await);
        assert!(governor::admit_chat_action(ChatId(777), Some(42)).await);
        let bot = Bot::new("TESTTOKEN");
        assert!(
            governor::edit_admission(
                &bot,
                ChatId(777),
                MessageId(1),
                governor::EditClass::Final,
                "<b>x</b>".into(),
                false,
            )
            .await
        );
    }
    governor::pace_send(ChatId(777)).await;

    assert!(
        ts::snapshot(ChatId(777)).is_none(),
        "a DM must never be registered as a governed peer"
    );
}

#[tokio::test(start_paused = true)]
async fn negative_peer_stays_ungoverned_until_a_topic_is_seen() {
    let _guard = ts::registry_guard().await;
    ts::reset(2_000);
    rl_config!(
        enabled: true,
        typing_burst: 2,
        typing_max_hold_secs: 0,
    );

    // A group chat reached WITHOUT any topic id passes ungoverned — the
    // forums-only rollout contract. Burst of 2 would normally bite, but no
    // governance engages pre-topic.
    assert!(governor::admit_chat_action(ChatId(-700), None).await);
    assert!(governor::admit_chat_action(ChatId(-700), None).await);
    assert!(governor::admit_chat_action(ChatId(-700), None).await);
    let snap = ts::snapshot(ChatId(-700)).expect("gate observed the chat");
    assert!(!snap.forum_seen, "topic-less traffic must not arm the peer");
    assert_eq!(snap.typing_admitted, 0, "ungoverned passes are not counted");
    assert_eq!(snap.typing_dropped, 0);

    // First topic-scoped call arms the peer; the (fresh, capacity-2) bucket
    // starts admitting, then drops with hold=0 once spent.
    assert!(governor::admit_chat_action(ChatId(-700), Some(9)).await);
    assert!(governor::admit_chat_action(ChatId(-700), Some(9)).await);
    assert!(!governor::admit_chat_action(ChatId(-700), Some(9)).await);
    let snap = ts::snapshot(ChatId(-700)).unwrap();
    assert!(snap.forum_seen);
    assert_eq!(snap.typing_admitted, 2);
    assert_eq!(snap.typing_dropped, 1);
}

#[tokio::test(start_paused = true)]
async fn simultaneous_agent_sessions_share_one_typing_budget() {
    let _guard = ts::registry_guard().await;
    ts::reset(3_000);
    // The storm shape from the Aug-25 logs: N concurrent turns in ONE forum,
    // each firing its own indicator refresh into the single per-peer budget.
    // hold=0 makes overflow DROP deterministically instead of pacing.
    rl_config!(
        enabled: true,
        typing_burst: 8,
        typing_max_hold_secs: 0,
    );

    const CHAT: ChatId = ChatId(-100_111);
    const TOPIC: i32 = 30679;

    // Spend the burst sequentially, like eight turns starting one after
    // another.
    for _ in 0..8 {
        assert!(governor::admit_chat_action(CHAT, Some(TOPIC)).await);
    }

    // Now twelve MORE sessions light up simultaneously (mocked agents) and
    // all hammer the same per-peer bucket. Every one of them must be refused
    // — none may slip through to re-amplify a 429 retry storm.
    let tasks: Vec<_> = (0..12)
        .map(|_| tokio::spawn(governor::admit_chat_action(CHAT, Some(TOPIC))))
        .collect();
    let mut refused = 0;
    for t in tasks {
        assert!(
            !t.await.expect("typing gate task panicked"),
            "an over-budget typing refresh leaked past G1"
        );
        refused += 1;
    }

    let snap = ts::snapshot(CHAT).unwrap();
    assert_eq!(refused, 12);
    assert_eq!(snap.typing_dropped, 12, "every refusal must be counted");
    assert_eq!(snap.typing_admitted, 8);

    // Virtual refill: one token after `typing_min_interval_secs` (3s default)
    // lets exactly one more refresh through — throttle semantics, not off.
    ts::advance(3_100);
    assert!(governor::admit_chat_action(CHAT, Some(TOPIC)).await);
}

#[tokio::test(start_paused = true)]
async fn edit_ladder_drops_in_priority_order_and_queues_latest_wins_finals() {
    let _guard = ts::registry_guard().await;
    ts::reset(4_000);
    rl_config!(
        enabled: true,
        edits_per_minute: 60, // 1 token/s steady state
        edit_burst: 2,
    );

    const CHAT: ChatId = ChatId(-100_222);
    let bot = Bot::new("TESTTOKEN");

    ts::mark_forum(CHAT);
    ts::burn_bucket(CHAT, ts::BucketKind::Edits, 2, 1.0);

    // Empty bucket: the four chrome classes drop IN LADDER ORDER — here
    // exercised in call sequence, with each drop counted under its own name
    // so a reordered enum would fail this ledger immediately.
    for class in [
        governor::EditClass::Clock,
        governor::EditClass::BrainPreview,
        governor::EditClass::Intermediary,
        governor::EditClass::Status,
    ] {
        assert!(
            !governor::edit_admission(&bot, CHAT, MessageId(5), class, "h".into(), false).await,
            "{class:?} must DROP on an empty bucket"
        );
    }
    let snap = ts::snapshot(CHAT).unwrap();
    assert_eq!(snap.edits_admitted, 0);
    assert_eq!(snap.dropped_clock, 1);
    assert_eq!(snap.dropped_brain_preview, 1);
    assert_eq!(snap.dropped_intermediary, 1);
    assert_eq!(snap.dropped_status, 1);

    // Finals NEVER drop: they queue latest-wins per message id. Two enqueues
    // for the same message collapse into ONE pending payload.
    assert!(
        !governor::edit_admission(
            &bot,
            CHAT,
            MessageId(9),
            governor::EditClass::Final,
            "<b>v1</b>".into(),
            false,
        )
        .await
    );
    assert!(
        !governor::edit_admission(
            &bot,
            CHAT,
            MessageId(9),
            governor::EditClass::Final,
            "<b>v2</b>".into(),
            false,
        )
        .await
    );
    let snap = ts::snapshot(CHAT).unwrap();
    assert_eq!(snap.queued_finals, 2, "both enqueues are counted");
    assert_eq!(snap.superseded_finals, 1, "v1 was replaced by v2");
    assert_eq!(snap.finals_pending, 1, "latest-wins keeps a single payload");

    // Refill admits chrome edits again — drops self-heal on the next render.
    ts::advance(1_100);
    assert!(
        governor::edit_admission(
            &bot,
            CHAT,
            MessageId(5),
            governor::EditClass::Status,
            "h".into(),
            false,
        )
        .await
    );
}

/// Install (once per process) a warn-level tracing subscriber writing to
/// stderr, so the governor drainer's abandonment `warn!` — which carries the
/// underlying wire error string — lands in the failing test's captured
/// output instead of vanishing (the default test binary installs no
/// subscriber). The drainer runs as a spawned task on this test's own
/// current-thread runtime, so libtest attaches its stderr to THIS test.
/// Without this a red drainer run leaves no receipt for why the wire attempt
/// failed (#28 mode 3).
fn ensure_tracing_capture() {
    use std::sync::OnceLock;
    static HOOK: OnceLock<()> = OnceLock::new();
    HOOK.get_or_init(|| {
        // Best effort: only the first caller in the process may install the
        // global default; a later Err means one is already in place.
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::WARN)
            .with_writer(std::io::stderr)
            .try_init();
    });
}

/// One full drainer round: fresh registry + virtual clock, fresh mock server
/// and bot, one final queued through the starved edits bucket, then the
/// spawned drainer settles it on the wire. Each round is an independent draw
/// of the paused-clock schedule lottery; the stress variant below runs six
/// draws so a rare wire race cannot hide behind a green run.
///
/// `label` names the round in every race-relevant failure message.
async fn drainer_wire_round(label: &str) {
    let _guard = ts::registry_guard().await;
    ts::reset(5_000);
    rl_config!(
        enabled: true,
        edits_per_minute: 60,
        edit_burst: 2,
    );

    const CHAT: ChatId = ChatId(-100_333);
    ts::mark_forum(CHAT);
    ts::burn_bucket(CHAT, ts::BucketKind::Edits, 2, 1.0);

    // Mocked Telegram server: the drainer's classic-HTML edit must arrive
    // carrying the LATEST payload.
    //
    // The method segment is PascalCase — teloxide 0.17 sends
    // `EditMessageText`, not the lowercase form the Bot API docs use. With
    // the lowercase path the mock never matched, so mockito answered with its
    // own unmatched response, teloxide failed to decode that as a `Message`,
    // and the drainer read it as a network error and re-queued forever.
    let mut server = mockito::Server::new_async().await;
    let delivered = server
        .mock("POST", "/botTESTTOKEN/EditMessageText")
        .match_body(mockito::Matcher::PartialJson(
            serde_json::json!({"text": "<b>settled</b>"}),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        // A full Message, not just an id: teloxide decodes editMessageText's
        // result into `Message`, and a bare {"message_id":11} fails to
        // deserialize. The drainer then saw a network error, re-queued, and
        // the mock's single expected hit never settled.
        .with_body(
            r#"{"ok":true,"result":{"message_id":11,"date":1756166400,"chat":{"id":-100333,"title":"gates","type":"supergroup"},"text":"settled"}}"#,
        )
        // Retry-budget tolerance (#28, mode 2): a transient Err on the FIRST
        // attempt — the wire hit counted, the 200 served, yet reqwest
        // surfaced an error under paused-clock skew — makes the drainer
        // requeue and retry, as designed. `.expect(1)` stopped the mock
        // matching after one hit (mockito's is_missing_hits gate), so every
        // retry got an unmatched 501 and the test died in a retry storm:
        // one wire hit, delivered_finals stuck at 0, twice red on 39857b16.
        // Serve the real response for the drainer's full retry budget
        // instead; exactly-once is enforced where it lives — the
        // delivered_finals counter asserted below.
        .expect_at_least(1)
        .expect_at_most(8) // governor's FINAL_MAX_ATTEMPTS
        .create_async()
        .await;

    // Not Bot::new: that applies teloxide's default_reqwest_settings(),
    // which includes a 17s request timeout built from tokio timers. Under
    // this test's paused clock the deadline sits on VIRTUAL time, so the
    // converge loop's advances can cancel an in-flight request that mockito
    // already counted -> the drainer sees Err, requeues, retries -> second
    // wire hit -> .expect(1) flakes (~50% of runs). A client with no
    // timeout leaves nothing for the virtual clock to race.
    // `reqwest_teloxide` is reqwest 0.12, the version teloxide-core takes. Our
    // own dependency is on 0.13, and `Bot::with_client` wants teloxide's type
    // (#1345, #1346).
    let bot = Bot::with_client(
        "TESTTOKEN",
        reqwest_teloxide::Client::builder().build().unwrap(),
    )
    .set_api_url(server.url().parse().unwrap());
    assert!(
        !governor::edit_admission(
            &bot,
            CHAT,
            MessageId(11),
            governor::EditClass::Final,
            "<b>settled</b>".into(),
            false,
        )
        .await,
        "starved bucket queues the final"
    );
    assert_eq!(ts::snapshot(CHAT).unwrap().finals_pending, 1);

    // Event-driven settle wait (#28 mode 4). `take_due_final` pops the final
    // BEFORE the wire call, so `finals_pending` hits 0 while
    // `delivered_finals` is still 0 — the old poll's exit condition
    // (`delivered==1 && pending==0`) was unreachable mid-flight. Its 400ms
    // sleep pump kept a virtual timer pending every iteration, so the
    // paused runtime auto-advanced instead of real-parking on the reactor;
    // the buffered mockito 200 never reached the drainer and the whole
    // budget burned out in virtual milliseconds (receipt: run 33262674165,
    // 0/0/0 at the exactly-once assert). Now `deliver_final` fires
    // `ts::notify_settled()` on every verdict exit, and this loop parks on
    // that notifier with NO tokio timer pending in this task — a real
    // reactor park, so the in-flight response lands in real time and the
    // drainer's verdict wakes the loop. The drainer's own DRAIN_TICK timer
    // keeps auto-advance ticking until the wire call is in flight, then
    // real time takes over. No real sleeping anywhere in the test itself.
    //
    // Watchdog: one real-clock thread per round. If the drainer is genuinely
    // wedged (no verdict, no notify), the thread fires after 30s and the
    // self-reporting panic below trips — the backstop the deleted 120s
    // virtual timeout used to be (with the sleep pump gone that timeout
    // would be the ONLY pending virtual timer and auto-advance to zero,
    // panicking every run).
    let watchdog = std::sync::Arc::new(tokio::sync::Notify::new());
    let armed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    {
        let watchdog = std::sync::Arc::clone(&watchdog);
        let armed = std::sync::Arc::clone(&armed);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(30));
            if armed.load(std::sync::atomic::Ordering::SeqCst) {
                watchdog.notify_one();
            }
        });
    }
    let wedged = loop {
        let settled = ts::snapshot(CHAT)
            .map(|s| s.finals_pending == 0 && (s.delivered_finals + s.failed_finals) >= 1)
            .unwrap_or(false);
        if settled {
            break false;
        }
        tokio::select! {
            _ = ts::settle_notifier().notified() => {
                // A drainer exit happened; re-read the counters next pass.
                // Keep advancing virtual time for refill / 429-wait
                // bookkeeping while drainer timers are still pending.
                ts::advance(600); // > 1/s refill AND > DRAIN_TICK (400ms)
            }
            _ = watchdog.notified() => break true,
        }
    };
    armed.store(false, std::sync::atomic::Ordering::SeqCst); // disarm
    if wedged {
        // Self-reporting failure (#28): the counters say WHICH half of
        // the drainer stalled — wire verdict vs bookkeeping.
        match ts::snapshot(CHAT) {
            Some(s) => panic!(
                "queued final never drained ({label}): delivered={} failed={} pending={}",
                s.delivered_finals, s.failed_finals, s.finals_pending
            ),
            None => panic!("queued final never drained: peer snapshot vanished"),
        }
    }

    delivered.assert(); // wire hits within the retry budget, body matched above
    let snap = ts::snapshot(CHAT).unwrap();
    // The exactly-once invariant lives in the governor's own counter: a
    // double-delivered final lands at 2, a lost delivery at 0. On red, the
    // counters (plus the captured drainer warn below) self-report WHICH half
    // of the pipeline stalled (#28).
    assert_eq!(
        snap.delivered_finals, 1,
        "one delivered final must be accounted exactly once ({label}): \
         delivered={} failed={} pending={}",
        snap.delivered_finals, snap.failed_finals, snap.finals_pending,
    );
    assert_eq!(
        snap.failed_finals, 0,
        "drainer abandoned within the retry budget ({label}): \
         delivered={} failed={} pending={}",
        snap.delivered_finals, snap.failed_finals, snap.finals_pending,
    );
    assert_eq!(snap.finals_pending, 0);
}

#[tokio::test(start_paused = true)]
async fn queued_final_drains_over_the_wire_through_mock_bot_api() {
    ensure_tracing_capture();
    drainer_wire_round("single").await;
}

#[tokio::test(start_paused = true)]
async fn queued_final_drains_over_the_wire_through_mock_bot_api_stress_6x() {
    ensure_tracing_capture();
    for round in 0..6 {
        drainer_wire_round(&format!("stress-{round}")).await;
    }
}

#[tokio::test(start_paused = true)]
async fn send_pacer_delays_then_fails_open_never_drops() {
    let _guard = ts::registry_guard().await;
    ts::reset(6_000);
    rl_config!(
        enabled: true,
        send_min_interval_millis: 50,
        sends_ceiling_per_minute: 2,
        sends_burst: 2,
    );

    const CHAT: ChatId = ChatId(-100_444);
    ts::mark_forum(CHAT);
    ts::burn_bucket(CHAT, ts::BucketKind::SendsSec, 2, 20.0);
    ts::burn_bucket(CHAT, ts::BucketKind::SendsMin, 2, 2.0 / 60.0);

    // Starved pacer: the next send WAITS for a token (throttled ms accrue in
    // VIRTUAL time — the wait is real logic, the clock is ours), and when the
    // minute-ceiling stays dry past SEND_MAX_HOLD the pacer FAILS OPEN —
    // the send proceeds and the reactive backstop owns the risk (#297:
    // delay-never-drop). Both outcomes observable below without a single
    // real sleep.
    let paced = async {
        governor::pace_send(CHAT).await; // waits ~1 spacing interval on virtual time...
        ts::advance(31_000); // ...then we push past SEND_MAX_HOLD (30s)
        governor::pace_send(CHAT).await; // minute-bucket still dry -> fail-open return
    };
    tokio::time::timeout(Duration::from_secs(300), paced)
        .await
        .expect("pacer wedged instead of failing open");

    let snap = ts::snapshot(CHAT).unwrap();
    assert!(
        snap.throttled_send_ms > 0,
        "held sends must be attributed to the throttle ledger"
    );
    // No drop counter exists for G3 BY DESIGN — the assertion is that the
    // calls above RETURNED (fail-open) rather than hanging or discarding.
}

// ---------------------------------------------------------------------------
// G4 — rich endpoint pacing (#1211 follow-up)
// ---------------------------------------------------------------------------

/// The rich endpoint is metered separately from typing / edits / sends, so
/// none of G1-G3 sees its traffic.
///
/// This is not hypothetical. On the maintainer's deployment, over the same
/// four days that produced 391 typing 429s on the reporter's box, typing 429s
/// were **zero** and the rich endpoint took 261-462 real 429s per day, each
/// carrying a 17-24 s `retry_after`. On 2026-08-25: 7,891 rich edits, 399
/// rich sends, 260 429s — roughly 87 minutes of stalled flow rendering. The
/// 429s cluster in the p99 minutes (40-46 calls) while the median minute runs
/// 18, which is what the 30/min default is sized against.
#[tokio::test(start_paused = true)]
async fn rich_calls_are_paced_once_the_bucket_empties() {
    let _guard = ts::registry_guard().await;
    ts::reset(1_000);
    rl_config!(enabled: true, rich_per_minute: 30, rich_burst: 4);

    const CHAT: ChatId = ChatId(-100_555);
    const TOPIC: i32 = 4242;

    // The burst passes without any hold at all: ordinary traffic is untouched.
    for _ in 0..4 {
        governor::pace_rich(CHAT, Some(TOPIC)).await;
    }
    let snap = ts::snapshot(CHAT).unwrap();
    assert_eq!(snap.admitted_rich, 4);
    assert_eq!(
        snap.throttled_rich_ms, 0,
        "nothing within the burst may be held"
    );

    // The next one has to wait for a refill — 30/min is one token every 2 s.
    ts::burn_bucket(CHAT, ts::BucketKind::Rich, 4, 0.5);
    ts::advance(2_100);
    governor::pace_rich(CHAT, Some(TOPIC)).await;
    assert_eq!(
        ts::snapshot(CHAT).unwrap().admitted_rich,
        5,
        "a refilled token must admit rather than drop — a rich call is content"
    );
}

#[tokio::test(start_paused = true)]
async fn rich_pacing_waits_for_refill_without_failing_open() {
    let _guard = ts::registry_guard().await;
    ts::reset(6_000);
    rl_config!(
        enabled: true,
        rich_per_minute: 1, // 1 token every 60 s
        rich_burst: 1,
    );

    const CHAT: ChatId = ChatId(-100_556);
    const TOPIC: i32 = 4243;

    // Burst token consumed immediately at t=0.
    governor::pace_rich(CHAT, Some(TOPIC)).await;
    let snap0 = ts::snapshot(CHAT).unwrap();
    assert_eq!(snap0.admitted_rich, 1);
    assert_eq!(snap0.throttled_rich_ms, 0);

    // Call pace_rich again: bucket is empty, next token is 60 s away.
    // Under the old code, at SEND_MAX_HOLD (30 s), waited + need > 30s triggered
    // FailOpen immediately without holding.
    // With fail-open eliminated (#176), it waits cleanly for the full 60 s refill,
    // admits the call, and attributes the full wait time to throttled_rich_ms.
    governor::pace_rich(CHAT, Some(TOPIC)).await;

    let snap = ts::snapshot(CHAT).unwrap();
    assert_eq!(snap.admitted_rich, 2, "refilled token must admit cleanly");
    assert!(
        snap.throttled_rich_ms >= 50_000,
        "rich pacer must wait past SEND_MAX_HOLD (30s) without failing open; throttled_rich_ms={}",
        snap.throttled_rich_ms
    );
}

#[tokio::test(start_paused = true)]
async fn rich_pacing_leaves_dms_and_non_forums_alone() {
    let _guard = ts::registry_guard().await;
    ts::reset(1_000);
    rl_config!(enabled: true, rich_burst: 1);

    // DM: positive id, never governed, no peer state created.
    for _ in 0..10 {
        governor::pace_rich(ChatId(4242), Some(7)).await;
    }
    assert!(
        ts::snapshot(ChatId(4242)).is_none(),
        "a DM must not even allocate peer state"
    );

    // Group that has never been seen carrying a topic: forums-only rollout.
    const PLAIN: ChatId = ChatId(-100_666);
    for _ in 0..10 {
        governor::pace_rich(PLAIN, None).await;
    }
    assert_eq!(
        ts::snapshot(PLAIN).map(|s| s.admitted_rich).unwrap_or(0),
        0,
        "a non-forum group is passed through ungoverned"
    );
}

#[tokio::test(start_paused = true)]
async fn a_disabled_limiter_governs_nothing() {
    let _guard = ts::registry_guard().await;
    ts::reset(1_000);
    rl_config!(enabled: false, rich_burst: 1, typing_burst: 1);

    const CHAT: ChatId = ChatId(-100_777);
    for _ in 0..10 {
        governor::pace_rich(CHAT, Some(9)).await;
        assert!(governor::admit_chat_action(CHAT, Some(9)).await);
    }
    // The master switch is what restores fully reactive behaviour, so it must
    // short-circuit before any bucket is touched.
    ts::burn_bucket(CHAT, ts::BucketKind::Typing, 1, 1.0);
    assert!(governor::admit_chat_action(CHAT, Some(9)).await);
}
