//! `/respond_to` on surfaces without per-channel config (#2013).
//!
//! The shared command handler writes the Telegram section, and Discord, Slack
//! and WhatsApp call it without a chat id, so a `/respond_to` typed there used
//! to change Telegram's global mode. Until per-channel config exists (#2014),
//! those surfaces answer from their own channel-level setting and never write.

use crate::config::RespondTo;

const RESTRICTED: &str = "🔒 `/respond_to` is restricted to the bot owner.";

/// Lowercase label for a mode, as the command and config spell it.
pub(crate) fn mode_label(mode: &RespondTo) -> &'static str {
    match mode {
        RespondTo::All => "all",
        RespondTo::DmOnly => "dm_only",
        RespondTo::Mention => "mention",
        RespondTo::Auto => "auto",
    }
}

/// The reply for a `/respond_to` typed on `platform`, or `None` when `text`
/// is not that command.
///
/// `mode` is the surface's channel-level `respond_to`; `None` for a surface
/// that has none (WhatsApp uses `response_policy`). Nothing is written here.
pub(crate) fn respond_to_outside_telegram(
    text: &str,
    is_owner: bool,
    platform: &str,
    mode: Option<&RespondTo>,
) -> Option<String> {
    let rest = text.trim().strip_prefix("/respond_to")?;
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    if !is_owner {
        return Some(RESTRICTED.to_string());
    }
    let key = platform.to_lowercase();
    let Some(mode) = mode else {
        return Some(format!(
            "ℹ️ {platform} has no respond mode: it is set by `response_policy` under `[channels.{key}]`."
        ));
    };
    if rest.trim().is_empty() {
        return Some(format!(
            "📢 Respond-to mode for {platform} (all channels): **{}**.\n\
             Set it in `[channels.{key}] respond_to`. Per-channel modes are tracked in #2014.",
            mode_label(mode)
        ));
    }
    Some(format!(
        "ℹ️ `/respond_to` does not change {platform} yet: per-channel switching is tracked in #2014. \
         Edit `[channels.{key}] respond_to` instead. Nothing was written."
    ))
}
