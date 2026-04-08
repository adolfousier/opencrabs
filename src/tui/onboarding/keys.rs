use super::types::*;
use super::wizard::OnboardingWizard;

impl OnboardingWizard {
    /// Detect existing Discord bot token from keys.toml
    pub(super) fn detect_existing_discord_token(&mut self) {
        if let Ok(config) = crate::config::Config::load()
            && config
                .channels
                .discord
                .token
                .as_ref()
                .is_some_and(|t| !t.is_empty())
        {
            self.discord_token_input = EXISTING_KEY_SENTINEL.to_string();
        }
    }

    /// Check if discord token holds a pre-existing value
    pub fn has_existing_discord_token(&self) -> bool {
        self.discord_token_input == EXISTING_KEY_SENTINEL
    }

    /// Detect existing Discord channel ID from config.toml
    pub(super) fn detect_existing_discord_channel_id(&mut self) {
        if let Ok(config) = crate::config::Config::load()
            && !config.channels.discord.allowed_channels.is_empty()
        {
            self.discord_channel_id_input = EXISTING_KEY_SENTINEL.to_string();
        }
    }

    /// Check if discord channel ID holds a pre-existing value
    pub fn has_existing_discord_channel_id(&self) -> bool {
        self.discord_channel_id_input == EXISTING_KEY_SENTINEL
    }

    /// Detect existing Discord allowed users from config.toml
    pub(super) fn detect_existing_discord_allowed_list(&mut self) {
        if let Ok(config) = crate::config::Config::load()
            && !config.channels.discord.allowed_users.is_empty()
        {
            self.discord_allowed_list_input = EXISTING_KEY_SENTINEL.to_string();
        }
    }

    /// Check if Discord allowed list holds a pre-existing value
    pub fn has_existing_discord_allowed_list(&self) -> bool {
        self.discord_allowed_list_input == EXISTING_KEY_SENTINEL
    }

    /// Detect existing Slack tokens from keys.toml
    pub(super) fn detect_existing_slack_tokens(&mut self) {
        if let Ok(config) = crate::config::Config::load() {
            if config
                .channels
                .slack
                .token
                .as_ref()
                .is_some_and(|t| !t.is_empty())
            {
                self.slack_bot_token_input = EXISTING_KEY_SENTINEL.to_string();
            }
            if config
                .channels
                .slack
                .app_token
                .as_ref()
                .is_some_and(|t| !t.is_empty())
            {
                self.slack_app_token_input = EXISTING_KEY_SENTINEL.to_string();
            }
        }
    }

    /// Check if slack bot token holds a pre-existing value
    pub fn has_existing_slack_bot_token(&self) -> bool {
        self.slack_bot_token_input == EXISTING_KEY_SENTINEL
    }

    /// Check if slack app token holds a pre-existing value
    pub fn has_existing_slack_app_token(&self) -> bool {
        self.slack_app_token_input == EXISTING_KEY_SENTINEL
    }

    /// Detect existing Slack channel ID from config.toml
    pub(super) fn detect_existing_slack_channel_id(&mut self) {
        if let Ok(config) = crate::config::Config::load()
            && !config.channels.slack.allowed_channels.is_empty()
        {
            self.slack_channel_id_input = EXISTING_KEY_SENTINEL.to_string();
        }
    }

    /// Check if slack channel ID holds a pre-existing value
    pub fn has_existing_slack_channel_id(&self) -> bool {
        self.slack_channel_id_input == EXISTING_KEY_SENTINEL
    }

    /// Detect existing Slack allowed IDs from config.toml
    pub(super) fn detect_existing_slack_allowed_list(&mut self) {
        if let Ok(config) = crate::config::Config::load()
            && !config.channels.slack.allowed_users.is_empty()
        {
            self.slack_allowed_list_input = EXISTING_KEY_SENTINEL.to_string();
        }
    }

    /// Check if Slack allowed list holds a pre-existing value
    pub fn has_existing_slack_allowed_list(&self) -> bool {
        self.slack_allowed_list_input == EXISTING_KEY_SENTINEL
    }

    /// Detect existing Telegram bot token from keys.toml
    pub(super) fn detect_existing_telegram_token(&mut self) {
        if let Ok(config) = crate::config::Config::load()
            && config
                .channels
                .telegram
                .token
                .as_ref()
                .is_some_and(|t| !t.is_empty())
        {
            self.telegram_token_input = EXISTING_KEY_SENTINEL.to_string();
        }
    }

    /// Check if telegram token holds a pre-existing value
    pub fn has_existing_telegram_token(&self) -> bool {
        self.telegram_token_input == EXISTING_KEY_SENTINEL
    }

    /// Load the first existing Telegram allowed_user from config.toml
    /// directly into the input field. Unlike secret fields, this is a
    /// plain numeric chat ID — no sentinel masking so the user can see
    /// and edit what's actually stored.
    pub(super) fn detect_existing_telegram_user_id(&mut self) {
        if let Ok(config) = crate::config::Config::load()
            && let Some(first) = config.channels.telegram.allowed_users.first()
        {
            self.telegram_user_id_input = first.clone();
        }
    }

