//! Channel-capable `/onboard:*` handlers.
//!
//! The TUI onboarding wizard is an interactive screen, so on channels
//! (Telegram/Discord/Slack/WhatsApp) the `/onboard:*` steps were unreachable.
//! These text-driven handlers expose the same setup over a chat: called with
//! no args they print a menu; called with args they write the exact same
//! config/keys the wizard writes. Routed from `slash_command` via the
//! `/onboard:<step>` prefix.
//!
//! `provider` is intentionally absent — `/models` already covers it on
//! channels. `brain` and the brain-directory change stay TUI-only.

use super::error::Result;
use super::r#trait::ToolResult;
use crate::config::Config;

/// Route `/onboard:<sub>` to the matching handler.
pub(crate) fn dispatch(sub: &str, args: &str) -> Result<ToolResult> {
    match sub.trim().to_lowercase().as_str() {
        "image" => onboard_image(args),
        "voice" => onboard_voice(args),
        "channels" => onboard_channels(args),
        "brain" => Ok(ToolResult::success(
            "Brain/persona setup edits multiple markdown files and is TUI-only \
             (type /onboard:brain in the desktop app). To tweak persona text from \
             here, read/edit the brain files via the brain tools."
                .into(),
        )),
        other => Ok(ToolResult::error(format!(
            "Unknown onboarding step '{other}'. Available on channels: image, voice, channels."
        ))),
    }
}

/// Map the active provider id (`anthropic`, `custom:foo`, …) to its config
/// section (`providers.anthropic`, `providers.custom.foo`). `None` when no
/// provider is configured.
fn active_provider_section(config: &Config) -> Option<String> {
    let (id, _model) = config.providers.active_provider_and_model();
    match id.as_str() {
        "none" => None,
        s => match s.strip_prefix("custom:") {
            Some(name) => Some(format!("providers.custom.{name}")),
            None => Some(format!("providers.{s}")),
        },
    }
}

/// Split `"first rest of the string"` into (`"first"`, `"rest of the string"`).
fn split_first(s: &str) -> (&str, &str) {
    let s = s.trim();
    match s.split_once(char::is_whitespace) {
        Some((a, b)) => (a, b.trim()),
        None => (s, ""),
    }
}

const IMAGE_MENU: &str = "Image setup — pick one:\n\
    • `/onboard:image gemini <GOOGLE_AI_KEY>` — Google vision + image generation (Nano Banana).\n\
    • `/onboard:image provider <VISION_MODEL>` — use your ACTIVE provider's vision model \
    (OpenAI-compatible, no extra key). e.g. `/onboard:image provider mimo-v2.5-pro`.\n\
    • `/onboard:image generation <GENERATION_MODEL>` — route generate_image to your \
    ACTIVE provider's images endpoint (OpenAI-compatible). e.g. `/onboard:image generation \
    qwen-image`. Non-active providers: `[providers.fallback] generation = [\"name\"]`.\n\
    Ask the user which they want and re-run with the argument.";

