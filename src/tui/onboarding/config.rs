use std::path::PathBuf;

use crate::config::Config;

use super::types::*;
use super::wizard::OnboardingWizard;

impl OnboardingWizard {
    /// Ensure config.toml and keys.toml exist in the workspace directory
    pub(super) fn ensure_config_files(&mut self) -> Result<(), String> {
        let workspace_path = std::path::PathBuf::from(&self.workspace_path);

        // Create workspace directory if it doesn't exist
        if !workspace_path.exists() {
            std::fs::create_dir_all(&workspace_path)
                .map_err(|e| format!("Failed to create workspace directory: {}", e))?;
        }

        let config_path = workspace_path.join("config.toml");
        let keys_path = workspace_path.join("keys.toml");

        // Create config.toml if it doesn't exist (copy from embedded example)
        if !config_path.exists() {
            let config_content = include_str!("../../../config.toml.example");
            std::fs::write(&config_path, config_content)
                .map_err(|e| format!("Failed to write config.toml: {}", e))?;
            tracing::info!("Created config.toml at {:?}", config_path);
        }

        // Create keys.toml if it doesn't exist (copy from embedded example)
        if !keys_path.exists() {
            let keys_content = include_str!("../../../keys.toml.example");
            std::fs::write(&keys_path, keys_content)
                .map_err(|e| format!("Failed to write keys.toml: {}", e))?;
            tracing::info!("Created keys.toml at {:?}", keys_path);
        }

        // Create usage_pricing.toml if it doesn't exist
        let pricing_path = workspace_path.join("usage_pricing.toml");
        if !pricing_path.exists() {
            let pricing_content = include_str!("../../../usage_pricing.toml.example");
            std::fs::write(&pricing_path, pricing_content)
                .map_err(|e| format!("Failed to write usage_pricing.toml: {}", e))?;
            tracing::info!("Created usage_pricing.toml at {:?}", pricing_path);
        }

        // Reload models for the selected provider from the newly created config
        self.reload_config_models();

        Ok(())
    }

    /// Initialize health check results
    pub fn start_health_check(&mut self) {
        let mut checks = vec![
            ("API Key Present".to_string(), HealthStatus::Pending),
            ("Config File".to_string(), HealthStatus::Pending),
            ("Workspace Directory".to_string(), HealthStatus::Pending),
            ("Template Files".to_string(), HealthStatus::Pending),
        ];

        // Add channel-specific checks for enabled channels
        if self.is_telegram_enabled() {
            checks.push(("Telegram Token".to_string(), HealthStatus::Pending));
            checks.push(("Telegram User ID".to_string(), HealthStatus::Pending));
        }
        if self.is_discord_enabled() {
            checks.push(("Discord Token".to_string(), HealthStatus::Pending));
            checks.push(("Discord Channel ID".to_string(), HealthStatus::Pending));
        }
        if self.is_slack_enabled() {
            checks.push(("Slack Bot Token".to_string(), HealthStatus::Pending));
            checks.push(("Slack Channel ID".to_string(), HealthStatus::Pending));
        }
        if self.is_whatsapp_enabled() {
            checks.push(("WhatsApp Connected".to_string(), HealthStatus::Pending));
        }
        if self.is_trello_enabled() {
            checks.push(("Trello API Key".to_string(), HealthStatus::Pending));
            checks.push(("Trello API Token".to_string(), HealthStatus::Pending));
            checks.push(("Trello Board ID".to_string(), HealthStatus::Pending));
        }

        self.health_results = checks;
        self.health_running = true;
        self.health_complete = false;

        // Run health checks synchronously (they're fast local checks)
        self.run_health_checks();
    }