    /// Telegram user IDs are never masked — this always returns false.
    /// Kept for API compatibility with the onboarding render/config code.
    pub fn has_existing_telegram_user_id(&self) -> bool {
        false
    }

    /// Detect existing WhatsApp allowed phones from config.toml
    pub(super) fn detect_existing_whatsapp_phone(&mut self) {
        if let Ok(config) = crate::config::Config::load()
            && !config.channels.whatsapp.allowed_phones.is_empty()
        {
            self.whatsapp_phone_input = EXISTING_KEY_SENTINEL.to_string();
        }
    }

    /// Check if WhatsApp phone holds a pre-existing value
    pub fn has_existing_whatsapp_phone(&self) -> bool {
        self.whatsapp_phone_input == EXISTING_KEY_SENTINEL
    }

    /// Detect existing Trello credentials (API Key, API Token, Board ID) from config/keys.toml
    pub(super) fn detect_existing_trello_credentials(&mut self) {
        if let Ok(config) = crate::config::Config::load() {
            if config
                .channels
                .trello
                .app_token
                .as_ref()
                .is_some_and(|t| !t.is_empty())
            {
                self.trello_api_key_input = EXISTING_KEY_SENTINEL.to_string();
            }
            if config
                .channels
                .trello
                .token
                .as_ref()
                .is_some_and(|t| !t.is_empty())
            {
                self.trello_api_token_input = EXISTING_KEY_SENTINEL.to_string();
            }
            if !config.channels.trello.board_ids.is_empty() {
                self.trello_board_id_input = EXISTING_KEY_SENTINEL.to_string();
            }
            if !config.channels.trello.allowed_users.is_empty() {
                self.trello_allowed_users_input = EXISTING_KEY_SENTINEL.to_string();
            }
        }
    }

    /// Check if Trello API Key holds a pre-existing value
    pub fn has_existing_trello_api_key(&self) -> bool {
        self.trello_api_key_input == EXISTING_KEY_SENTINEL
    }

    /// Check if Trello API Token holds a pre-existing value
    pub fn has_existing_trello_api_token(&self) -> bool {
        self.trello_api_token_input == EXISTING_KEY_SENTINEL
    }

    /// Check if Trello Board ID holds a pre-existing value
    pub fn has_existing_trello_board_id(&self) -> bool {
        self.trello_board_id_input == EXISTING_KEY_SENTINEL
    }

    /// Check if Trello Allowed Users holds a pre-existing value
    pub fn has_existing_trello_allowed_users(&self) -> bool {
        self.trello_allowed_users_input == EXISTING_KEY_SENTINEL
    }

    /// Detect existing respond_to values from config for all channels
    pub(super) fn detect_existing_respond_to(&mut self) {
        use crate::config::RespondTo;
        if let Ok(config) = crate::config::Config::load() {
            self.telegram_respond_to = match config.channels.telegram.respond_to {
                RespondTo::All => 0,
                RespondTo::DmOnly => 1,
                RespondTo::Mention => 2,
            };
            self.discord_respond_to = match config.channels.discord.respond_to {
                RespondTo::All => 0,
                RespondTo::DmOnly => 1,
                RespondTo::Mention => 2,
            };
            self.slack_respond_to = match config.channels.slack.respond_to {
                RespondTo::All => 0,
                RespondTo::DmOnly => 1,
                RespondTo::Mention => 2,
            };
        }
    }

    /// Detect existing image API key from keys.toml
    pub fn detect_existing_image_key(&mut self) {
        if let Ok(config) = crate::config::Config::load()
            && (config
                .image
                .generation
                .api_key
                .as_ref()
                .is_some_and(|k| !k.is_empty())
                || config
                    .image
                    .vision
                    .api_key
                    .as_ref()
                    .is_some_and(|k| !k.is_empty()))
        {
            self.image_api_key_input = EXISTING_KEY_SENTINEL.to_string();
        }
    }

    /// Check if image api key holds a pre-existing value
    pub fn has_existing_image_key(&self) -> bool {
        self.image_api_key_input == EXISTING_KEY_SENTINEL
    }

    /// Detect existing Groq API key from keys.toml
    pub fn detect_existing_groq_key(&mut self) {
        if let Ok(config) = crate::config::Config::load()
            && config
                .providers
                .stt
                .as_ref()
                .and_then(|s| s.groq.as_ref())
                .and_then(|p| p.api_key.as_ref())
                .is_some_and(|k| !k.is_empty())
        {
            self.groq_api_key_input = EXISTING_KEY_SENTINEL.to_string();
        }
    }

    /// Check if groq key holds a pre-existing value
    pub fn has_existing_groq_key(&self) -> bool {
        self.groq_api_key_input == EXISTING_KEY_SENTINEL
    }
}
