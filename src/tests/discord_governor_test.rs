//! Discord outbound-write governor (#1910).
//!
//! Two layers, tested separately, exactly like the Telegram suite:
//!
//! - **Pure admission math** - [`admit`] and [`note_429`] take an injected
//!   `Instant`, so spacing, holds, parks and the fail-open ceiling are pinned
//!   without tokio, a config mirror or a clock seam.
//! - **Gate wiring** - the real [`Governor`] under the virtual clock and a
//!   swapped config, asserting that chrome self-heals, a park refuses the
//!   ticker's next paint, and the window the API named beats the fallback.
//!
//! The behavior contract these pins exist to hold: content (an answer, a
//! settle stamp) is delay-never-drop, chrome (the 4 s flow ticker's clock, a
//! waiting-line refresh) is dropped the moment it is refused because the next
//! paint restates it, and a 429 parks the channel it happened on.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::channels::discord::governor::test_support as ts;
use crate::channels::discord::governor::{
    ChannelBudget, Decision, Governor, Limits, Surface, admit, note_429,
};
use crate::config::Config;

/// A governor-shaped set of knobs with the numbers the prose uses, so each
/// test only mutates the one field it is about.
fn limits() -> Limits {
    Limits {
        enabled: true,
        ceiling_per_min: 60,
        burst: 3,
        chrome_spacing: Duration::from_millis(1_000),
        content_max_hold: Duration::from_secs(30),
        pause_fallback: Duration::from_secs(5),
        pause_ceiling: Duration::from_secs(20),
    }
}

/// Every surface, for the loops that assert "all of them".
const ALL_SURFACES: [Surface; 3] = [Surface::Send, Surface::Final, Surface::Chrome];

// ---------------------------------------------------------------------------
// Pure math
// ---------------------------------------------------------------------------

#[test]
fn a_disabled_governor_admits_every_surface() {
    // The master switch is the escape hatch the issue asked for: one config
    // line restores pre-#1910 behaviour, so it must be genuinely inert.
    let l = Limits {
        enabled: false,
        ..limits()
    };
    let t = Instant::now();
    let mut b = ChannelBudget::default();
    for s in ALL_SURFACES {
        assert_eq!(
            admit(&mut b, &l, s, Duration::ZERO, t),
            Decision::Admit,
            "{s:?}"
        );
    }
}

#[test]
fn chrome_inside_the_spacing_window_is_dropped_and_the_next_tick_recovers() {
    let l = limits();
    let t0 = Instant::now();
    let mut b = ChannelBudget::default();
    assert_eq!(
        admit(&mut b, &l, Surface::Chrome, Duration::ZERO, t0),
        Decision::Admit
    );
    // 999 ms later: still one paint too many for the same second of clock.
    assert_eq!(
        admit(
            &mut b,
            &l,
            Surface::Chrome,
            Duration::ZERO,
            t0 + Duration::from_millis(999)
        ),
        Decision::Drop
    );
    // On the window boundary the next tick paints again: nothing was queued,
    // nothing was lost, the state is restated.
    assert_eq!(
        admit(
            &mut b,
            &l,
            Surface::Chrome,
            Duration::ZERO,
            t0 + Duration::from_millis(1_000)
        ),
        Decision::Admit
    );
}

#[test]
fn spacing_only_governs_chrome_content_never_waits_for_a_clock_line() {
    // The rule that keeps an answer behind a ticker edit, not in front of it.
    let l = limits();
    let t0 = Instant::now();
    let mut b = ChannelBudget::default();
    assert_eq!(
        admit(&mut b, &l, Surface::Chrome, Duration::ZERO, t0),
        Decision::Admit
    );
    for s in [Surface::Send, Surface::Final] {
        assert_eq!(
            admit(&mut b, &l, s, Duration::ZERO, t0),
            Decision::Admit,
            "{s:?}"
        );
    }
}

#[test]
fn a_dry_bucket_drops_chrome_holds_content_then_fails_open() {
    // burst 1: one write is free, after that the ceiling bites.
    let l = Limits {
        burst: 1,
        ..limits()
    };
    let t0 = Instant::now();
    let mut b = ChannelBudget::default();
    assert_eq!(
        admit(&mut b, &l, Surface::Send, Duration::ZERO, t0),
        Decision::Admit
    );
    // Chrome on dry: discarded, never held.
    assert_eq!(
        admit(&mut b, &l, Surface::Chrome, Duration::ZERO, t0),
        Decision::Drop
    );
    // Content on dry: held for the refill (#297), and the hold is a real,
    // bounded number rather than "forever".
    match admit(&mut b, &l, Surface::Final, Duration::ZERO, t0) {
        Decision::Hold(wait) => {
            assert!(wait > Duration::ZERO, "a dry bucket must hold, not admit");
            assert!(
                wait <= Duration::from_secs(1),
                "one token at 60/min: {wait:?}"
            );
        }
        other => panic!("expected Hold, got {other:?}"),
    }
    // Past the hold ceiling the write goes anyway: a late answer beats a lost
    // one, and serenity's limiter plus a real 429 are still downstream.
    assert_eq!(
        admit(&mut b, &l, Surface::Final, Duration::from_secs(30), t0),
        Decision::FailOpen
    );
}

