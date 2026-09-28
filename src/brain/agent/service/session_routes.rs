//! Where a background-task completion is delivered (#940, #1036).
//!
//! Delivery is keyed by SESSION, never by whichever surface happened to run
//! the command. Every surface builds its own manager from its own enqueue
//! callback, so routing by the executing service sent a channel-bound session
//! driven from the TUI back to the TUI, and the channel that asked for the
//! work never heard the answer.
//!
//! Split out of the task manager because none of it knows anything about
//! tasks: it is a session-to-callback registry that spawning, sub-agents and
//! restart recovery all consult. Keeping it next to spawning meant three
//! unrelated concerns shared one file and one set of locks to reason about.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use uuid::Uuid;

use super::types::{MessageEnqueueCallback, QueuedUserMessage};

/// Where a session's background-task completion must be delivered, keyed by
/// session rather than by whichever surface happened to run the command.
///
/// Every surface builds its own `BackgroundTaskManager` from its own enqueue
/// callback, so the completion used to follow the *executing* service. A
/// channel-bound session driven from the TUI therefore reported back to the
/// TUI, and the channel that started the work never heard the answer (#940).
/// A channel registers its session here when it binds one; the manager
/// consults this first and only falls back to its own callback when nothing
/// claims the session (a genuinely TUI-local or CLI-local session).
static SESSION_ROUTES: Mutex<Option<HashMap<Uuid, MessageEnqueueCallback>>> = Mutex::new(None);

/// Bind `session_id`'s background-task completions to `enqueue`.
///
/// Idempotent: re-binding the same session replaces the route, which is what
/// a reconnect or a bot restart needs.
pub fn register_session_route(session_id: Uuid, enqueue: MessageEnqueueCallback) {
    match SESSION_ROUTES.lock() {
        Ok(mut guard) => {
            guard
                .get_or_insert_with(HashMap::new)
                .insert(session_id, enqueue.clone());
            // Startup recovery runs before any channel connects, so this
            // session may already have reports waiting for someone to claim
            // it. Hand them over now that there is somewhere to send them
            // (#1037). Done after the insert so the route is live first.
            super::restart_recovery::claim_session(session_id, &enqueue);
        }
        Err(e) => {
            // Worth saying out loud: without the route this session's next
            // background completion silently goes to the wrong surface.
            tracing::error!(
                target: "background_task",
                "Could not register resume route for session {session_id}: {e}"
            );
        }
    }
}

/// Who should receive `session_id`'s completion: the surface that claimed the
/// session, falling back to `executing` when nothing did.
///
/// The whole fix in one line — pick by session, never by who ran the command —
/// so it is a pure function and directly testable. Test-only since fork #19:
/// production delivery now flows through [`deliver_to_session`], but the
/// pre-#19 routing suites still exercise these semantics directly (#22).
#[cfg(test)]
pub fn resolve_route(
    session_id: Uuid,
    executing: &MessageEnqueueCallback,
) -> MessageEnqueueCallback {
    if let Some(route) = session_route(session_id) {
        return route;
    }
    // A session recovery revived from a channel is NOT local, however it
    // looks from here: it simply has not been claimed yet, because recovery
    // bypasses the ingress handlers where claiming happens. Falling back to
    // the executing surface delivered its completion into a daemon's unread
    // TuiEvent channel, and under the TUI into the wrong window entirely
    // (#1206). Park it for the channel that owns it instead.
    if super::restart_recovery::awaits_channel_route(session_id) {
        return super::restart_recovery::parking_route();
    }
    executing.clone()
}

/// The surface this process booted on, used when no channel claims a session.
///
/// `spawn_command` carries the executing service's callback on the manager, so
/// it always has a fallback. A sub-agent has no such handle — it is reached
/// from a tool with no service context — so the local surface is registered
/// once at startup and resolved on demand instead (#1036).
static LOCAL_ROUTE: Mutex<Option<MessageEnqueueCallback>> = Mutex::new(None);

