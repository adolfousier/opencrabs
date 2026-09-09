//! Notify delivery policy (fork #146, DRY): the ONE home of the v2
//! delivery policy — mode resolution, post-route confirmation, sender-label
//! validation — shared by every consumer of `deliver_to_session`: the agent
//! `session_notify` tool, the A2A `session/notify` method (and through it
//! the CLI verb), and the cron scheduler arm. Consumers map the policy's
//! plain-`String` errors onto their own surfaces (tool → `ToolError`,
//! A2A → JSON-RPC INVALID_PARAMS, CLI → transport exit); the policy itself
//! stays surface-agnostic.

use std::time::Duration;

use serde_json::Value;

/// Resolved v2 delivery policy (fork #50).
#[derive(Debug)]
pub(crate) enum DeliveryMode {
    /// Refuse while the target is mid-turn (the failsafe default).
    Now,
    /// Queue for the target's next tool-loop boundary (alias: interrupt=true).
    TurnEnd,
    /// Defer until the target has been quiet for `quiet_for`; `max_delay`
    /// forces delivery into a busy turn (fork #43/#50).
    Quiet {
        quiet_for: Duration,
        max_delay: Duration,
    },
}

/// Confirmation budget for `confirm: true`: how long the sender watches the
/// receiving machinery for a wake before falling back to an honest
/// "routed, unconfirmed" verdict.
pub(crate) const CONFIRM_CAP: Duration = Duration::from_secs(10);

/// Cap for a sender label: the label rides inside the
/// `[session-notify from=cli:<label>]` header and the receipt-card summary
/// line, so a pathological value must not eat the preview budget.
pub(crate) const SENDER_LABEL_MAX_CHARS: usize = 64;

/// Validate a caller-supplied sender label (fork #146, one copy for the
/// A2A handler and the CLI verb): the label rides inside
/// `[session-notify from=cli:<label>]`, so it may not contain the closing
/// bracket or newlines, and it is capped to keep the receipt-card summary
/// readable. Empty handling (default vs error) stays with the caller —
/// the surfaces differ there by design.
pub(crate) fn validate_sender_label(label: &str) -> Result<(), String> {
    if label.contains(']') || label.contains('\n') || label.contains('\r') {
        return Err("sender label must not contain ']' or newlines".into());
    }
    if label.chars().count() > SENDER_LABEL_MAX_CHARS {
        return Err(format!(
            "sender label must be at most {SENDER_LABEL_MAX_CHARS} chars"
        ));
    }
    Ok(())
}

/// Post-route confirmation (owner-approved state-diag, 2026-09-01): watch
/// the receiving machinery for a bounded budget instead of reporting
/// "delivered" as a mere queue hand-off. The wake path is verifiable
/// in-process — a channel-registered turn probe flips to true the moment
/// the target's loop starts — so the sender gets `woke` (idle target
/// started a turn), `queued_pending_drain` (already mid-turn; the message
/// injects at its next tool-loop boundary), or an honest `delivered`
/// (routed, but no wake observed within `cap`).
pub(crate) async fn confirm_route(
    target: uuid::Uuid,
    cap: Duration,
) -> (&'static str, String, &'static str) {
    use crate::brain::agent::service::session_routes::turn_probe;
    let mid_turn = |t| turn_probe(t).is_some_and(|probe| probe());

    if mid_turn(target) {
        return (
            "queued_pending_drain",
            "Confirmed queued: the target is mid-turn; the message injects at its next \
             tool-loop boundary."
                .into(),
            "mid_turn",
        );
    }
    let deadline = tokio::time::Instant::now() + cap;
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if mid_turn(target) {
            return (
                "woke",
                "Confirmed end-to-end: the target was idle and has started a turn on the \
                 message."
                    .into(),
                "wake_confirmed",
            );
        }
    }
    (
        "delivered",
        format!(
            "Routed to session {target}, but no wake was observed within {}s — the \
             target may be parked (channel not claimed since boot) or slow to pick the \
             message up. Re-check via session_search before resending.",
            cap.as_secs()
        ),
        "unconfirmed",
    )
}

/// Resolve the v2 delivery policy against the deprecated `interrupt` alias
/// (fork #50). `interrupt=true` was always "queue for the in-flight turn's
/// next tool-loop boundary" — that is mode `turn-end`; unset/false was
/// "refuse while streaming" — mode `now`. Both may be passed only when they
/// agree; a disagreement is an error, never a silent precedence. `quiet`
/// defers until the target has been idle for `quiet_for_secs` (starvation
/// cap `max_delay_secs` forces delivery into a busy turn).
///
/// Errors are plain strings; each consumer frames them for its own surface.
pub(crate) fn resolve_mode(
    mode: Option<&str>,
    interrupt: Option<bool>,
    delivery: Option<&Value>,
) -> Result<DeliveryMode, String> {
    fn secs(parent: Option<&Value>, key: &str, default: u64) -> Result<Duration, String> {
        match parent.and_then(|d| d.get(key)) {
            None => Ok(Duration::from_secs(default)),
            Some(v) => {
                let n = v
                    .as_u64()
                    .ok_or_else(|| format!("delivery.{key} must be a non-negative integer"))?;
                Ok(Duration::from_secs(n))
            }
        }
    }
    let resolved = match mode {
        None => None,
        Some(known @ ("now" | "turn-end")) => Some(known),
        Some("quiet") => {
            // quiet contradicts interrupt=true by definition: quiet WAITS,
            // turn-end DERAILS. interrupt=false/unset is the natural form.
            if interrupt == Some(true) {
                return Err(
                    "delivery.mode 'quiet' and interrupt=true disagree — quiet defers, \
                     interrupt derails"
                        .into(),
                );
            }
            let quiet_for = secs(delivery, "quiet_for_secs", 60)?;
            let max_delay = secs(delivery, "max_delay_secs", 1800)?;
            return Ok(DeliveryMode::Quiet {
                quiet_for,
                max_delay,
            });
        }
        Some(other) => {
            return Err(format!(
                "delivery.mode '{other}' is not available yet — use 'now', 'turn-end' or 'quiet'"
            ));
        }
    };
    match (resolved, interrupt) {
        (Some("turn-end"), None | Some(true)) | (None, Some(true)) => Ok(DeliveryMode::TurnEnd),
        (Some("now"), None | Some(false)) | (None, None | Some(false)) => Ok(DeliveryMode::Now),
        _ => Err("delivery.mode and interrupt disagree — pass one, not both".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sender_label_rejects_framing_breakers_and_overlong() {
        assert!(validate_sender_label("ok label").is_ok());
        let brk = validate_sender_label("bad]label").unwrap_err();
        assert!(brk.contains("must not contain"), "got: {brk}");
        let nl = validate_sender_label("bad\nlabel").unwrap_err();
        assert!(nl.contains("must not contain"), "got: {nl}");
        let long =
            validate_sender_label("x".repeat(SENDER_LABEL_MAX_CHARS + 1).as_str()).unwrap_err();
        assert!(long.contains("at most"), "got: {long}");
        assert!(validate_sender_label("x".repeat(SENDER_LABEL_MAX_CHARS).as_str()).is_ok());
    }

    #[test]
    fn sender_label_cap_matches_the_a2a_constant() {
        // The A2A handler re-exports the cap for the CLI import path; the
        // two must never drift.
        assert_eq!(
            SENDER_LABEL_MAX_CHARS,
            crate::a2a::handler::notify::CLI_SENDER_LABEL_MAX_CHARS
        );
    }
}
