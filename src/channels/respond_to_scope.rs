//! `/respond_to` on surfaces without per-channel config (#2013).
//!
//! The shared command handler writes the Telegram section, and Discord, Slack
//! and WhatsApp call it without a chat id, so a `/respond_to` typed there used
//! to change Telegram's global mode. Until per-channel config exists (#2014),
//! those surfaces answer from their own channel-level setting and never write.

use crate::config::RespondTo;

/// Parse a mode argument. Accepts the spellings `/respond_to` documents.
pub(crate) fn parse_mode(arg: &str) -> Option<RespondTo> {
    match arg.to_lowercase().as_str() {
        "all" => Some(RespondTo::All),
        "mention" | "mentions" => Some(RespondTo::Mention),
        "auto" => Some(RespondTo::Auto),
        "dm_only" => Some(RespondTo::DmOnly),
        _ => None,
    }
}

/// `/respond_to` in a Discord channel or thread (#2014). A bare command shows
/// the mode that applies here; with a mode it writes this channel's own entry
/// through `write(channel_id, mode_label)`. The writer is injected so the
/// decision stays pure. A failed write is reported, never swallowed.
pub(crate) fn respond_to_discord_channel(
    text: &str,
    is_owner: bool,
    channel_id: &str,
    effective: &RespondTo,
    write: impl FnOnce(&str, &str) -> Result<(), String>,
) -> Option<String> {
    let rest = text.trim().strip_prefix("/respond_to")?;
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    if !is_owner {
        return Some(RESTRICTED.to_string());
    }
    let arg = rest.trim();
    if arg.is_empty() {
        return Some(format!(
            "📢 Respond-to mode for this Discord channel: **{}**.\n\
             Switch it with `/respond_to <all|dm_only|mention|auto>`, or set `[channels.discord.channels.{channel_id}] respond_to`.",
            mode_label(effective)
        ));
    }
    let Some(mode) = parse_mode(arg) else {
        return Some(format!(
            "❌ Unknown mode \"{arg}\". Use: `/respond_to <all|dm_only|mention|auto>`"
        ));
    };
    let label = mode_label(&mode);
    match write(channel_id, label) {
        Ok(()) => Some(format!(
            "✅ Respond-to mode for this Discord channel switched to **{label}**."
        )),
        Err(e) => Some(format!("❌ Could not save the respond mode: {e}")),
    }
}

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
