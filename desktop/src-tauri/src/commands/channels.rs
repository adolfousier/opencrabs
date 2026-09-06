use crate::AppState;
use serde::Serialize;
use tauri::State;

#[derive(Serialize)]
pub struct ChannelStatus {
    pub name: String,
    pub display_name: String,
    pub enabled: bool,
    pub alive: bool,
    pub error: Option<String>,
}

fn status_error(enabled: bool, configured: bool) -> Option<String> {
    if !enabled {
        Some("Disabled".to_string())
    } else if configured {
        None
    } else {
        Some("Missing credentials/config".to_string())
    }
}

#[tauri::command]
pub async fn get_channel_statuses(
    state: State<'_, AppState>,
) -> Result<Vec<ChannelStatus>, String> {
    let config = state.config.read().await;
    let channels = &config.channels;

    let items = vec![
        (
            "telegram",
            "Telegram Bot",
            channels.telegram.enabled,
            channels
                .telegram
                .token
                .as_ref()
                .is_some_and(|t| !t.trim().is_empty()),
            channels
                .telegram
                .token
                .as_ref()
                .is_some_and(|t| !t.trim().is_empty()),
        ),
        (
            "discord",
            "Discord Bot",
            channels.discord.enabled,
            channels
                .discord
                .token
                .as_ref()
                .is_some_and(|t| !t.trim().is_empty()),
            channels
                .discord
                .token
                .as_ref()
                .is_some_and(|t| !t.trim().is_empty()),
        ),
        (
            "whatsapp",
            "WhatsApp Bot",
            channels.whatsapp.enabled,
            false,
            false,
        ),
        (
            "slack",
            "Slack Bot",
            channels.slack.enabled,
            channels
                .slack
                .token
                .as_ref()
                .is_some_and(|t| !t.trim().is_empty())
                && channels
                    .slack
                    .app_token
                    .as_ref()
                    .is_some_and(|t| !t.trim().is_empty()),
            channels
                .slack
                .token
                .as_ref()
                .is_some_and(|t| !t.trim().is_empty())
                && channels
                    .slack
                    .app_token
                    .as_ref()
                    .is_some_and(|t| !t.trim().is_empty()),
        ),
        (
            "trello",
            "Trello Bot",
            channels.trello.enabled,
            channels
                .trello
                .token
                .as_ref()
                .is_some_and(|t| !t.trim().is_empty())
                && channels
                    .trello
                    .app_token
                    .as_ref()
                    .is_some_and(|k| !k.trim().is_empty()),
            channels
                .trello
                .token
                .as_ref()
                .is_some_and(|t| !t.trim().is_empty())
                && channels
                    .trello
                    .app_token
                    .as_ref()
                    .is_some_and(|k| !k.trim().is_empty()),
        ),
    ];

    Ok(items
        .into_iter()
        .map(
            |(name, display_name, enabled, configured, credentials_present)| ChannelStatus {
                name: name.to_string(),
                display_name: display_name.to_string(),
                enabled,
                // This is credential/config readiness, not a live socket probe.
                alive: enabled && credentials_present,
                error: status_error(enabled, configured),
            },
        )
        .collect())
}

#[tauri::command]
pub async fn toggle_channel(
    state: State<'_, AppState>,
    name: String,
    enabled: bool,
) -> Result<(), String> {
    let canonical = match name.as_str() {
        "telegram" | "discord" | "whatsapp" | "slack" | "trello" => name,
        _ => return Err(format!("Unknown channel: {}", name)),
    };
    let section = format!("channels.{}", canonical);
    opencrabs::config::Config::write_key(
        &section,
        "enabled",
        if enabled { "true" } else { "false" },
    )
    .map_err(|e| e.to_string())?;

    let mut config = state.config.write().await;
    match canonical.as_str() {
        "telegram" => config.channels.telegram.enabled = enabled,
        "discord" => config.channels.discord.enabled = enabled,
        "whatsapp" => config.channels.whatsapp.enabled = enabled,
        "slack" => config.channels.slack.enabled = enabled,
        "trello" => config.channels.trello.enabled = enabled,
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_channel_reports_disabled() {
        assert_eq!(status_error(false, false).as_deref(), Some("Disabled"));
    }

    #[test]
    fn enabled_but_unconfigured_channel_reports_missing_credentials() {
        assert_eq!(
            status_error(true, false).as_deref(),
            Some("Missing credentials/config")
        );
    }

    #[test]
    fn enabled_and_configured_channel_has_no_error() {
        assert_eq!(status_error(true, true), None);
    }
}