/// `/onboard:image [gemini <key> | provider <vision_model> | generation <generation_model>]`
pub(crate) fn onboard_image(args: &str) -> Result<ToolResult> {
    let args = args.trim();
    if args.is_empty() {
        return Ok(ToolResult::success(IMAGE_MENU.into()));
    }
    let (choice, rest) = split_first(args);
    match choice.to_lowercase().as_str() {
        "gemini" | "google" => {
            let key = rest.trim();
            if key.is_empty() {
                return Ok(ToolResult::error(
                    "Need the key: `/onboard:image gemini <GOOGLE_AI_KEY>`.".into(),
                ));
            }
            if let Err(e) =
                crate::config::write_secret_key("providers.image.gemini", "api_key", key)
            {
                return Ok(ToolResult::error(format!("Failed to save key: {e}")));
            }
            let model = crate::config::default_image_model();
            for (section, k, v) in [
                ("image.vision", "enabled", "true"),
                ("image.vision", "model", model.as_str()),
                ("image.generation", "enabled", "true"),
                ("image.generation", "model", model.as_str()),
            ] {
                if let Err(e) = Config::write_key(section, k, v) {
                    return Ok(ToolResult::error(format!(
                        "Failed to write {section}.{k}: {e}"
                    )));
                }
            }
            Ok(ToolResult::success(
                "Google vision + image generation enabled. analyze_image and generate_image \
                 now use Gemini."
                    .into(),
            ))
        }
        "provider" | "openai" | "openai-compatible" | "compat" => {
            let model = rest.trim();
            if model.is_empty() {
                return Ok(ToolResult::error(
                    "Need the vision model: `/onboard:image provider <VISION_MODEL>` \
                     (a vision-capable model on your active provider)."
                        .into(),
                ));
            }
            let config = match Config::load() {
                Ok(c) => c,
                Err(e) => return Ok(ToolResult::error(format!("Failed to load config: {e}"))),
            };
            let Some(section) = active_provider_section(&config) else {
                return Ok(ToolResult::error(
                    "No active provider. Set one up with /models or /onboard:provider first."
                        .into(),
                ));
            };
            if let Err(e) = Config::write_key(&section, "vision_model", model) {
                return Ok(ToolResult::error(format!(
                    "Failed to write {section}.vision_model: {e}"
                )));
            }
            Ok(ToolResult::success(format!(
                "Vision now routes through your active provider's '{model}' (OpenAI-compatible) — \
                 no extra key needed. analyze_image will use it, with Gemini as fallback if a key \
                 is set."
            )))
        }
        "generation" | "generate" | "gen" => {
            let model = rest.trim();
            if model.is_empty() {
                return Ok(ToolResult::error(
                    "Need the generation model: `/onboard:image generation <GENERATION_MODEL>` \
                     (an images-capable model on your active provider, e.g. qwen-image)."
                        .into(),
                ));
            }
            let config = match Config::load() {
                Ok(c) => c,
                Err(e) => return Ok(ToolResult::error(format!("Failed to load config: {e}"))),
            };
            let Some(section) = active_provider_section(&config) else {
                return Ok(ToolResult::error(
                    "No active provider. Set one up with /models or /onboard:provider first."
                        .into(),
                ));
            };
            if let Err(e) = Config::write_key(&section, "generation_model", model) {
                return Ok(ToolResult::error(format!(
                    "Failed to write {section}.generation_model: {e}"
                )));
            }
            Ok(ToolResult::success(format!(
                "Generation now routes through your active provider's '{model}' \
                 (OpenAI-compatible /images/generations). To keep generating on this model \
                 while chatting elsewhere, add it to [providers.fallback] generation. \
                 Gemini stays last resort."
            )))
        }
        other => Ok(ToolResult::error(format!(
            "Unknown image option '{other}'. Use 'gemini', 'provider' or 'generation'.\n\n{IMAGE_MENU}"
        ))),
    }
}

const VOICE_MENU: &str = "Voice setup — STT (speech→text) and TTS (text→speech). Pick:\n\
    • `/onboard:voice stt groq <GROQ_KEY>` — Groq Whisper.\n\
    • `/onboard:voice stt openai <BASE_URL> <MODEL> <KEY>` — any OpenAI-compatible STT.\n\
    • `/onboard:voice stt off`\n\
    • `/onboard:voice tts openai <OPENAI_KEY>` — OpenAI TTS.\n\
    • `/onboard:voice tts off`\n\
    Ask the user what they want, then re-run with the argument.";

/// `/onboard:voice [stt … | tts …]`
pub(crate) fn onboard_voice(args: &str) -> Result<ToolResult> {
    let args = args.trim();
    if args.is_empty() {
        return Ok(ToolResult::success(VOICE_MENU.into()));
    }
    let (kind, rest) = split_first(args);
    match kind.to_lowercase().as_str() {
        "stt" => onboard_voice_stt(rest),
        "tts" => onboard_voice_tts(rest),
        other => Ok(ToolResult::error(format!(
            "Use 'stt' or 'tts'. Got '{other}'.\n\n{VOICE_MENU}"
        ))),
    }
}

/// Put the engine this command just enabled at the head of the on-disk
/// chain (#1399): enabling a flag while the chain still leads with a
/// disabled engine leaves voice dead from the next boot.
fn promote_chain_head(section: &str, head: &str) -> Result<()> {
    let vc = crate::config::Config::current().voice_config();
    let current = if section == "providers.stt" {
        &vc.stt_fallback_chain
    } else {
        &vc.tts_fallback_chain
    };
    let chain = crate::tui::onboarding::voice_chain::promote_head(current, head);
    Config::write_array(section, "fallback_chain", &chain).map_err(|e| {
        super::error::ToolError::Execution(format!("Failed to write {section}.fallback_chain: {e}"))
    })
}

