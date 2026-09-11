//! Outbound rate limiting for WhatsApp (#1407).
//!
//! The channel with the heaviest ban consequences was the only one with
//! no budget machinery: streaming turns post every chunk as its own
//! message, exactly the volume pattern Meta's heuristics hunt. This
//! module mirrors the SHAPE of Telegram's governor budgets without its
//! complexity:
//!
//! - Token bucket (`messages_per_minute`, default 30): burst capacity
//!   equals the per-minute rate, refilled continuously. Over-budget
//!   sends are PACED (the caller sleeps for the refill gap), never
//!   dropped - [`Decision`] has no reject variant.
//! - Rolling 24h cap (`daily_cap`, default 800): hard stop. On
//!   saturation the send parks in a FIFO queue that the drainer task
//!   flushes as the window slides.
//! - Exactly-once owner alert per saturation episode: the first queued
//!   send of an episode flags it; the drainer notifies `owner_jid` once
//!   and re-arms when the window drops back under the cap.
//! - Owner bypass: sends addressed to the owner skip budget consumption
//!   entirely (reactivity preserved - the issue's owner-initiated-reply
//!   case), as does the alert itself (safety notice, not content).
//!
//! All budget math lives in the synchronous, I/O-free [`admit`] and
//! [`drain_ready`] cores which take `now: Instant` explicitly, so tests
//! advance time by hand - no sleeps, no global clock seam.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use crate::config::types::WaRateLimitConfig;

/// A send parked when the daily cap saturated. The drainer flushes it as
/// a plain conversation message once the rolling window recovers (quote
/// context, if any, is lost - acceptable degradation, documented).
#[derive(Debug, Clone, PartialEq)]
pub struct QueuedSend {
    pub jid: String,
    pub text: String,
}

/// Pure admission decision from [`admit`].
#[derive(Debug, PartialEq)]
pub enum Decision {
    /// Budget consumed (one token + one window timestamp): send now.
    SendNow,
    /// Bucket empty: sleep `wait` for refill, then admit again. Nothing
    /// was consumed. Sends are paced, never dropped.
    Pace(Duration),
    /// Rolling cap full: park the send in the FIFO queue. `alert` is
    /// true exactly once per saturation episode (first queue decision).
    Queue { alert: bool },
}

/// Budget math state. Lives inside [`WhatsappRateLimiter`]'s mutex; the
/// pure cores take it by `&mut` so tests drive it directly.
#[derive(Debug, Default)]
pub struct LimiterState {
    /// Token bucket level (fractional during refill).
    tokens: f64,
    /// Last refill instant; None = never admitted (bucket starts full).
    last_refill: Option<Instant>,
    /// Rolling 24h send timestamps, oldest first.
    window: VecDeque<Instant>,
    /// True while a daily-cap saturation episode is in flight (drives
    /// the exactly-once alert flag in [`Decision::Queue`]).
    saturation: bool,
    /// FIFO of parked sends awaiting window recovery. Tests and the
    /// drainer push/pop here directly.
    pub(crate) queue: VecDeque<QueuedSend>,
}

impl LimiterState {
    /// Queued-send count (alert text, tool reports).
    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    /// True while a saturation episode is in flight.
    pub fn saturated(&self) -> bool {
        self.saturation
    }
}

/// Pure budget math - no I/O, no clock reads (`now` is injected).
///
/// On [`Decision::SendNow`] one token and one window timestamp are
/// consumed. [`Decision::Pace`] consumes nothing (re-admit after the
/// sleep). [`Decision::Queue`] consumes nothing; the caller parks the
/// text (see [`WhatsappRateLimiter::gate`]).
pub fn admit(
    state: &mut LimiterState,
    cfg: &WaRateLimitConfig,
    now: Instant,
    is_owner: bool,
) -> Decision {
    // Owner-bound sends bypass the budget entirely (reactivity).
    if is_owner {
        return Decision::SendNow;
    }
    let bucket_on = cfg.messages_per_minute > 0;
    let cap_on = cfg.daily_cap > 0;
    if !bucket_on && !cap_on {
        return Decision::SendNow; // limiter disabled
    }

    // Continuous refill: per_minute / 60 tokens per second, capped at
    // the per-minute rate (the burst capacity).
    if bucket_on {
        let capacity = cfg.messages_per_minute as f64;
        state.tokens = match state.last_refill {
            Some(last) => {
                let elapsed = now.saturating_duration_since(last).as_secs_f64();
                (state.tokens + elapsed * capacity / 60.0).min(capacity)
            }
            None => capacity,
        };
        state.last_refill = Some(now);
    }

    // Prune the rolling 24h window, then hard-stop on the cap.
    if cap_on {
        let cutoff = now - Duration::from_secs(24 * 60 * 60);
        while let Some(front) = state.window.front() {
            if *front <= cutoff {
                state.window.pop_front();
            } else {
                break;
            }
        }
        if state.window.len() >= cfg.daily_cap as usize {
            let first_of_episode = !state.saturation;
            state.saturation = true;
            return Decision::Queue {
                alert: first_of_episode,
            };
        }
        // Window slid back under the cap: episode over, re-arm alert.
        state.saturation = false;
    }

    // Token bucket: pace (never drop) when empty.
    if bucket_on {
        if state.tokens < 1.0 {
            let capacity = cfg.messages_per_minute as f64;
            let wait = Duration::from_secs_f64((1.0 - state.tokens) * 60.0 / capacity);
            return Decision::Pace(wait);
        }
        state.tokens -= 1.0;
    }
    if cap_on {
        state.window.push_back(now);
    }
    Decision::SendNow
}