/// Record the booting surface as the fallback destination. Called once per
/// process start; re-registering replaces it.
pub fn register_local_route(enqueue: MessageEnqueueCallback) {
    match LOCAL_ROUTE.lock() {
        Ok(mut guard) => *guard = Some(enqueue),
        Err(e) => {
            // Without it, a sub-agent finishing on a session no channel owns
            // has nowhere to report and its output is dropped.
            tracing::error!(
                target: "background_task",
                "Could not register the local delivery route: {e}"
            );
        }
    }
}

/// Is `session_id` mid-turn right now?
///
/// Channels that expose turn state (telegram's #501/#845 gate) register a
/// probe beside their delivery route; surfaces without one simply don't,
/// which the gate below reads as "unknown" and fails open — a headless
/// session must stay notifyable. The probe captures an `Arc` of the channel
/// state, so it stays valid across route re-binds and reconnects.
pub type TurnProbe = Arc<dyn Fn() -> bool + Send + Sync>;

/// Per-session in-flight probes, keyed like [`SESSION_ROUTES`].
static TURN_PROBES: Mutex<Option<HashMap<Uuid, TurnProbe>>> = Mutex::new(None);

/// Register (or replace) the in-flight probe for `session_id`.
pub fn register_turn_probe(session_id: Uuid, probe: TurnProbe) {
    match TURN_PROBES.lock() {
        Ok(mut guard) => {
            guard
                .get_or_insert_with(HashMap::new)
                .insert(session_id, probe);
        }
        Err(e) => {
            tracing::error!(
                target: "background_task",
                "Could not register the in-flight probe for session {session_id}: {e}"
            );
        }
    }
}

/// The session's in-flight probe, if its channel registered one.
pub(crate) fn turn_probe(session_id: Uuid) -> Option<TurnProbe> {
    match TURN_PROBES.lock() {
        Ok(guard) => guard.as_ref()?.get(&session_id).cloned(),
        Err(e) => {
            tracing::error!(
                target: "background_task",
                "Could not read the in-flight probe for session {session_id}: {e}"
            );
            None
        }
    }
}

/// Does the session still own the channel it was bound to?
///
/// A channel session can be REPLACED on its chat/topic — the idle-timeout
/// reset archives the old session and creates a successor for the same
/// topic — while its delivery route stays registered (routes are keyed by
/// session UUID and never evicted). Without a gate, any push then wakes the
/// replaced session and it posts into a conversation a different session now
/// owns (fork #17). Channels that track ownership register a probe beside
/// their delivery route; surfaces without one simply don't, which the gate
/// below reads as "unknown" and fails open — same posture as the in-flight
/// probe: the gate must never make a surface unnotifyable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelOwnership {
    /// The session's bound chat/topic still resolves back to it.
    Owned,
    /// A DIFFERENT session now occupies the bound chat/topic.
    Occupied { occupant: Uuid },
    /// No binding recorded (or the channel does not track ownership).
    Unknown,
}

pub type ChannelOwnerProbe = Arc<dyn Fn() -> ChannelOwnership + Send + Sync>;

/// Per-session channel-ownership probes, keyed like [`SESSION_ROUTES`].
static CHANNEL_OWNER_PROBES: Mutex<Option<HashMap<Uuid, ChannelOwnerProbe>>> = Mutex::new(None);

/// Register (or replace) the channel-ownership probe for `session_id`.
pub fn register_channel_owner_probe(session_id: Uuid, probe: ChannelOwnerProbe) {
    match CHANNEL_OWNER_PROBES.lock() {
        Ok(mut guard) => {
            guard
                .get_or_insert_with(HashMap::new)
                .insert(session_id, probe);
        }
        Err(e) => {
            tracing::error!(
                target: "background_task",
                "Could not register the channel-ownership probe for session {session_id}: {e}"
            );
        }
    }
}