fn onboard_voice_stt(rest: &str) -> Result<ToolResult> {
    let (provider, params) = split_first(rest);
    match provider.to_lowercase().as_str() {
        "off" => set_flags(&[
            ("providers.stt.openai_compatible", "enabled", "false"),
            ("providers.stt.voicebox", "enabled", "false"),
        ])
        .map(|_| ToolResult::success("STT disabled.".into())),
        "groq" => {
            let key = params.trim();
            if key.is_empty() {
                return Ok(ToolResult::error(
                    "Need the key: stt groq <GROQ_KEY>".into(),
                ));
            }
            if let Err(e) = crate::config::write_secret_key("providers.groq", "api_key", key) {
                return Ok(ToolResult::error(format!("Failed to save key: {e}")));
            }
            // Persist the enablement too: keys alone do not flip the runtime
            // voice gate (#1233). Mirrors the openai-compatible arm below.
            set_flags(&[("providers.stt.groq", "enabled", "true")])?;
            promote_chain_head("providers.stt", "groq")?;
            Ok(ToolResult::success(
                "Groq Whisper STT enabled (uses your Groq key).".into(),
            ))
        }
        "openai" | "openai-compatible" | "compat" => {
            let parts: Vec<&str> = params.split_whitespace().collect();
            let [base_url, model, key] = parts.as_slice() else {
                return Ok(ToolResult::error(
                    "Usage: stt openai <BASE_URL> <MODEL> <KEY>".into(),
                ));
            };
            set_flags(&[
                ("providers.stt.openai_compatible", "enabled", "true"),
                ("providers.stt.openai_compatible", "base_url", base_url),
                ("providers.stt.openai_compatible", "model", model),
            ])?;
            if let Err(e) =
                crate::config::write_secret_key("providers.stt.openai_compatible", "api_key", key)
            {
                return Ok(ToolResult::error(format!("Failed to save key: {e}")));
            }
            promote_chain_head("providers.stt", "openai_compatible")?;
            Ok(ToolResult::success(format!(
                "OpenAI-compatible STT enabled ({model} @ {base_url})."
            )))
        }
        other => Ok(ToolResult::error(format!(
            "Unknown STT '{other}'. Use groq, openai, or off."
        ))),
    }
}

fn onboard_voice_tts(rest: &str) -> Result<ToolResult> {
    let (provider, params) = split_first(rest);
    match provider.to_lowercase().as_str() {
        "off" => set_flags(&[("providers.tts.openai", "enabled", "false")])
            .map(|_| ToolResult::success("TTS disabled.".into())),
        "openai" => {
            let key = params.trim();
            if key.is_empty() {
                return Ok(ToolResult::error(
                    "Need the key: tts openai <OPENAI_KEY>".into(),
                ));
            }
            set_flags(&[("providers.tts.openai", "enabled", "true")])?;
            if let Err(e) = crate::config::write_secret_key("providers.openai", "api_key", key) {
                return Ok(ToolResult::error(format!("Failed to save key: {e}")));
            }
            promote_chain_head("providers.tts", "openai")?;
            Ok(ToolResult::success("OpenAI TTS enabled.".into()))
        }
        other => Ok(ToolResult::error(format!(
            "Unknown TTS '{other}'. Use openai or off."
        ))),
    }
}

const TELEGRAM_NO_TOKEN_HELP: &str = "Telegram setup — create a bot on @BotFather:\n\
    1. Open @BotFather in Telegram (https://t.me/BotFather)\n\
    2. Send /newbot\n\
    3. Choose a display name for your bot\n\
    4. Choose a username (must end with 'bot', e.g. myteam_crab_bot)\n\
    5. Copy the token BotFather gives you (looks like: 123456789:ABCdef...)\n\
    6. Send it here: `/onboard:channels telegram <TOKEN>`\n\n\
    Once the token is saved, the bot starts automatically.";

const CHANNELS_MENU: &str = "Channel setup — connect a messenger:\n\
    • `/onboard:channels telegram <BOT_TOKEN> [YOUR_NUMERIC_ID]` — or just `/onboard:channels telegram` for setup steps\n\
    • `/onboard:channels telegram richtext on|off`: rich text experience (needs the latest Telegram app)\n\
    • `/onboard:channels discord <BOT_TOKEN>`\n\
    • `/onboard:channels whatsapp` — starts pairing; I'll send you a QR to scan.\n\
    Ask the user which channel + token, then re-run with the argument.";

/// Validate a Telegram bot token format: "numbers:alphanumeric" with key >= 30 chars.
fn validate_telegram_token(token: &str) -> std::result::Result<(), String> {
    if token.is_empty() {
        return Err("Token is empty.".into());
    }
    if !token.contains(':') {
        return Err("Token missing ':' separator. Expected format: 123456789:ABCdef...".into());
    }
    let parts: Vec<&str> = token.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Err("Token has invalid format. Expected: 123456789:ABCdef...".into());
    }
    if parts[0].parse::<u64>().is_err() {
        return Err("Token bot ID (before ':') must be numeric.".into());
    }
    if parts[1].len() < 30 {
        return Err(
            "Token API key (after ':') is too short. Expected at least 30 characters.".into(),
        );
    }
    Ok(())
}

