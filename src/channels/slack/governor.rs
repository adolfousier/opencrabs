//! Outbound write governor for Slack (#2012).
//!
//! Discord ships this layer in [`crate::channels::discord::governor`] (#1910)
//! and Telegram has shipped the product version of it far longer
//! ([`crate::channels::telegram::governor`]). Slack had neither: every
//! `chat.postMessage` and `chat.update` left the process ungated, and a
//! `Retry-After` in a 429 was logged as a generic failure with nothing
//! separating "Slack told us to slow down" from "the message was deleted
//! under us". That is the same exposure, and the same two surfaces make it
//! acute: the step-group bubble and the 4 s flow ticker
//! (`handler.rs::spawn_flow_ticker`) that keeps rewriting its clock. A turn
//! with eight tool calls is one message edited every few seconds, all of it
//! cosmetic, all of it competing with the answer that has to land at settle.
//!
//! The policy is the Discord policy, with Slack's error shape:
//!
//! - **Content is delay-never-drop** ([`Surface::Send`], [`Surface::Final`]).
//!   A breach holds the write until the budget refills; past
//!   [`Limits::content_max_hold`] it fails open and goes anyway, because a
//!   late answer beats a lost one.
//! - **Chrome yields** ([`Surface::Chrome`]). The ticker's clock line and the
//!   tool-group status re-render carry no information a later edit will not
//!   restate (every paint renders the FULL current group), so a chrome write
//!   is DROPPED the moment it is refused. It never waits.
//! - **A 429 is evidence.** Slack DOES publish per-method tier budgets and
//!   slack-morphism enforces them underneath, but the one number that says
//!   how long THIS response wanted is its `Retry-After`, and the response's
//!   `retry_after` is what parks the conversation. The config numbers are
//!   safety ceilings around that.
//!
//! All the math is synchronous and I/O-free in [`admit`] / [`note_429`], with
//! the clock injected as `now: Instant`, so tests advance time by hand instead
//! of sleeping. [`Governor::gate`] is the thin async shell, and
//! [`GatedWrites`] is the seam the call sites actually use: a Slack write goes
//! out through `session.post(..)` / `session.update(..)`, so the policy cannot
//! be skipped by forgetting to call it.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::config::Config;

/// Process-wide virtual clock offset, `cfg(test)` only. Every governor path
/// reads [`now`] instead of `Instant::now()`, so a test can move time by hand
/// and watch a park expire or a bucket refill without sleeping. Same seam as
/// Discord's `governor::now`, same reason: the suite must not depend on
/// wall-clock luck.
#[cfg(test)]
static CLOCK_OFFSET_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[cfg(not(test))]
fn now() -> Instant {
    Instant::now()
}

#[cfg(test)]
fn now() -> Instant {
    use std::sync::atomic::Ordering;
    let off = CLOCK_OFFSET_MS.load(Ordering::Relaxed);
    Instant::now() + Duration::from_millis(off)
}

/// Which kind of write is asking. The class decides what a dry budget does to
/// it; see the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Surface {
    /// A brand-new message: the answer, a notice, an approval prompt.
    Send,
    /// The last word on something already posted: a settle stamp, a waiting
    /// line flipped to its terminal line, an approval result label.
    Final,
    /// A live re-render of state already shown: the flow ticker's clock, the
    /// tool-group status edit. Safe to drop; the next paint restates it.
    Chrome,
}

impl Surface {
    /// True when a breach must never lose the payload.
    pub(crate) fn is_content(self) -> bool {
        matches!(self, Surface::Send | Surface::Final)
    }
}

/// What [`admit`] decided for one write.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Decision {
    /// Take the write now: the budget was charged.
    Admit,
    /// Content: sleep `wait`, then re-ask. Chrome never waits, it drops.
    Hold(Duration),
    /// Chrome on a dry, spaced-out or parked budget: discard this re-render.
    /// Nothing was charged and nothing is lost, the next tick paints the same
    /// state.
    Drop,
    /// The hold ceiling was reached and the write goes out anyway, uncharged.
    FailOpen,
}

