//! The shared [`SlackState`] struct: fields, `new()` and `Default`.
//!
//! Every field is `pub(super)` so the per-concern impl modules beside this
//! file (`approval`, `cancel`, `connection`, `followups`, `sessions`,
//! `tool_group`) can reach them without widening the crate-visible
//! surface. Behaviour lives there, not here.

use slack_morphism::prelude::SlackHyperClient;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, oneshot};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::tool_group;

/// Shared Slack state for proactive messaging.
///
/// Set when the bot connects via Socket Mode.
/// Read by the `slack_send` tool to send messages on demand.
pub struct SlackState {
    pub(super) client: Mutex<Option<Arc<SlackHyperClient>>>,
    pub(super) bot_token: Mutex<Option<String>>,
    /// Channel ID of the owner's last message — used as default for proactive sends
    pub(super) owner_channel_id: Mutex<Option<String>>,
    /// Maps session_id → channel_id for approval routing
    pub(super) session_channels: Mutex<HashMap<Uuid, String>>,
    /// Reverse ownership map (#148): channel_id → session_id, written in
    /// lockstep with `session_channels` at `register_session_channel` (the
    /// ONLY write site for both). Last writer wins — mirrors the forward map.
    pub(super) channel_sessions: Mutex<HashMap<String, Uuid>>,
    /// Pending approval channels: approval_id → oneshot sender of (approved, always)
    pub(super) pending_approvals: Mutex<HashMap<String, oneshot::Sender<(bool, bool)>>>,
    /// Pending follow-up questions: question_id → (oneshot sender,
    /// options). Same shape as the other channels — action_id only
    /// carries the option index, the click handler maps it back via
    /// the stored options list.
    /// Per-session OPTIONAL follow-up suggestions from `suggest_options`
    /// (#599). Non-blocking: buttons ride under the response and a tap injects
    /// the chosen suggestion as a new turn. Keyed by session; the tap handler
    /// resolves `idx -> text`. Cleared on tap or when the user sends anything.
    pub(super) pending_followups: Mutex<HashMap<Uuid, Vec<String>>>,
    /// Per-session cancel tokens for aborting in-flight agent tasks via /stop
    pub(super) cancel_tokens: Mutex<HashMap<Uuid, CancellationToken>>,
    /// Collapsible tool groups keyed by their message ts, so the Expand /
    /// Collapse interaction can re-render long after the turn ended.
    /// Insertion-ordered for pruning; bounded at [`Self::TOOL_GROUP_CAP`] (see `tool_group`).
    pub(super) tool_groups: Mutex<(Vec<String>, HashMap<String, tool_group::GroupState>)>,
}

impl Default for SlackState {
    fn default() -> Self {
        Self::new()
    }
}

impl SlackState {
    pub fn new() -> Self {
        Self {
            client: Mutex::new(None),
            bot_token: Mutex::new(None),
            owner_channel_id: Mutex::new(None),
            session_channels: Mutex::new(HashMap::new()),
            channel_sessions: Mutex::new(HashMap::new()),
            pending_approvals: Mutex::new(HashMap::new()),
            pending_followups: Mutex::new(HashMap::new()),
            cancel_tokens: Mutex::new(HashMap::new()),
            tool_groups: Mutex::new((Vec::new(), HashMap::new())),
        }
    }
}