/// `/onboard:channels [telegram <token> | discord <token> | whatsapp]`
pub(crate) fn onboard_channels(args: &str) -> Result<ToolResult> {
    let args = args.trim();
    if args.is_empty() {
        return Ok(ToolResult::success(CHANNELS_MENU.into()));
    }
    let (channel, params) = split_first(args);
    match channel.to_lowercase().as_str() {
        "telegram" => {
            // `<token> [numeric_user_id]` — the token has no spaces, so the
            // optional second word is the owner's numeric ID.
            let parts: Vec<&str> = params.split_whitespace().collect();
            // Rich text toggle (#418): `telegram richtext on|off` flips
            // channels.telegram.rich_messages; hot-reload applies it live.
            if matches!(
                parts.first().map(|p| p.to_lowercase()).as_deref(),
                Some("richtext" | "rich_text" | "rich")
            ) {
                let value = match parts.get(1).map(|v| v.to_lowercase()).as_deref() {
                    Some("on" | "true" | "yes") => true,
                    Some("off" | "false" | "no") => false,
                    _ => {
                        return Ok(ToolResult::error(
                            "Usage: `/onboard:channels telegram richtext on|off`. Enable only \
                             if you run the latest Telegram app version (older clients will \
                             not render rich messages correctly)."
                                .into(),
                        ));
                    }
                };
                set_flags(&[("channels.telegram", "rich_messages", &value.to_string())])?;
                return Ok(ToolResult::success(if value {
                    "Rich text experience enabled. It only renders on current Telegram \
                     apps; run `/onboard:channels telegram richtext off` if messages \
                     show as unsupported."
                        .into()
                } else {
                    "Rich text experience disabled. Messages use the universal HTML \
                     rendering that works on every client."
                        .into()
                }));
            }
            let token = parts.first().copied().unwrap_or("");
            if token.is_empty() {
                // No token provided — show BotFather setup instructions
                return Ok(ToolResult::success(TELEGRAM_NO_TOKEN_HELP.into()));
            }
            if let Err(e) = validate_telegram_token(token) {
                return Ok(ToolResult::error(format!("Invalid token: {e}")));
            }
            set_flags(&[("channels.telegram", "enabled", "true")])?;
            if let Err(e) = crate::config::write_secret_key("channels.telegram", "token", token) {
                return Ok(ToolResult::error(format!("Failed to save token: {e}")));
            }
            // Optional numeric user ID → allowlist. Telegram's Bot API can't
            // reveal who owns the bot from the token, so the owner's ID has to
            // be supplied (or the bot learns it when the owner messages it).
            let id_note = match parts.get(1) {
                Some(id)
                    if id
                        .trim_start_matches('-')
                        .chars()
                        .all(|c| c.is_ascii_digit()) =>
                {
                    if let Err(e) =
                        Config::write_array("channels.telegram", "allowed_users", &[id.to_string()])
                    {
                        return Ok(ToolResult::error(format!("Failed to save user ID: {e}")));
                    }
                    format!(" Allowed user set to {id}.")
                }
                Some(bad) => {
                    return Ok(ToolResult::error(format!(
                        "User ID '{bad}' must be numeric — get it from @userinfobot on Telegram."
                    )));
                }
                None => " No numeric user ID given — message the bot so it learns your ID, or \
                         pass it: `/onboard:channels telegram <token> <numeric_id>`."
                    .to_string(),
            };
            Ok(ToolResult::success(format!(
                "Telegram enabled.{id_note} The bot starts on next restart or hot-reload — \
                 message it to confirm it replies."
            )))
        }
        "discord" => {
            let token = params.trim();
            if token.is_empty() {
                return Ok(ToolResult::error(
                    "Need the bot token: `/onboard:channels discord <BOT_TOKEN>`.".into(),
                ));
            }
            set_flags(&[("channels.discord", "enabled", "true")])?;
            if let Err(e) = crate::config::write_secret_key("channels.discord", "token", token) {
                return Ok(ToolResult::error(format!("Failed to save token: {e}")));
            }
            Ok(ToolResult::success(
                "Discord enabled. Restart (or hot-reload) to start the bot.".into(),
            ))
        }
        "whatsapp" => Ok(ToolResult::success(
            "WhatsApp pairing uses a QR code. Call the `whatsapp_connect` tool — it starts \
             pairing and yields a QR to scan in WhatsApp → Linked Devices."
                .into(),
        )),
        other => Ok(ToolResult::error(format!(
            "Unknown channel '{other}'. Use telegram, discord, or whatsapp.\n\n{CHANNELS_MENU}"
        ))),
    }
}

/// Write a batch of `config.toml` flags, stopping at the first error.
fn set_flags(flags: &[(&str, &str, &str)]) -> Result<()> {
    for (section, key, val) in flags {
        if let Err(e) = Config::write_key(section, key, val) {
            return Err(super::error::ToolError::Execution(format!(
                "Failed to write {section}.{key}: {e}"
            )));
        }
    }
    Ok(())
}