/// The session's channel-ownership probe, if its channel registered one.
fn channel_owner_probe(session_id: Uuid) -> Option<ChannelOwnerProbe> {
    match CHANNEL_OWNER_PROBES.lock() {
        Ok(guard) => guard.as_ref()?.get(&session_id).cloned(),
        Err(e) => {
            tracing::error!(
                target: "background_task",
                "Could not read the channel-ownership probe for session {session_id}: {e}"
            );
            None
        }
    }
}

/// True when any channel registered an ownership probe for `session_id` and
/// that probe no longer reads `Unknown` (#1773, per-session channel
/// awareness for the capabilities preamble). A failed probe read counts as
/// NOT bound: a missing capabilities block is cosmetic, a false one is not.
pub(super) fn session_is_channel_bound(session_id: Uuid) -> bool {
    match channel_owner_probe(session_id) {
        Some(probe) => !matches!(probe(), ChannelOwnership::Unknown),
        None => false,
    }
}

/// What happened to a message handed to [`deliver_to_session`].
///
/// A bare bool used to be enough, because the only two outcomes were "went
/// out" and "nothing could take it". Parking added a third (#1206), and it
/// reads as failure to a bool caller even though the message is safe and
/// will leave on the next claim. `session_notify` reported exactly that as
/// "no live route for this session", which is the opposite of what happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// Handed to the surface that owns the session, or to the local one.
    Delivered,
    /// Held for a channel that has not claimed the session yet. Not lost.
    Parked,
    /// Nothing can receive it and nothing is holding it.
    NoRoute,
    /// The target is mid-turn and the caller did not ask to interrupt
    /// (fork #13). Nothing was delivered; the sender retries when the
    /// target goes idle, or resends with `interrupt` set. When the
    /// refusal follows a redirect (fork #19), `redirected_to` names the
    /// occupant that is mid-turn — the message WOULD have gone there, and
    /// will once it goes idle or `interrupt` is set.
    RefusedInFlight { redirected_to: Option<Uuid> },
    /// The target no longer owns its channel — a different session now
    /// occupies the chat/topic it was bound to (fork #17). Instead of
    /// refusing, the message was REDIRECTED to the occupant `to`, with a
    /// provenance frame so the successor does not adopt a dead session's
    /// task as its own (fork #19).
    Redirected { to: Uuid },
}