/// Pure drain core: pop queued sends the recovered budget allows, FIFO.
/// Each pop consumes budget through [`admit`] (owner flag false -
/// drained sends are bulk by definition). Stops at the first non-SendNow
/// decision. Note: a flush that later fails transmission is re-queued
/// WITHOUT refunding its consumed budget (v1: budget under-counts by
/// the failed attempt, acceptable).
pub fn drain_ready(
    state: &mut LimiterState,
    cfg: &WaRateLimitConfig,
    now: Instant,
) -> Vec<QueuedSend> {
    let mut out = Vec::new();
    while !state.queue.is_empty() {
        match admit(state, cfg, now, false) {
            Decision::SendNow => {
                if let Some(item) = state.queue.pop_front() {
                    out.push(item);
                }
            }
            _ => break,
        }
    }
    out
}

/// Outcome of [`WhatsappRateLimiter::gate`] for one send.
#[derive(Debug, PartialEq)]
pub enum GateOutcome {
    /// Proceed with the send now (budget consumed, or owner bypass).
    SendNow,
    /// Parked in the daily-cap queue at 1-based `position`; the drainer
    /// flushes it as the window slides.
    Queued { position: usize },
}

/// The channel-wide limiter: budget state + saturation-alert flags.
/// Hangs off `WhatsAppState` so the tool, handler chunk loops, resume
/// path and drainer share ONE budget.
#[derive(Default)]
pub struct WhatsappRateLimiter {
    state: Mutex<LimiterState>,
    /// Set by `gate` on the first Queue of an episode; the drainer
    /// consumes it to deliver the owner alert exactly once.
    alert_pending: AtomicBool,
    /// True while this episode's alert has been delivered.
    alert_sent: AtomicBool,
}

impl WhatsappRateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Admit one outbound send. Paces (sleeps) on an empty bucket and
    /// re-admits - over-budget sends are never dropped. On daily-cap
    /// saturation the text parks in the FIFO queue and the caller gets
    /// [`GateOutcome::Queued`].
    pub async fn gate(
        &self,
        cfg: &WaRateLimitConfig,
        jid: &str,
        text: &str,
        is_owner: bool,
    ) -> GateOutcome {
        loop {
            let decision = {
                let mut state = self.state.lock().await;
                admit(&mut state, cfg, Instant::now(), is_owner)
            };
            match decision {
                Decision::SendNow => return GateOutcome::SendNow,
                Decision::Pace(wait) => tokio::time::sleep(wait).await,
                Decision::Queue { alert } => {
                    let position = {
                        let mut state = self.state.lock().await;
                        state.queue.push_back(QueuedSend {
                            jid: jid.to_string(),
                            text: text.to_string(),
                        });
                        state.queue.len()
                    };
                    if alert {
                        self.alert_pending.store(true, Ordering::SeqCst);
                    }
                    return GateOutcome::Queued { position };
                }
            }
        }
    }

    /// Pacing-only admission for EPHEMERAL sends (streaming intermediates):
    /// the token bucket paces as usual, but a saturated daily cap DROPS
    /// the send instead of queueing it. A stale partial update is
    /// worthless by the time the window slides, and queued intermediates
    /// would confuse the edit-handle tracking. Returns true when the send
    /// may proceed. A saturating episode still arms the one-time owner
    /// alert (#1407).
    pub async fn gate_ephemeral(&self, cfg: &WaRateLimitConfig, is_owner: bool) -> bool {
        loop {
            let decision = {
                let mut state = self.state.lock().await;
                admit(&mut state, cfg, Instant::now(), is_owner)
            };
            match decision {
                Decision::SendNow => return true,
                Decision::Pace(wait) => tokio::time::sleep(wait).await,
                Decision::Queue { alert } => {
                    if alert {
                        self.alert_pending.store(true, Ordering::SeqCst);
                    }
                    return false;
                }
            }
        }
    }

    /// Queue depth (reports, alert text).
    pub async fn queue_len(&self) -> usize {
        self.state.lock().await.queue_len()
    }

    /// One drainer pass: flush queued sends while budget allows and
    /// re-arm the alert flags when the saturation episode ends.
    pub async fn drain(&self, cfg: &WaRateLimitConfig) -> Vec<QueuedSend> {
        let mut state = self.state.lock().await;
        let ready = drain_ready(&mut state, cfg, Instant::now());
        if !state.saturated() {
            // Episode over: the NEXT saturation alerts again.
            self.alert_pending.store(false, Ordering::SeqCst);
            self.alert_sent.store(false, Ordering::SeqCst);
        }
        ready
    }

    /// Put a failed flush back at the FRONT of the queue (drainer
    /// retries next tick; v1: no retry counter, failures warn).
    pub async fn requeue_front(&self, item: QueuedSend) {
        self.state.lock().await.queue.push_front(item);
    }

    /// True when the drainer owes the owner a saturation alert.
    pub fn alert_due(&self) -> bool {
        self.alert_pending.load(Ordering::SeqCst) && !self.alert_sent.load(Ordering::SeqCst)
    }

    /// Mark this episode's alert delivered.
    pub fn mark_alert_sent(&self) {
        self.alert_sent.store(true, Ordering::SeqCst);
    }
}

