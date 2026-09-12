//! Rate-limiter budget math for #1407: burst pacing without drops,
//! rolling-cap queueing with an exactly-once alert, owner bypass,
//! unlimited knobs, window-slide recovery, FIFO drain, and serde
//! defaults. The pure cores (`admit`, `drain_ready`) take `now`
//! explicitly, so every test advances time by hand - no sleeps.

use std::time::{Duration, Instant};

use crate::channels::whatsapp::rate_limit::{
    Decision, LimiterState, QueuedSend, WhatsappRateLimiter, admit, drain_ready,
};
use crate::config::types::WaRateLimitConfig;

fn cfg(per_minute: u32, daily_cap: u32) -> WaRateLimitConfig {
    WaRateLimitConfig {
        messages_per_minute: per_minute,
        daily_cap,
    }
}

/// Acceptance 1: a burst of 50 under the default config all deliver,
/// pacing is enforced, nothing drops.
#[test]
fn burst_of_fifty_paces_without_drops() {
    let cfg = WaRateLimitConfig::default(); // 30/min, 800/day
    let mut state = LimiterState::default();
    let t0 = Instant::now();
    let mut now = t0;
    let mut delivered = 0;
    let mut paced = 0;
    while delivered < 50 {
        match admit(&mut state, &cfg, now, false) {
            Decision::SendNow => delivered += 1,
            Decision::Pace(wait) => {
                assert!(wait > Duration::ZERO, "pace must actually wait");
                assert!(
                    wait <= Duration::from_secs(2),
                    "refill gap at 30/min is at most 2s, got {:?}",
                    wait
                );
                now += wait;
                paced += 1;
            }
            Decision::Queue { .. } => panic!("burst of 50 must never hit the 800/day cap"),
        }
    }
    assert_eq!(delivered, 50);
    assert_eq!(
        paced, 20,
        "bucket holds 30; sends 31-50 pace one refill gap each"
    );
    assert!(
        now - t0 >= Duration::from_secs(38),
        "20 waits of ~2s must stretch the burst to ~40s, got {:?}",
        now - t0
    );
}

/// Acceptance 2: cap hit -> sends queue; the alert fires exactly once
/// per saturation episode.
#[test]
fn daily_cap_queues_with_exactly_one_alert() {
    let cfg = cfg(0, 5); // no pacing, cap 5
    let mut state = LimiterState::default();
    let now = Instant::now();
    for _ in 0..5 {
        assert_eq!(admit(&mut state, &cfg, now, false), Decision::SendNow);
    }
    assert_eq!(
        admit(&mut state, &cfg, now, false),
        Decision::Queue { alert: true },
        "first over-cap send raises the one alert"
    );
    assert_eq!(
        admit(&mut state, &cfg, now, false),
        Decision::Queue { alert: false }
    );
    assert_eq!(
        admit(&mut state, &cfg, now, false),
        Decision::Queue { alert: false }
    );
    assert!(state.saturated());
}

/// Window slide ends the episode: the queue drains FIFO with budget and
/// the NEXT saturation alerts again (exactly-once is per episode).
#[test]
fn window_slide_resets_episode_and_drains_fifo() {
    let cfg = cfg(0, 2);
    let mut state = LimiterState::default();
    let t0 = Instant::now();
    assert_eq!(admit(&mut state, &cfg, t0, false), Decision::SendNow);
    assert_eq!(admit(&mut state, &cfg, t0, false), Decision::SendNow);
    assert_eq!(
        admit(&mut state, &cfg, t0, false),
        Decision::Queue { alert: true }
    );
    state.queue.push_back(QueuedSend {
        jid: "a@s.whatsapp.net".into(),
        text: "first".into(),
    });
    state.queue.push_back(QueuedSend {
        jid: "b@s.whatsapp.net".into(),
        text: "second".into(),
    });
    assert!(state.saturated(), "episode in flight before the slide");

    // 24h later the window slides: episode ends, drainer flushes FIFO.
    let later = t0 + Duration::from_secs(24 * 3600 + 1);
    let drained = drain_ready(&mut state, &cfg, later);
    assert_eq!(
        drained.len(),
        2,
        "both queued sends flush once the window empties"
    );
    assert_eq!(drained[0].text, "first");
    assert_eq!(drained[1].text, "second");
    assert!(!state.saturated(), "episode cleared by the slide");

    // Fresh saturation alerts again: window now holds the 2 drained sends.
    assert_eq!(
        admit(&mut state, &cfg, later, false),
        Decision::Queue { alert: true }
    );
}

