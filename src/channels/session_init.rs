//! Channel session initialization
//!
//! When a messaging channel (Discord, Telegram, Slack, WhatsApp, Trello) needs
//! a new session — a fresh DM, a new group thread, a per-channel session — it
//! used to call `session_svc.create_session(title)` directly, which stamped
//! `provider_name = NULL` / `model = NULL`. Downstream, `sync_provider_for_session`
//! saw `None` and fell through to `config.active_provider_and_model()` — the
//! fixed priority list in config.toml — which does NOT reflect whatever the TUI
//! user had actively selected (TUI persists provider changes per-session, not
//! into config.toml).
//!
//! Net effect: a user who picked OpenRouter in the TUI would open Discord and
//! find the bot pinned to whatever provider happened to be first-enabled in
//! config.toml, not the one they were actually using.
//!
//! `create_channel_session` closes that gap: it creates the session and stamps
//! it with the most recent existing session's provider/model (falling back to
//! `None` if no such session exists, which then falls through to the config
//! priority list — the previous behavior). Call this instead of
//! `session_svc.create_session(...)` in every channel handler.

use anyhow::Result;

use crate::db::models::Session;
use crate::services::SessionService;

/// Create a new channel session, inheriting provider + model from the most
/// recent existing session (TUI or any channel) when available.
///
/// Returns the created `Session`. Provider inheritance is best-effort: if the
/// most-recent-session lookup fails for any reason, the session is still
/// created with `provider_name = None` / `model = None`, matching the old
/// behavior so callers never fail because of this helper.
pub async fn create_channel_session(
    session_svc: &SessionService,
    title: Option<String>,
) -> Result<Session> {
    let (inherited_provider, inherited_model) = match session_svc.get_most_recent_session().await {
        Ok(Some(prev)) => (prev.provider_name, prev.model),
        _ => (None, None),
    };

    if inherited_provider.is_some() {
        tracing::info!(
            "Channel session inherited provider {:?} / model {:?} from most recent session",
            inherited_provider,
            inherited_model,
        );
    }

    session_svc
        .create_session_with_provider(title, inherited_provider, inherited_model)
        .await
}