#[test]
fn a_chrome_write_charges_the_bucket_it_consumes() {
    // If chrome were free it could starve content forever: 60/min with burst
    // 3, three chrome paints exhaust it, the fourth decision proves the charge.
    let l = Limits {
        burst: 3,
        chrome_spacing: Duration::ZERO,
        ..limits()
    };
    let t0 = Instant::now();
    let mut b = ChannelBudget::default();
    for i in 0..3 {
        assert_eq!(
            admit(
                &mut b,
                &l,
                Surface::Chrome,
                Duration::ZERO,
                t0 + Duration::from_millis(i)
            ),
            Decision::Admit
        );
    }
    assert_eq!(
        admit(
            &mut b,
            &l,
            Surface::Chrome,
            Duration::ZERO,
            t0 + Duration::from_millis(3)
        ),
        Decision::Drop
    );
}

#[test]
fn a_parked_channel_holds_content_and_discards_chrome() {
    let l = limits();
    let t0 = Instant::now();
    let mut b = ChannelBudget::default();
    assert_eq!(
        note_429(&mut b, &l, Some(Duration::from_secs(4)), t0),
        Duration::from_secs(4)
    );
    match admit(&mut b, &l, Surface::Send, Duration::ZERO, t0) {
        Decision::Hold(wait) => assert_eq!(wait, Duration::from_secs(4)),
        other => panic!("a park must hold content, got {other:?}"),
    }
    assert_eq!(
        admit(&mut b, &l, Surface::Chrome, Duration::ZERO, t0),
        Decision::Drop
    );
    // The window closes by itself: 5 s in, writes flow again with no help.
    assert_eq!(
        admit(
            &mut b,
            &l,
            Surface::Send,
            Duration::ZERO,
            t0 + Duration::from_secs(5)
        ),
        Decision::Admit
    );
}

#[test]
fn a_park_holds_until_its_last_millisecond_then_stops_holding() {
    // Boundary guard on `is_paused`: an instant-exact expiry must NOT keep
    // holding content, or a 429 would stall the channel for good.
    let l = limits();
    let t0 = Instant::now();
    let mut b = ChannelBudget::default();
    note_429(&mut b, &l, Some(Duration::from_secs(4)), t0);
    assert_eq!(
        admit(
            &mut b,
            &l,
            Surface::Chrome,
            Duration::ZERO,
            t0 + Duration::from_millis(3_999)
        ),
        Decision::Drop
    );
    assert_eq!(
        admit(
            &mut b,
            &l,
            Surface::Chrome,
            Duration::ZERO,
            t0 + Duration::from_secs(4)
        ),
        Decision::Admit
    );
}

#[test]
fn note_429_falls_back_clamps_and_takes_the_latest_window() {
    let l = limits();
    let t0 = Instant::now();
    let mut b = ChannelBudget::default();
    // Nothing named: the configured fallback window, not zero and not panic.
    assert_eq!(note_429(&mut b, &l, None, t0), Duration::from_secs(5));
    // A named window longer than the ceiling parks for the ceiling only: a
    // governor that stalls a channel forever is a worse outage than the one
    // it prevents.
    assert_eq!(
        note_429(&mut b, &l, Some(Duration::from_secs(3_600)), t0),
        Duration::from_secs(20)
    );
    // Latest wins upward, and a shorter ask can never shorten a park already
    // armed - the API's word is the floor, not a suggestion to renegotiate.
    assert_eq!(
        note_429(&mut b, &l, Some(Duration::from_secs(1)), t0),
        Duration::from_secs(20)
    );
    // A fresh budget is an unparked budget: no shared state leaks between
    // channels.
    let mut fresh = ChannelBudget::default();
    assert_eq!(
        admit(&mut fresh, &l, Surface::Chrome, Duration::ZERO, t0),
        Decision::Admit
    );
}

#[test]
fn only_a_429_is_a_rate_limit() {
    // A deleted message or a network fault must not park a channel: before
    // this layer every failure looked the same in the log, which is the
    // observability half of the issue.
    let plain = serenity::Error::Other("message was deleted");
    assert!(!Governor::is_rate_limited(&plain));
    assert_eq!(Governor::retry_after(&plain), None);
}