/// Deliver `msg` to whoever owns `session_id`, falling back to the booting
/// surface, parking it when a channel owns the session but has not claimed it.
///
/// `interrupt` is the failsafe valve (fork #13): a push into a streaming turn
/// arrives in the receiver's context as a bare user message — receivers
/// task-switch or skim past it. Unless the sender explicitly asked to
/// interrupt, delivery is refused while the target is mid-turn. Unknown turn
/// state (no probe registered) delivers: the gate must never make a surface
/// unnotifyable.
///
/// The channel-ownership gate runs OUTERMOST (fork #17): it guards WHO owns
/// the conversation, not the target's turn state, so `interrupt` does not
/// override it. Unknown ownership (no probe, or no binding recorded) fails
/// open, same posture as the turn gate.
///
/// Occupied no longer REFUSES (fork #19): the replaced session's message is
/// nobody's fault, and retrying a session that no longer owns its channel
/// never fixes it. Instead the message is REDIRECTED to the occupant — the
/// session that owns the channel now — with a provenance frame prefixed to
/// its context text so the successor does not mistake it for its own task.
/// The redirect follows the occupant chain up to a hop cap (cycle insurance:
/// an occupant can itself be replaced mid-flight) and parks past the cap —
/// held, never dropped. The original `interrupt` flag is then honored
/// against the FINAL target: a redirect landing on a mid-turn occupant is
/// still gated by the sender's original intent.
pub fn deliver_to_session(session_id: Uuid, msg: QueuedUserMessage, interrupt: bool) -> Delivery {
    // Channel-ownership gate, outermost (fork #17). Occupied → redirect to
    // the occupant (fork #19), with provenance framing once per chain and a
    // hop cap so a replacing chain (A replaced B replaced C…) cannot loop.
    const REDIRECT_HOP_CAP: usize = 3;
    let mut target = session_id;
    let mut hops: usize = 0;
    let mut msg = msg;
    loop {
        if let Some(probe) = channel_owner_probe(target)
            && let ChannelOwnership::Occupied { occupant } = probe()
        {
            if hops >= REDIRECT_HOP_CAP {
                // Cycle insurance exhausted: park rather than loop. The
                // message is safe and leaves when a channel claims it.
                tracing::warn!(
                    target: "background_task",
                    "Redirect cap hit for session {session_id}: its channel-owner chain \
                     (ending at session {occupant}) did not stabilise after {hops} hops; \
                     parking instead of delivering"
                );
                super::restart_recovery::parking_route()(target, msg);
                return Delivery::Parked;
            }
            hops += 1;
            tracing::info!(
                target: "background_task",
                "Redirecting delivery for session {target}: its channel is occupied by \
                 session {occupant} (hop {hops}); delivering to the new owner"
            );
            if hops == 1 {
                // Provenance framing (#19): the successor must know this
                // message was written for the session that lost the
                // channel, not for itself. Framed once, against the ORIGINAL
                // target, so a multi-hop chain does not stack frames.
                msg.context_text = format!(
                    "[redirected — originally for session {session_id}, which no longer \
                     owns this channel] {}",
                    msg.context_text
                );
            }
            target = occupant;
            continue;
        }
        break;
    }
    // Mid-turn gate (fork #13), honored against the FINAL target: a redirect
    // can land on an occupant that is mid-turn, and the sender's original
    // `interrupt` flag travels with the message. Refuse with the redirect
    // context attached so the sender hears where the message WOULD have gone.
    if !interrupt && turn_probe(target).is_some_and(|probe| probe()) {
        tracing::info!(
            target: "background_task",
            "Refusing delivery to session {target}: mid-turn and interrupt not set"
        );
        return Delivery::RefusedInFlight {
            redirected_to: (hops > 0).then_some(target),
        };
    }
    if let Some(route) = session_route(target) {
        route(target, msg);
        return if hops > 0 {
            Delivery::Redirected { to: target }
        } else {
            Delivery::Delivered
        };
    }
    // Same reasoning as `resolve_route`: an unclaimed session that recovery
    // knows came from a channel must wait for that channel, not be handed to
    // whatever this process booted on (#1206).
    if super::restart_recovery::awaits_channel_route(target) {
        super::restart_recovery::parking_route()(target, msg);
        return Delivery::Parked;
    }
    let local = match LOCAL_ROUTE.lock() {
        Ok(guard) => guard.clone(),
        Err(e) => {
            tracing::error!(
                target: "background_task",
                "Could not read the local delivery route for session {target}: {e}"
            );
            None
        }
    };
    match local {
        Some(route) => {
            route(target, msg);
            if hops > 0 {
                Delivery::Redirected { to: target }
            } else {
                Delivery::Delivered
            }
        }
        None => {
            tracing::error!(
                target: "background_task",
                "Nothing can receive a message for session {target}; it is dropped: {}",
                msg.display_text
            );
            Delivery::NoRoute
        }
    }
}

/// The surface that owns `session_id`'s completions, if one claimed it.
pub fn session_route(session_id: Uuid) -> Option<MessageEnqueueCallback> {
    match SESSION_ROUTES.lock() {
        Ok(guard) => guard.as_ref()?.get(&session_id).cloned(),
        Err(e) => {
            tracing::error!(
                target: "background_task",
                "Could not read resume route for session {session_id}: {e}"
            );
            None
        }
    }
}

/// Claim `session_id`'s background-task completions for a channel.
///
/// Every channel handler did this inline, with four copies of the same
/// rationale comment above four copies of the same `if let`. `enqueue` is
/// `None` on a surface with no enqueue callback wired, which is simply nothing
/// to claim with. Idempotent, so calling it on every inbound message is free.
pub fn claim_for_channel(session_id: Uuid, enqueue: Option<MessageEnqueueCallback>) {
    if let Some(enqueue) = enqueue {
        register_session_route(session_id, enqueue);
    }
}