/// Effective knobs, read from `[channels.slack.governor]` at each gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Limits {
    /// Master switch. Off means every gate answers [`Decision::Admit`] and
    /// records nothing: pre-#2012 behaviour, one config line away.
    pub enabled: bool,
    /// Rolling per-conversation write ceiling (all surfaces together) and its
    /// burst capacity. `ceiling == 0` disables the ceiling.
    pub ceiling_per_min: u32,
    pub burst: u32,
    /// Minimum spacing between two admitted chrome writes on one conversation.
    pub chrome_spacing: Duration,
    /// Longest a CONTENT write may be held before it fails open.
    pub content_max_hold: Duration,
    /// Fallback park window for a 429 that named no `Retry-After`.
    pub pause_fallback: Duration,
    /// Safety ceiling on any park window, named or fallback.
    pub pause_ceiling: Duration,
}

impl Limits {
    /// Snapshot the live config. Mirrors Discord's `Limits::from_config`; the
    /// hold ceiling is one tick shorter than the flow ticker's own 30-minute
    /// horizon, so a held write can still land before the ticker gives up.
    fn from_config() -> Self {
        let g = &Config::current().channels.slack.governor;
        Self {
            enabled: g.enabled,
            ceiling_per_min: g.writes_per_minute,
            burst: g.burst.max(1),
            chrome_spacing: Duration::from_millis(g.chrome_min_spacing_ms),
            content_max_hold: Duration::from_secs(25 * 60),
            pause_fallback: Duration::from_secs(g.pause_secs.max(1)),
            pause_ceiling: Duration::from_secs(g.pause_secs.max(1) * 4),
        }
    }
}

/// Per-conversation budget state. One entry per Slack channel id, created on
/// first write; the whole map lives under one lock because every path drops
/// the guard before it awaits (see [`Governor::gate`]).
#[derive(Debug, Default)]
pub(crate) struct ChannelBudget {
    /// Token bucket. Starts full so a turn's opening burst is free.
    pub(crate) tokens: f64,
    /// Last refill instant; `None` until the first refill runs.
    pub(crate) last_refill: Option<Instant>,
    /// When the last chrome write was admitted: the spacing clock.
    pub(crate) last_chrome: Option<Instant>,
    /// Until when this conversation is parked by a 429.
    pub(crate) pause_until: Option<Instant>,
}

impl ChannelBudget {
    /// Refill the bucket up to `burst` at `ceiling_per_min` tokens/minute and
    /// clear an expired park. Idempotent, so callers run it before every read.
    /// A disabled ceiling leaves the bucket infinite: nothing paces.
    fn refresh(&mut self, l: &Limits, now: Instant) {
        if l.ceiling_per_min == 0 {
            self.tokens = f64::INFINITY;
            self.last_refill = Some(now);
        } else {
            let capacity = l.burst.max(1) as f64;
            let rate = l.ceiling_per_min as f64 / 60.0;
            self.tokens = match self.last_refill {
                Some(last) => {
                    let dt = now.saturating_duration_since(last).as_secs_f64();
                    (self.tokens + dt * rate).min(capacity)
                }
                None => capacity,
            };
            self.last_refill = Some(now);
        }
        if self.pause_until.is_some_and(|until| until <= now) {
            self.pause_until = None;
        }
    }

    /// True while a 429 park is in force.
    fn is_paused(&self, now: Instant) -> bool {
        self.pause_until.is_some_and(|until| until > now)
    }
}

/// Pure admission for one write. No I/O, no clock reads: `now` is injected and
/// `held` is what the caller has already spent waiting, so a test can drive a
/// whole hold ladder in one line.
///
/// Charges the bucket only on [`Decision::Admit`]; [`Decision::FailOpen`]
/// charges nothing because the write bypassed the budget by policy.
pub(crate) fn admit(
    b: &mut ChannelBudget,
    l: &Limits,
    surface: Surface,
    held: Duration,
    now: Instant,
) -> Decision {
    if !l.enabled {
        return Decision::Admit;
    }
    b.refresh(l, now);

    // A parked conversation is a parked conversation: content waits out the
    // window, chrome has nothing useful to say into it.
    if b.is_paused(now) {
        let until = b.pause_until.unwrap_or(now);
        let wait = until.saturating_duration_since(now);
        return if surface.is_content() {
            if held + wait > l.content_max_hold {
                Decision::FailOpen
            } else {
                Decision::Hold(wait)
            }
        } else {
            Decision::Drop
        };
    }

    // Chrome spacing: two clock re-renders inside one window is one too many.
    if surface == Surface::Chrome
        && !l.chrome_spacing.is_zero()
        && let Some(last) = b.last_chrome
        && now.saturating_duration_since(last) < l.chrome_spacing
    {
        return Decision::Drop;
    }

    // The ceiling. Chrome drops on dry; content holds for the refill and fails
    // open past the hold ceiling.
    if l.ceiling_per_min > 0 && b.tokens < 1.0 {
        let need = Duration::from_secs_f64((1.0 - b.tokens) * 60.0 / l.ceiling_per_min as f64);
        if !surface.is_content() {
            return Decision::Drop;
        }
        if held + need > l.content_max_hold {
            tracing::warn!(
                "Slack: governor write ceiling held a content write past its ceiling, \
                 failing open (#2012)"
            );
            return Decision::FailOpen;
        }
        return Decision::Hold(need);
    }

    if l.ceiling_per_min > 0 {
        b.tokens -= 1.0;
    }
    if surface == Surface::Chrome {
        b.last_chrome = Some(now);
    }
    Decision::Admit
}