#[test]
fn a_named_retry_after_is_read_from_the_error_and_zero_is_not_a_window() {
    // Serenity 0.12.5 does not carry `retry_after` as a field
    // (`DiscordJsonError` is code/message/errors), so this is text arithmetic
    // on the rendered error. `Error::Other` renders its message verbatim,
    // which is the one shape a test can build without a live response.
    let named = serenity::Error::Other("error decoding response body: retry_after 2.5");
    assert_eq!(
        Governor::retry_after(&named),
        Some(Duration::from_secs_f64(2.5))
    );
    let spaced = serenity::Error::Other("429 too many requests, retry after 7 seconds");
    assert_eq!(Governor::retry_after(&spaced), Some(Duration::from_secs(7)));
    // A zero must answer None: parking for "0 s" would mean "not parked", and
    // the caller must land on the fallback instead of a no-op window.
    let zero = serenity::Error::Other("retry_after 0.0");
    assert_eq!(Governor::retry_after(&zero), None);
}

// ---------------------------------------------------------------------------
// Gate wiring (real Governor, virtual clock, swapped config)
// ---------------------------------------------------------------------------

/// Install a config mirror with the governor knobs this suite expects, run
/// `body`, then put the previous mirror back. The registry guard serializes
/// against every other test here; the restore keeps the full-suite gate honest,
/// since `Config::current()` is one process-wide mirror. #2018: the swap lands
/// in state the Telegram gates read while holding `telegram_cooldown_lock`, and
/// Discord's registry blocks nobody outside this suite, so call sites must take
/// that guard BEFORE invoking this macro, not after. No Discord body waits on
/// the cooldown while holding the registry, so cooldown-then-registry cannot
/// close a cycle.
macro_rules! governor_config {
    ($($field:ident : $value:expr),* $(,)?) => {{
        let _guard = ts::registry_guard().await;
        ts::reset(0);
        let prev = Config::current();
        let mut cfg: Config = toml::from_str(include_str!("../../config.toml.example"))
            .expect("embedded config.toml.example must parse");
        $(cfg.channels.discord.governor.$field = $value;)*
        Config::set_current(cfg);
        (prev, _guard)
    }};
}

#[tokio::test]
async fn chrome_admits_the_first_paint_and_refuses_the_one_inside_the_window() {
    // `ts::advance` moves the process-wide virtual clock the global 429
    // cooldown rides on, so this test must not run beside the other guarded
    // ones. The cooldown comes BEFORE the macro now (#2018): the macro holds
    // only Discord's own registry, and its config swap lands in the
    // process-wide mirror the Telegram gates read while holding this lock.
    // No Discord-registry holder ever waits on the cooldown lock, so this
    // order cannot cycle.
    let _cooldown = crate::tests::telegram_cooldown_lock::guard().await;
    let (prev, _guard) = governor_config!(chrome_min_spacing_ms: 1_000u64);
    let g = Governor::default();
    let ch = 7u64;
    assert!(g.chrome_admits(ch), "the first paint of a turn must go out");
    ts::advance(500);
    assert!(
        !g.chrome_admits(ch),
        "a second paint inside the window is dropped"
    );
    ts::advance(600);
    assert!(g.chrome_admits(ch), "and the next tick restates it");
    if let Ok(cfg) = Arc::try_unwrap(prev) {
        Config::set_current(cfg);
    }
}

#[tokio::test]
async fn a_429_parks_its_channel_and_no_other() {
    // Same discipline as above: the cooldown comes BEFORE the macro so the
    // swap lands inside the lock the Telegram gates read the mirror under.
    // #1854, #2018.
    let _cooldown = crate::tests::telegram_cooldown_lock::guard().await;
    let (prev, _guard) = governor_config!(pause_secs: 5u64);
    let g = Governor::default();
    let parked = 11u64;
    let neighbour = 12u64;
    // The API named 3 s; that is what we honor, and it is shorter than the
    // fallback, which is the point of reading it at all.
    g.note_rate_limited(parked, Some(Duration::from_secs(3)));
    assert!(!g.chrome_admits(parked), "a parked channel gets no chrome");
    assert!(
        g.chrome_admits(neighbour),
        "the park is per channel, not global"
    );
    ts::advance(3_100);
    assert!(g.chrome_admits(parked), "the window closes on its own");
    if let Ok(cfg) = Arc::try_unwrap(prev) {
        Config::set_current(cfg);
    }
}

#[tokio::test]
async fn a_gate_on_a_disabled_governor_returns_immediately() {
    // The escape hatch has to be inert at the ASYNC layer too, not just in the
    // pure math: a config that says "off" must never park a caller in `gate`.
    // This body used to swap the mirror holding NO cross-family lock at all:
    // the #2018 clobber. The cooldown guard now covers the swap.
    let _cooldown = crate::tests::telegram_cooldown_lock::guard().await;
    let (prev, _guard) = governor_config!(enabled: false);
    let g = Governor::default();
    let t = Instant::now();
    for s in ALL_SURFACES {
        g.gate(99, s).await;
    }
    assert!(
        t.elapsed() < Duration::from_secs(1),
        "a disabled gate must not wait"
    );
    if let Ok(cfg) = Arc::try_unwrap(prev) {
        Config::set_current(cfg);
    }
}
