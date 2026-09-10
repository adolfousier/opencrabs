//! The shared [`DiscordState`] struct: fields, `new()` and `Default`.
//!
//! Every field is `pub(super)` so the per-concern impl modules beside this
//! file (`approval`, `cancel`, `connection`, `pending_interactions`,
//! `sessions`, `tool_group`) can reach them without widening the
//! crate-visible surface. Behaviour lives there, not here.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, oneshot};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{interactions, tool_group};

/// Shared Discord state for proactive messaging.
///
/// Set when the bot connects via the `ready` event.
/// Read by the `discord_send` tool to send messages on demand.
pub struct DiscordState {
    pub(super) http: Mutex<Option<Arc<serenity::http::Http>>>,
    /// Channel ID of the owner's last message — used as default for proactive sends
    pub(super) owner_channel_id: Mutex<Option<u64>>,
    /// Bot's own user ID — set on ready, used for @mention detection
    pub(super) bot_user_id: Mutex<Option<u64>>,
    /// Guild ID of the last guild message — needed for guild-scoped actions
    pub(super) guild_id: Mutex<Option<u64>>,
    /// Maps session_id → channel_id for approval routing
    pub(super) session_channels: Mutex<HashMap<Uuid, u64>>,
    /// Reverse ownership map (#148): channel_id → session_id, written in
    /// lockstep with `session_channels` at `register_session_channel` (the
    /// ONLY write site for both). Last writer wins — mirrors the forward map.
    pub(super) channel_sessions: Mutex<HashMap<u64, Uuid>>,
    /// Pending approval channels: approval_id → oneshot sender of (approved, always)
    pub(super) pending_approvals: Mutex<HashMap<String, oneshot::Sender<(bool, bool)>>>,
    /// Per-session cancel tokens for aborting in-flight agent tasks via /stop
    pub(super) cancel_tokens: Mutex<HashMap<Uuid, CancellationToken>>,
    /// Pending select menus: id -> (created, options) (#382). Lazy TTL:
    /// stale picks answer "expired" (#386).
    pub(super) pending_selects: Mutex<HashMap<String, (std::time::Instant, Vec<String>)>>,
    /// Pending modal forms: id -> (created, spec) (#383). Same lazy TTL.
    pub(super) pending_forms: Mutex<HashMap<String, (std::time::Instant, interactions::FormSpec)>>,
    /// Collapsible tool groups keyed by message id, so the Expand/Collapse
    /// interaction can re-render after the turn ended. Insertion-ordered
    /// for pruning; bounded at [`Self::TOOL_GROUP_CAP`] (see `tool_group`).
    pub(super) tool_groups: Mutex<(Vec<u64>, HashMap<u64, tool_group::GroupState>)>,
}

impl Default for DiscordState {
    fn default() -> Self {
        Self::new()
    }
}

impl DiscordState {
    pub fn new() -> Self {
        Self {
            http: Mutex::new(None),
            owner_channel_id: Mutex::new(None),
            bot_user_id: Mutex::new(None),
            guild_id: Mutex::new(None),
            session_channels: Mutex::new(HashMap::new()),
            channel_sessions: Mutex::new(HashMap::new()),
            pending_approvals: Mutex::new(HashMap::new()),
            cancel_tokens: Mutex::new(HashMap::new()),
            pending_selects: Mutex::new(HashMap::new()),
            pending_forms: Mutex::new(HashMap::new()),
            tool_groups: Mutex::new((Vec::new(), HashMap::new())),
        }
    }
}