/// Singleton guard: the agent can restart (reconcile_whatsapp), but the
/// drainer must exist exactly once per process - duplicates would
/// double-send queued items and alerts.
static DRAINER_SPAWNED: AtomicBool = AtomicBool::new(false);

/// Spawn the background drainer (once per process). Every 60s it
/// delivers the pending owner alert (exactly once per episode), flushes
/// queued sends while the recovered budget allows, and re-arms the
/// alert when the episode ends. Config is snapshotted at first spawn
/// (the inline gate paths always read fresh config; a drainer-side
/// change needs a restart - documented v1 behavior).
pub(crate) fn spawn_drainer(wa_state: Arc<super::WhatsAppState>, cfg: WaRateLimitConfig) {
    if DRAINER_SPAWNED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(60));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let limiter = &wa_state.rate_limiter;

            // 1. Owner alert (exactly once per saturation episode).
            //    Bypasses the limiter itself - safety notice, not content.
            if limiter.alert_due() {
                let client = wa_state.client.lock().await.clone();
                let owner_jid = wa_state
                    .owner_jid
                    .lock()
                    .await
                    .clone()
                    .and_then(|j| j.parse::<wacore_binary::jid::Jid>().ok());
                if let (Some(client), Some(owner_jid)) = (client, owner_jid) {
                    let queued = limiter.queue_len().await;
                    let text = format!(
                        "{}\n\nWhatsApp daily cap reached ({} messages / rolling 24h). {} message(s) queued; they flush automatically as the window slides. Messages to you are never limited.",
                        super::handler::MSG_HEADER,
                        cfg.daily_cap,
                        queued
                    );
                    let msg = waproto::whatsapp::Message {
                        conversation: Some(text),
                        ..Default::default()
                    };
                    match client.send_message(owner_jid, msg).await {
                        Ok(_) => limiter.mark_alert_sent(),
                        Err(e) => {
                            tracing::warn!("WhatsApp rate-limit: owner alert failed: {e}");
                        }
                    }
                }
            }

            // 2. Flush queued sends while budget allows.
            for item in limiter.drain(&cfg).await {
                let client = wa_state.client.lock().await.clone();
                let Some(client) = client else {
                    // No client (not connected): put it back, stop this tick.
                    limiter.requeue_front(item).await;
                    break;
                };
                let Ok(jid) = item.jid.parse::<wacore_binary::jid::Jid>() else {
                    tracing::warn!(
                        "WhatsApp rate-limit: queued jid {} unparseable; dropping",
                        item.jid
                    );
                    continue;
                };
                let msg = waproto::whatsapp::Message {
                    conversation: Some(item.text.clone()),
                    ..Default::default()
                };
                match client.send_message(jid.clone(), msg).await {
                    Ok(_) => {
                        // Persist flushed sends so history matches reality
                        // (same channel_messages contract as the tool).
                        crate::brain::tools::whatsapp_send::persist_outgoing(&jid, &item.text)
                            .await;
                    }
                    Err(e) => {
                        tracing::warn!(
                            "WhatsApp rate-limit: queued send to {} failed: {e}; re-queueing",
                            item.jid
                        );
                        limiter.requeue_front(item).await;
                        break; // stop this tick; retry next
                    }
                }
            }
        }
    });
}