    /// Execute all health checks
    fn run_health_checks(&mut self) {
        // Check 1: API key present
        self.health_results[0].1 = if !self.api_key_input.is_empty()
            || (self.is_custom_provider() && !self.custom_base_url.is_empty())
        {
            HealthStatus::Pass
        } else {
            HealthStatus::Fail("No API key provided".to_string())
        };

        // Check 2: Config path writable
        let config_path = crate::config::opencrabs_home().join("config.toml");
        self.health_results[1].1 = if let Some(parent) = config_path.parent() {
            if parent.exists() || std::fs::create_dir_all(parent).is_ok() {
                HealthStatus::Pass
            } else {
                HealthStatus::Fail(format!("Cannot create {}", parent.display()))
            }
        } else {
            HealthStatus::Fail("Invalid config path".to_string())
        };

        // Check 3: Workspace directory
        let workspace = PathBuf::from(&self.workspace_path);
        self.health_results[2].1 =
            if workspace.exists() || std::fs::create_dir_all(&workspace).is_ok() {
                HealthStatus::Pass
            } else {
                HealthStatus::Fail(format!("Cannot create {}", workspace.display()))
            };

        // Check 4: Template files available (they're compiled in, always present)
        self.health_results[3].1 = HealthStatus::Pass;

        // Channel checks (by name, since indices depend on which channels are enabled)
        for i in 0..self.health_results.len() {
            let name = self.health_results[i].0.clone();
            self.health_results[i].1 = match name.as_str() {
                "Telegram Token" => {
                    if !self.telegram_token_input.is_empty() {
                        HealthStatus::Pass
                    } else {
                        HealthStatus::Fail("No token provided".to_string())
                    }
                }
                "Telegram User ID" => {
                    if !self.telegram_user_id_input.is_empty() {
                        HealthStatus::Pass
                    } else {
                        HealthStatus::Fail("No user ID — bot won't know who to talk to".to_string())
                    }
                }
                "Discord Token" => {
                    if !self.discord_token_input.is_empty() {
                        HealthStatus::Pass
                    } else {
                        HealthStatus::Fail("No token provided".to_string())
                    }
                }
                "Discord Channel ID" => {
                    if !self.discord_channel_id_input.is_empty() {
                        HealthStatus::Pass
                    } else {
                        HealthStatus::Fail(
                            "No channel ID — bot won't know where to post".to_string(),
                        )
                    }
                }
                "Slack Bot Token" => {
                    if !self.slack_bot_token_input.is_empty() {
                        HealthStatus::Pass
                    } else {
                        HealthStatus::Fail("No bot token provided".to_string())
                    }
                }
                "Slack Channel ID" => {
                    if !self.slack_channel_id_input.is_empty() {
                        HealthStatus::Pass
                    } else {
                        HealthStatus::Fail(
                            "No channel ID — bot won't know where to post".to_string(),
                        )
                    }
                }
                "WhatsApp Connected" => {
                    if self.whatsapp_connected {
                        HealthStatus::Pass
                    } else {
                        HealthStatus::Fail("Not paired — scan QR code to connect".to_string())
                    }
                }
                "Trello API Key" => {
                    if !self.trello_api_key_input.is_empty() {
                        HealthStatus::Pass
                    } else {
                        HealthStatus::Fail("No API Key provided".to_string())
                    }
                }
                "Trello API Token" => {
                    if !self.trello_api_token_input.is_empty() {
                        HealthStatus::Pass
                    } else {
                        HealthStatus::Fail("No API Token provided".to_string())
                    }
                }
                "Trello Board ID" => {
                    if !self.trello_board_id_input.is_empty() {
                        HealthStatus::Pass
                    } else {
                        HealthStatus::Fail(
                            "No Board ID — agent won't know which board to poll".to_string(),
                        )
                    }
                }
                _ => continue, // Already set above
            };
        }

        self.health_running = false;
        self.health_complete = true;
    }

    /// Check if all health checks passed
    pub fn all_health_passed(&self) -> bool {
        self.health_complete
            && self
                .health_results
                .iter()
                .all(|(_, s)| matches!(s, HealthStatus::Pass))
    }