/// Park `until` if it is later than what is already armed, and return the
/// window that took effect measured from `now`. Latest-wins: a 429 naming 30 s
/// must not be shortened by an earlier one that named 2 s.
fn arm_park(b: &mut ChannelBudget, until: Instant, now: Instant) -> Duration {
    match b.pause_until {
        Some(existing) if existing >= until => existing.saturating_duration_since(now),
        _ => {
            b.pause_until = Some(until);
            until.saturating_duration_since(now)
        }
    }
}

/// Learn from a 429. `retry_after` is the window the RESPONSE named; `None`
/// means "we know it was a rate limit, we do not know how long" and the
/// fallback applies. Either way the number is clamped to
/// [`Limits::pause_ceiling`]: a window measured in hours parks the
/// conversation for the ceiling and then hands the problem back to Slack.
/// Returns the effective pause.
pub(crate) fn note_429(
    b: &mut ChannelBudget,
    l: &Limits,
    retry_after: Option<Duration>,
    now: Instant,
) -> Duration {
    let requested = retry_after.unwrap_or(l.pause_fallback);
    let clamped = requested.min(l.pause_ceiling);
    // Return what the conversation is ACTUALLY parked for, not what this one
    // response asked for.
    arm_park(b, now + clamped, now)
}

// ---------------------------------------------------------------------------
// The ledger
// ---------------------------------------------------------------------------

use std::sync::OnceLock;

use futures::FutureExt;
use futures::future::BoxFuture;
use slack_morphism::errors::SlackClientError;
use slack_morphism::prelude::{
    SlackApiChatPostMessageRequest, SlackApiChatPostMessageResponse, SlackApiChatUpdateRequest,
    SlackApiChatUpdateResponse, SlackClientHyperHttpsConnector, SlackClientSession, ValueStruct,
};

/// Per-conversation write ledger, one per process.
///
/// A static rather than a `SlackState` field because the writers here are not
/// methods on a handle: the flow ticker, `sync_step_group`,
/// `settle_step_group` and ~40 handler blocks are plain async closures that
/// hold a `SlackClient` and a token, not the state struct. Threading a
/// governor through all of them would be plumbing that carries no extra
/// information, since one process has one bot connection. The numbers are NOT
/// cached in the singleton: [`Limits::from_config`] is read on every gate, so
/// a hot-reloaded `[channels.slack.governor]` takes effect on the next write,
/// exactly like the Discord governor (#1910) and Telegram's (#1211).
#[derive(Debug, Default)]
pub(crate) struct Governor {
    /// One budget per conversation id (Slack channel ids are `C…`/`D…`
    /// strings, not integers like Discord's snowflakes). Created on first
    /// write; never evicted, the cardinality is one per channel the bot has
    /// ever spoken in.
    budgets: Mutex<HashMap<String, ChannelBudget>>,
}

/// How many times one `gate` call may loop before giving up. A hold ladder is
/// bounded by [`Limits::content_max_hold`] anyway (`admit` fails open past
/// it), so this only guards against a pathological zero-length sleep.
const MAX_HOLD_ROUNDS: usize = 512;