/// Owner-bound sends bypass the budget: no consumption, no pacing, no
/// queueing even on a saturated cap and an empty bucket.
#[test]
fn owner_bypasses_a_saturated_budget() {
    let cfg = cfg(1, 1);
    let mut state = LimiterState::default();
    let now = Instant::now();
    // Saturate: cap 1 filled, bucket 1 emptied.
    assert_eq!(admit(&mut state, &cfg, now, false), Decision::SendNow);
    assert_eq!(
        admit(&mut state, &cfg, now, false),
        Decision::Queue { alert: true }
    );
    // Owner still flows and consumes nothing.
    for _ in 0..5 {
        assert_eq!(admit(&mut state, &cfg, now, true), Decision::SendNow);
    }
    assert_eq!(state.queue_len(), 0);
    // A non-owner send is still capped afterwards.
    assert_eq!(
        admit(&mut state, &cfg, now, false),
        Decision::Queue { alert: false }
    );
}

/// 0 = unlimited on both knobs.
#[test]
fn zero_knobs_disable_limiting() {
    let cfg = cfg(0, 0);
    let mut state = LimiterState::default();
    let now = Instant::now();
    for _ in 0..1000 {
        assert_eq!(admit(&mut state, &cfg, now, false), Decision::SendNow);
    }
}

/// drain_ready stops when the budget runs out and keeps FIFO order.
#[test]
fn drain_ready_respects_budget() {
    let cfg = cfg(0, 2);
    let mut state = LimiterState::default();
    for i in 0..3 {
        state.queue.push_back(QueuedSend {
            jid: "x@s.whatsapp.net".into(),
            text: format!("m{i}"),
        });
    }
    let drained = drain_ready(&mut state, &cfg, Instant::now());
    assert_eq!(drained.len(), 2, "cap 2 allows two flushes");
    assert_eq!(drained[0].text, "m0");
    assert_eq!(drained[1].text, "m1");
    assert_eq!(state.queue_len(), 1, "m2 waits for the next window slide");
}

/// Serde: container defaults give 30/800; partial config keeps the
/// other default; explicit values win; nesting through WhatsAppConfig
/// works with the `[rate_limit]` table.
#[test]
fn config_serde_defaults_and_overrides() {
    let empty: WaRateLimitConfig = toml::from_str("").unwrap();
    assert_eq!(empty.messages_per_minute, 30);
    assert_eq!(empty.daily_cap, 800);

    let partial: WaRateLimitConfig = toml::from_str("messages_per_minute = 10").unwrap();
    assert_eq!(partial.messages_per_minute, 10);
    assert_eq!(partial.daily_cap, 800);

    let full: WaRateLimitConfig =
        toml::from_str("messages_per_minute = 5\ndaily_cap = 100").unwrap();
    assert_eq!(
        full,
        WaRateLimitConfig {
            messages_per_minute: 5,
            daily_cap: 100
        }
    );

    let wa: crate::config::types::WhatsAppConfig =
        toml::from_str("enabled = true\n[rate_limit]\nmessages_per_minute = 7").unwrap();
    assert_eq!(wa.rate_limit.messages_per_minute, 7);
    assert_eq!(wa.rate_limit.daily_cap, 800);
}