    /// Apply wizard configuration — creates config.toml, stores API key, seeds workspace
    /// Merges with existing config to preserve settings not modified in wizard
    pub fn apply_config(&self) -> Result<(), String> {
        // Groq key for STT/TTS
        let groq_key = if !self.groq_api_key_input.is_empty() && !self.has_existing_groq_key() {
            Some(self.groq_api_key_input.clone())
        } else {
            None
        };

        // Write config.toml via merge (write_key) — never overwrite entire file
        // Disable all providers first, then enable selected one
        let all_provider_sections = [
            "providers.anthropic",
            "providers.openai",
            "providers.gemini",
            "providers.openrouter",
            "providers.minimax",
        ];
        for section in &all_provider_sections {
            let _ = Config::write_key(section, "enabled", "false");
        }
        // Disable all custom providers
        if let Ok(config) = Config::load()
            && let Some(customs) = &config.providers.custom
        {
            for name in customs.keys() {
                let section = format!("providers.custom.{}", name);
                let _ = Config::write_key(&section, "enabled", "false");
            }
        }

        // Enable + configure the selected provider
        let custom_section;
        let section = match self.selected_provider {
            0 => "providers.anthropic",
            1 => "providers.openai",
            2 => "providers.gemini",
            3 => "providers.openrouter",
            4 => "providers.minimax",
            5 => {
                custom_section = format!("providers.custom.{}", self.custom_provider_name);
                &custom_section
            }
            _ => {
                custom_section = format!("providers.custom.{}", self.custom_provider_name);
                &custom_section
            }
        };
        let _ = Config::write_key(section, "enabled", "true");
        let model = self.selected_model_name().to_string();
        if !model.is_empty() {
            let _ = Config::write_key(section, "default_model", &model);
        }

        // Write base_url for providers that need it
        match self.selected_provider {
            3 => {
                let _ = Config::write_key(
                    section,
                    "base_url",
                    "https://openrouter.ai/api/v1/chat/completions",
                );
            }
            4 => {
                let _ = Config::write_key(section, "base_url", "https://api.minimax.io/v1");
            }
            5 => {
                if !self.custom_base_url.is_empty() {
                    let _ = Config::write_key(section, "base_url", &self.custom_base_url);
                }
                if !self.custom_model.is_empty() {
                    let _ = Config::write_key(section, "default_model", &self.custom_model);
                }
            }
            _ => {}
        }

        // Write models array for providers that have static model lists
        if !self.config_models.is_empty() && matches!(self.selected_provider, 4 | 5) {
            let _ = Config::write_array(section, "models", &self.config_models);
        }

        // Gateway config
        let _ = Config::write_key("gateway", "port", &self.gateway_port);
        let _ = Config::write_key("gateway", "bind", &self.gateway_bind);
        let _ = Config::write_key(
            "gateway",
            "auth_mode",
            if self.gateway_auth == 0 {
                "token"
            } else {
                "none"
            },
        );

        // Channel enabled flags (from channel_toggles: 0=Telegram, 1=Discord, 2=WhatsApp, 3=Slack)
        let _ = Config::write_key(
            "channels.telegram",
            "enabled",
            &self.is_telegram_enabled().to_string(),
        );
        let _ = Config::write_key(
            "channels.discord",
            "enabled",
            &self.is_discord_enabled().to_string(),
        );
        let _ = Config::write_key(
            "channels.whatsapp",
            "enabled",
            &self.channel_toggles.get(2).is_some_and(|t| t.1).to_string(),
        );
        let _ = Config::write_key(
            "channels.slack",
            "enabled",
            &self.is_slack_enabled().to_string(),
        );
        let _ = Config::write_key(
            "channels.trello",
            "enabled",
            &self.is_trello_enabled().to_string(),
        );

        // Voice config
        let groq_key_exists = !self.groq_api_key_input.is_empty() || self.has_existing_groq_key();
        let _ = Config::write_key("voice", "stt_enabled", &groq_key_exists.to_string());
        let _ = Config::write_key("voice", "tts_enabled", &self.tts_enabled.to_string());

        // STT provider
        if !self.groq_api_key_input.is_empty() || self.has_existing_groq_key() {
            let _ = Config::write_key("providers.stt.groq", "enabled", "true");
            let _ = Config::write_key(
                "providers.stt.groq",
                "default_model",
                "whisper-large-v3-turbo",
            );
        }

        // TTS provider
        if self.tts_enabled && groq_key_exists {
            let _ = Config::write_key("providers.tts.openai", "enabled", "true");
            let _ = Config::write_key("providers.tts.openai", "default_model", "gpt-4o-mini-tts");
        }

        // Save API key to keys.toml via merge — never overwrite
        if !self.has_existing_key()
            && !self.api_key_input.is_empty()
            && let Err(e) = crate::config::write_secret_key(section, "api_key", &self.api_key_input)
        {
            tracing::warn!("Failed to save API key to keys.toml: {}", e);
        }

        // Save STT/TTS keys to keys.toml
        if let Some(ref groq_key) = groq_key
            && let Err(e) =
                crate::config::write_secret_key("providers.stt.groq", "api_key", groq_key)
        {
            tracing::warn!("Failed to save Groq key to keys.toml: {}", e);
        }
        if self.tts_enabled
            && let Some(ref groq_key) = groq_key
            && let Err(e) =
                crate::config::write_secret_key("providers.tts.openai", "api_key", groq_key)
        {
            tracing::warn!("Failed to save TTS key to keys.toml: {}", e);
        }

        // Persist channel tokens to keys.toml (if new)
        if !self.telegram_token_input.is_empty()
            && !self.has_existing_telegram_token()
            && let Err(e) = crate::config::write_secret_key(
                "channels.telegram",
                "token",
                &self.telegram_token_input,
            )
        {
            tracing::warn!("Failed to save Telegram token to keys.toml: {}", e);
        }
        if !self.discord_token_input.is_empty()
            && !self.has_existing_discord_token()
            && let Err(e) = crate::config::write_secret_key(
                "channels.discord",
                "token",
                &self.discord_token_input,
            )
        {
            tracing::warn!("Failed to save Discord token to keys.toml: {}", e);
        }
        if !self.slack_bot_token_input.is_empty()
            && !self.has_existing_slack_bot_token()
            && let Err(e) = crate::config::write_secret_key(
                "channels.slack",
                "token",
                &self.slack_bot_token_input,
            )
        {
            tracing::warn!("Failed to save Slack bot token to keys.toml: {}", e);
        }
        if !self.slack_app_token_input.is_empty()
            && !self.has_existing_slack_app_token()
            && let Err(e) = crate::config::write_secret_key(
                "channels.slack",
                "app_token",
                &self.slack_app_token_input,
            )
        {
            tracing::warn!("Failed to save Slack app token to keys.toml: {}", e);
        }
        // Trello API Key (saved as app_token) + API Token
        if !self.trello_api_key_input.is_empty()
            && !self.has_existing_trello_api_key()
            && let Err(e) = crate::config::write_secret_key(
                "channels.trello",
                "app_token",
                &self.trello_api_key_input,
            )
        {
            tracing::warn!("Failed to save Trello API Key to keys.toml: {}", e);
        }
        if !self.trello_api_token_input.is_empty()
            && !self.has_existing_trello_api_token()
            && let Err(e) = crate::config::write_secret_key(
                "channels.trello",
                "token",
                &self.trello_api_token_input,
            )
        {
            tracing::warn!("Failed to save Trello API Token to keys.toml: {}", e);
        }

        // Persist channel IDs/user IDs to config.toml (if new)
        if !self.telegram_user_id_input.is_empty() && !self.has_existing_telegram_user_id() {
            let _ = Config::write_array(
                "channels.telegram",
                "allowed_users",
                std::slice::from_ref(&self.telegram_user_id_input),
            );
        }
        if !self.discord_channel_id_input.is_empty() && !self.has_existing_discord_channel_id() {
            let _ = Config::write_array(
                "channels.discord",
                "allowed_channels",
                std::slice::from_ref(&self.discord_channel_id_input),
            );
        }
        if !self.slack_channel_id_input.is_empty() && !self.has_existing_slack_channel_id() {
            let _ = Config::write_array(
                "channels.slack",
                "allowed_channels",
                std::slice::from_ref(&self.slack_channel_id_input),
            );
        }
        if !self.discord_allowed_list_input.is_empty() && !self.has_existing_discord_allowed_list()
        {
            let _ = Config::write_array(
                "channels.discord",
                "allowed_users",
                std::slice::from_ref(&self.discord_allowed_list_input),
            );
        }
        if !self.slack_allowed_list_input.is_empty() && !self.has_existing_slack_allowed_list() {
            let _ = Config::write_array(
                "channels.slack",
                "allowed_users",
                std::slice::from_ref(&self.slack_allowed_list_input),
            );
        }
        if !self.whatsapp_phone_input.is_empty() && !self.has_existing_whatsapp_phone() {
            let _ = Config::write_array(
                "channels.whatsapp",
                "allowed_phones",
                std::slice::from_ref(&self.whatsapp_phone_input),
            );
        }
        if !self.trello_board_id_input.is_empty() && !self.has_existing_trello_board_id() {
            let boards: Vec<String> = self
                .trello_board_id_input
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !boards.is_empty() {
                let _ = Config::write_array("channels.trello", "allowed_channels", &boards);
            }
        }
        if !self.trello_allowed_users_input.is_empty() && !self.has_existing_trello_allowed_users()
        {
            let users: Vec<String> = self
                .trello_allowed_users_input
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !users.is_empty() {
                let _ = Config::write_array("channels.trello", "allowed_users", &users);
            }
        }

        // Seed workspace templates (use AI-generated content when available)
        if self.seed_templates {
            let workspace = PathBuf::from(&self.workspace_path);
            std::fs::create_dir_all(&workspace)
                .map_err(|e| format!("Failed to create workspace: {}", e))?;

            for (filename, content) in TEMPLATE_FILES {
                let file_path = workspace.join(filename);
                // Use AI-generated content when available, static template as fallback
                let generated = match *filename {
                    "SOUL.md" => self.generated_soul.as_deref(),
                    "IDENTITY.md" => self.generated_identity.as_deref(),
                    "USER.md" => self.generated_user.as_deref(),
                    "AGENTS.md" => self.generated_agents.as_deref(),
                    "TOOLS.md" => self.generated_tools.as_deref(),
                    "MEMORY.md" => self.generated_memory.as_deref(),
                    _ => None,
                };
                // Write if: AI-generated (always overwrite) or file doesn't exist (seed template)
                if generated.is_some() || !file_path.exists() {
                    let final_content = generated.unwrap_or(content);
                    std::fs::write(&file_path, final_content)
                        .map_err(|e| format!("Failed to write {}: {}", filename, e))?;
                }
            }
        }

        // Install daemon if requested
        if self.install_daemon
            && let Err(e) = install_daemon_service()
        {
            tracing::warn!("Failed to install daemon: {}", e);
            // Non-fatal — don't block onboarding completion
        }

        Ok(())
    }
}