impl Governor {
    pub(crate) fn global() -> &'static Governor {
        static GLOBAL: OnceLock<Governor> = OnceLock::new();
        GLOBAL.get_or_init(Governor::default)
    }

    /// Pay the budget, then let the write go. Returns `true` when the caller
    /// should send; `false` only ever for [`Surface::Chrome`], whose caller
    /// drops the re-render. Content never comes back `false`: this layer is
    /// delay-never-drop, and past [`Limits::content_max_hold`] it fails open
    /// (warns in `admit`) rather than losing an answer.
    ///
    /// The lock is dropped before every `sleep`, so one parked conversation
    /// never blocks another conversation's gate.
    pub(crate) async fn gate(&self, conversation: &str, surface: Surface) -> bool {
        let limits = Limits::from_config();
        let start = now();
        for round in 0..MAX_HOLD_ROUNDS {
            // Clamp to `start`: the test clock leaps, and `held` must never
            // read as negative.
            let at = start.max(now());
            let held = at.saturating_duration_since(start);
            let decision = {
                let mut guard = match self.budgets.lock() {
                    Ok(guard) => guard,
                    Err(poisoned) => poisoned.into_inner(),
                };
                let budget = guard.entry(conversation.to_owned()).or_default();
                admit(budget, &limits, surface, held, at)
            };
            match decision {
                Decision::Admit | Decision::FailOpen => return true,
                Decision::Drop => {
                    tracing::debug!(
                        "Slack: governor dropped a chrome write on {conversation} (#2012)"
                    );
                    return false;
                }
                Decision::Hold(wait) => {
                    tracing::debug!(
                        "Slack: governor held a {surface:?} write on {conversation} for {wait:?} \
                         (round {round}, #2012)"
                    );
                    tokio::time::sleep(wait).await;
                }
            }
        }
        tracing::warn!(
            "Slack: governor held a write on {conversation} for {MAX_HOLD_ROUNDS} rounds, \
             sending anyway (#2012)"
        );
        true
    }

    /// Learn from a real 429: park the conversation for the window the
    /// response named, or for the fallback when it named none. Logged at warn
    /// with both numbers, because the difference between "Slack said 30s" and
    /// "we guessed" is the whole diagnostic value of this line (#2012).
    pub(crate) fn note_rate_limited(&self, conversation: &str, retry_after: Option<Duration>) {
        let limits = Limits::from_config();
        let pause = {
            let mut guard = match self.budgets.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            note_429(
                guard.entry(conversation.to_owned()).or_default(),
                &limits,
                retry_after,
                now(),
            )
        };
        tracing::warn!(
            conversation,
            pause_ms = pause.as_millis() as u64,
            named_ms = retry_after.map(|d| d.as_millis() as u64),
            "Slack: 429 on {conversation}, parking outbound writes for {} ms{}",
            pause.as_millis(),
            match retry_after {
                Some(d) => format!(" (API named {d:?}, clamped to the ceiling)"),
                None => " (response named no Retry-After, fallback window)".to_string(),
            }
        );
    }

    /// Slack's 429 in this client's error type. In slack-morphism 2.26 a rate
    /// limit is its own variant (`RateLimitError`), not an `ApiError` with a
    /// status code, and the connector fills `retry_after` from the
    /// `Retry-After` header. Anything else (channel_not_found, message_not_found,
    /// `invalid_blocks`) is not a rate limit and must never park a conversation.
    pub(crate) fn is_rate_limited(err: &SlackClientError) -> bool {
        matches!(err, SlackClientError::RateLimitError(_))
    }

    /// The window the response named, if it named one. A zero `Retry-After` is
    /// not a window: it would park the conversation for an instant and then
    /// the next write would hammer again, so it is read as "no window" and the
    /// fallback governs.
    pub(crate) fn retry_after(err: &SlackClientError) -> Option<Duration> {
        match err {
            SlackClientError::RateLimitError(e) => e.retry_after.filter(|d| !d.is_zero()),
            _ => None,
        }
    }
}

/// Park the conversation when and only when the failure is a 429. Every write
/// helper below runs this on its error path, which is why a call site never
/// has to remember it: learning the window is the point of having the layer.
pub(crate) fn note_if_429<T>(
    governor: &Governor,
    conversation: &str,
    result: &Result<T, SlackClientError>,
) {
    if let Err(err) = result
        && Governor::is_rate_limited(err)
    {
        governor.note_rate_limited(conversation, Governor::retry_after(err));
    }
}

// ---------------------------------------------------------------------------
// The seam the call sites use
// ---------------------------------------------------------------------------