/// The async surface over the pure core: gate parks the text, reports
/// the 1-based queue position, and honors the owner bypass. The Queue
/// path never sleeps (only Pace does), so this runs instantly.
#[tokio::test]
async fn gate_queues_and_reports_position() {
    use crate::channels::whatsapp::rate_limit::{GateOutcome, WhatsappRateLimiter};
    let limiter = WhatsappRateLimiter::new();
    let cfg = cfg(0, 1);
    let jid = "25512345678@s.whatsapp.net";
    assert_eq!(
        limiter.gate(&cfg, jid, "first", false).await,
        GateOutcome::SendNow
    );
    assert_eq!(
        limiter.gate(&cfg, jid, "second", false).await,
        GateOutcome::Queued { position: 1 }
    );
    assert_eq!(limiter.queue_len().await, 1);
    assert!(
        limiter.alert_due(),
        "first queue of the episode owes an alert"
    );
    limiter.mark_alert_sent();
    assert!(!limiter.alert_due(), "exactly once: alert delivered");
    // Owner bypass through the async surface too.
    assert_eq!(
        limiter.gate(&cfg, jid, "to-owner", true).await,
        GateOutcome::SendNow
    );
}

/// Ephemeral gate: paces like gate() but DROPS instead of queueing under
/// saturation (stale intermediates are worthless), still arms the alert.
#[tokio::test]
async fn ephemeral_gate_drops_instead_of_queueing() {
    let limiter = WhatsappRateLimiter::new();
    let cfg = cfg(0, 1);
    assert!(
        limiter.gate_ephemeral(&cfg, false).await,
        "first send within cap"
    );
    assert!(
        !limiter.gate_ephemeral(&cfg, false).await,
        "saturated: ephemeral sends drop"
    );
    assert_eq!(limiter.queue_len().await, 0, "drops must not queue");
    assert!(
        limiter.alert_due(),
        "saturation still arms the one-time alert"
    );
    assert!(limiter.gate_ephemeral(&cfg, true).await, "owner bypasses");
}

/// Wiring sentinel (#1407): every agent-output send path gates through the
/// shared limiter before hitting the wire - tool send + reply arms, the
/// streamed intermediates (ephemeral gate) and final-text chunks, and the
/// bg-resume path. The drainer is spawned from the agent start path. The
/// connection greeting, command replies and self-heal alerts are
/// intentionally unwired (owner-bound or system traffic).
///
/// The streamed half moved to `stream.rs` with #1408, which turned chunk
/// spam into one edited message. Both of its paths are checked, not just
/// one: an edit is a stanza on the wire exactly like a fresh send, and a
/// version that paced only the new-message path would look wired here
/// while letting an edit-per-token turn run unbudgeted.
#[test]
fn agent_output_paths_gate_through_the_limiter() {
    const TOOL: &str = include_str!("../brain/tools/whatsapp_send.rs");
    const HANDLER: &str = include_str!("../channels/whatsapp/handler.rs");
    const STREAM: &str = include_str!("../channels/whatsapp/stream.rs");
    const RESUME: &str = include_str!("../channels/whatsapp/resume.rs");
    const AGENT: &str = include_str!("../channels/whatsapp/agent.rs");
    assert_eq!(
        TOOL.matches(".gate(&rl_cfg").count(),
        2,
        "tool send and reply arms must gate per chunk"
    );
    assert_eq!(
        STREAM.matches(".gate_ephemeral(&config.rate_limit").count(),
        2,
        "both streamed paths must use the ephemeral gate: the edit and the \
         new message that starts a fresh one"
    );
    assert!(
        HANDLER.contains(".gate(&wa_cfg.rate_limit"),
        "final-text chunks must gate"
    );
    assert!(
        RESUME.contains(".gate(&wa_cfg.rate_limit"),
        "bg-resume send must gate"
    );
    assert!(
        AGENT.contains("spawn_drainer("),
        "agent start must spawn the drainer exactly once"
    );
}