/// Install the appropriate daemon service for the current platform
fn install_daemon_service() -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        install_systemd_service()
    }

    #[cfg(target_os = "macos")]
    {
        install_launchagent()
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Err("Daemon installation not supported on this platform".to_string())
    }
}

#[cfg(target_os = "linux")]
fn install_systemd_service() -> Result<(), String> {
    let service_dir = dirs::config_dir()
        .ok_or("Cannot determine config dir")?
        .parent()
        .ok_or("Cannot determine parent of config dir")?
        .join(".config")
        .join("systemd")
        .join("user");

    // Try the standard XDG path first
    let service_dir = if service_dir.exists() {
        service_dir
    } else {
        dirs::home_dir()
            .ok_or("Cannot determine home dir")?
            .join(".config")
            .join("systemd")
            .join("user")
    };

    std::fs::create_dir_all(&service_dir)
        .map_err(|e| format!("Failed to create systemd dir: {}", e))?;

    let exe_path = std::env::current_exe().map_err(|e| format!("Failed to get exe path: {}", e))?;

    let service_content = format!(
        r#"[Unit]
Description=OpenCrabs AI Orchestration Agent
After=network.target

[Service]
Type=simple
ExecStart={}
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
"#,
        exe_path.display()
    );

    let service_path = service_dir.join("opencrabs.service");
    std::fs::write(&service_path, service_content)
        .map_err(|e| format!("Failed to write service file: {}", e))?;

    // Enable the service
    std::process::Command::new("systemctl")
        .args(["--user", "enable", "opencrabs"])
        .output()
        .map_err(|e| format!("Failed to enable service: {}", e))?;

    Ok(())
}

#[cfg(target_os = "macos")]
fn install_launchagent() -> Result<(), String> {
    let agents_dir = dirs::home_dir()
        .ok_or("Cannot determine home dir")?
        .join("Library")
        .join("LaunchAgents");

    std::fs::create_dir_all(&agents_dir)
        .map_err(|e| format!("Failed to create LaunchAgents dir: {}", e))?;

    let exe_path = std::env::current_exe().map_err(|e| format!("Failed to get exe path: {}", e))?;

    let plist_content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.opencrabs.agent</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
</dict>
</plist>
"#,
        exe_path.display()
    );

    let plist_path = agents_dir.join("com.opencrabs.agent.plist");
    std::fs::write(&plist_path, plist_content)
        .map_err(|e| format!("Failed to write plist: {}", e))?;

    std::process::Command::new("launchctl")
        .args(["load", &plist_path.to_string_lossy()])
        .output()
        .map_err(|e| format!("Failed to load launch agent: {}", e))?;

    Ok(())
}