/// Every outbound Slack chat write goes through these three methods, so the
/// policy cannot be skipped by forgetting to call it. They mirror the API
/// verbs they wrap: `session.chat_post_message(&req)` becomes
/// `session.post(&req)`, and a diff of the rewiring reads like the call it
/// replaced.
///
/// The futures are boxed and `Send` because these calls sit inside
/// `tokio::spawn` blocks; an `async fn` in a trait would not carry `Send`
/// through the return-position future, which is what makes the spawned turn
/// compile at all.
pub(crate) trait GatedWrites {
    /// A brand-new message: the answer, a notice, an approval prompt
    /// ([`Surface::Send`]).
    fn post<'a>(
        &'a self,
        req: &'a SlackApiChatPostMessageRequest,
    ) -> BoxFuture<'a, Result<SlackApiChatPostMessageResponse, SlackClientError>>;

    /// The last word on a message already posted: settle stamp, flipped
    /// waiting line, approval result label ([`Surface::Final`]).
    fn update<'a>(
        &'a self,
        req: &'a SlackApiChatUpdateRequest,
    ) -> BoxFuture<'a, Result<SlackApiChatUpdateResponse, SlackClientError>>;

    /// A live re-render of state already on screen ([`Surface::Chrome`]): the
    /// flow ticker's clock, the step-group status edit. `None` means the
    /// governor dropped this paint and the caller should change nothing else,
    /// because the next tick restates it; `Some(result)` is the API outcome,
    /// surfaced unchanged.
    fn update_chrome<'a>(
        &'a self,
        req: &'a SlackApiChatUpdateRequest,
    ) -> BoxFuture<'a, Option<Result<SlackApiChatUpdateResponse, SlackClientError>>>;
}

impl GatedWrites for SlackClientSession<'_, SlackClientHyperHttpsConnector> {
    fn post<'a>(
        &'a self,
        req: &'a SlackApiChatPostMessageRequest,
    ) -> BoxFuture<'a, Result<SlackApiChatPostMessageResponse, SlackClientError>> {
        let conversation = ValueStruct::value(&req.channel).to_owned();
        async move {
            let governor = Governor::global();
            // Delay-never-drop: this await can hold a turn's answer for a
            // budget window, and it will still send.
            governor.gate(&conversation, Surface::Send).await;
            let result = self.chat_post_message(req).await;
            note_if_429(governor, &conversation, &result);
            result
        }
        .boxed()
    }

    fn update<'a>(
        &'a self,
        req: &'a SlackApiChatUpdateRequest,
    ) -> BoxFuture<'a, Result<SlackApiChatUpdateResponse, SlackClientError>> {
        let conversation = ValueStruct::value(&req.channel).to_owned();
        async move {
            let governor = Governor::global();
            governor.gate(&conversation, Surface::Final).await;
            let result = self.chat_update(req).await;
            note_if_429(governor, &conversation, &result);
            result
        }
        .boxed()
    }

    fn update_chrome<'a>(
        &'a self,
        req: &'a SlackApiChatUpdateRequest,
    ) -> BoxFuture<'a, Option<Result<SlackApiChatUpdateResponse, SlackClientError>>> {
        let conversation = ValueStruct::value(&req.channel).to_owned();
        async move {
            let governor = Governor::global();
            if !governor.gate(&conversation, Surface::Chrome).await {
                return None;
            }
            let result = self.chat_update(req).await;
            note_if_429(governor, &conversation, &result);
            Some(result)
        }
        .boxed()
    }
}

// ---------------------------------------------------------------------------
// Test support (cfg(test)) - shared with src/tests/slack_governor_test.rs
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::sync::atomic::Ordering;

    /// Serialize every test that moves the virtual clock: it is one
    /// process-wide offset, so two clock-owning tests in flight would race.
    /// Async on purpose, the same as Discord's `governor::test_support`: a
    /// `std::sync` guard held across `gate().await` makes the test future
    /// non-`Send`.
    pub(crate) async fn registry_guard() -> tokio::sync::MutexGuard<'static, ()> {
        static GUARD: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        GUARD.lock().await
    }

    /// Zero the virtual clock offset.
    pub(crate) fn reset() {
        CLOCK_OFFSET_MS.store(0, Ordering::Relaxed);
    }

    /// Advance the virtual clock in milliseconds: refills, spacing windows and
    /// parks observe the jump on the next `now()` read. No test sleeps.
    pub(crate) fn advance(ms: u64) {
        CLOCK_OFFSET_MS.fetch_add(ms, Ordering::Relaxed);
    }
}
