//! /cowork Command: Add OpenCrabs to a Telegram group with auto-registration.
//!
//! Flow:
//! 1. User sends `/cowork` in DM → bot shows "Add to Group" button immediately
//! 2. User taps link → Telegram native UI lets them pick/create a group
//! 3. Bot detects `/start cowork_<id>` in new group → generates invite link + QR
//! 4. QR sent to user's DM. All group members auto-register in allowed_users.

use crate::config::{Config, opencrabs_home};

use super::TelegramState;
use super::send::{best_effort_note, message_in_thread, photo_in_thread};
use teloxide::prelude::*;
use teloxide::types::{ChatId, InlineKeyboardButton, InlineKeyboardMarkup, InputFile};

/// Prefix for cowork startgroup parameters.
const COWORK_PREFIX: &str = "cowork_";

/// Lightweight session linking a `/cowork` DM to the group that will be created.
/// Stored so when the bot joins via the deep link, we know where to send the QR.
#[derive(Debug, Clone)]
pub struct CoworkState {
    /// User who initiated /cowork.
    pub user_id: i64,
    /// DM chat where /cowork was sent (for sending QR back).
    pub chat_id: i64,
    /// Unique session identifier for this cowork flow.
    pub session_id: String,
    /// When this state was created. If the user never taps "Add to Group",
    /// the state silently expires and their next message is processed normally.
    pub created_at: std::time::Instant,
}

/// If the user doesn't click "Add to Group" within this window, the cowork
/// state is silently cleared and their next message goes through normally.
const COWORK_TIMEOUT_SECS: u64 = 120;

impl CoworkState {
    pub fn new(user_id: i64, chat_id: i64, session_id: String) -> Self {
        Self {
            user_id,
            chat_id,
            session_id,
            created_at: std::time::Instant::now(),
        }
    }

    /// Returns true if this cowork state has expired (user never tapped the link).
    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed().as_secs() > COWORK_TIMEOUT_SECS
    }
}

/// Check if a `/start` parameter is a cowork session.
pub fn is_cowork_session(param: &str) -> bool {
    param.starts_with(COWORK_PREFIX) && param.len() > COWORK_PREFIX.len()
}

/// Extract the session ID from a cowork startgroup parameter.
/// "cowork_abc123" → Some("abc123"), "other" → None.
pub fn parse_startgroup_param(param: &str) -> Option<&str> {
    if is_cowork_session(param) {
        Some(&param[COWORK_PREFIX.len()..])
    } else {
        None
    }
}

/// Admin rights requested inline in the cowork deep link (#709). Telegram
/// pre-checks these in the Add-to-Group UI so the bot is added already-promoted:
/// no manual promotion step. `invite_users` is what `create_chat_invite_link`
/// needs; being an admin also lets the bot read every message regardless of
/// privacy mode, plus light moderation. Space-separated per Telegram's format.
const COWORK_ADMIN_RIGHTS: &str = "invite_users delete_messages pin_messages manage_chat";

/// Build a Telegram deep link that opens the "create group with bot" UI, with
/// admin rights requested inline so the bot joins already-promoted (#709).
/// Format: `https://t.me/{bot}?startgroup=cowork_{id}&admin=<rights+list>`
pub fn build_cowork_deep_link(bot_username: &str, session_id: &str) -> String {
    format!(
        "https://t.me/{}?startgroup={}{}&admin={}",
        bot_username,
        COWORK_PREFIX,
        session_id,
        COWORK_ADMIN_RIGHTS.replace(' ', "+")
    )
}

/// Generate a QR code PNG from an invite link. Returns (png_bytes, file_path).
/// Reuses `render_qr_png` from whatsapp_connect.
#[cfg(feature = "whatsapp")]
pub fn build_invite_qr(invite_link: &str) -> Option<(Vec<u8>, std::path::PathBuf)> {
    let png_bytes = crate::brain::tools::whatsapp_connect::render_qr_png(invite_link)?;
    write_invite_qr(&png_bytes)
}

#[cfg(not(feature = "whatsapp"))]
pub fn build_invite_qr(_invite_link: &str) -> Option<(Vec<u8>, std::path::PathBuf)> {
    // QR rendering lives behind the whatsapp feature (qrcode dep); degrade gracefully
    None
}

fn write_invite_qr(png_bytes: &[u8]) -> Option<(Vec<u8>, std::path::PathBuf)> {
    let dir = opencrabs_home().join("tmp");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("cowork_invite_qr.png");
    std::fs::write(&path, png_bytes).ok()?;
    Some((png_bytes.to_vec(), path))
}

/// Append a user_id to a group's per-group allowlist
/// (`channels.telegram.groups.<chat_id>.allowed_users`) if not already present.
/// This keeps cowork members scoped to their group — they cannot DM the bot
/// privately unless also listed in the global `allowed_users` or `bot_owner`.
/// Returns Ok(true) if newly registered, Ok(false) if already existed.
pub fn auto_register_to_group(user_id: i64, chat_id: i64) -> Result<bool, String> {
    let config = Config::load().map_err(|e| format!("Failed to load config: {e}"))?;

    let id_str = user_id.to_string();
    let chat_id_str = chat_id.to_string();

    // Check if already in this group's allowlist
    if let Some(group) = config.channels.telegram.groups.get(&chat_id_str)
        && group.allowed_users.contains(&id_str)
    {
        return Ok(false);
    }

    // Build the new allowlist for this group (existing + new user)
    let mut users: Vec<String> = config
        .channels
        .telegram
        .groups
        .get(&chat_id_str)
        .map(|g| g.allowed_users.clone())
        .unwrap_or_default();
    users.push(id_str);

    Config::write_array(
        &format!("channels.telegram.groups.{}", chat_id_str),
        "allowed_users",
        &users,
    )
    .map_err(|e| format!("Failed to write group allowed_users: {e}"))?;

    Ok(true)
}

/// Check if a chat_id is a tracked cowork group.
/// Persist `[channels.telegram.groups.<chat_id>].open = true` (#718). This is the
/// owner-initiated opt-in that /cowork performs: every member of the group is
/// then allowed (and auto-registered on join / on /start), and it stays open
/// until the owner changes it. Goes through the guarded `write_key`, so it can
/// never corrupt config.toml.
pub fn set_group_open(chat_id: i64) -> Result<(), String> {
    Config::write_key(
        &format!("channels.telegram.groups.{}", chat_id),
        "open",
        "true",
    )
    .map(|_| ())
    .map_err(|e| format!("Failed to open group {chat_id}: {e}"))
}

/// Handle the /cowork command in DM. Immediately shows the "Add to Group" button.
pub async fn handle_cowork_command(
    bot: &Bot,
    _msg: &Message,
    state: &TelegramState,
    user_id: i64,
    chat_id: i64,
    thread_id: Option<teloxide::types::ThreadId>,
) -> Result<(), teloxide::RequestError> {
    let bot_username = state
        .bot_username()
        .await
        .unwrap_or_else(|| "opencrabsbot".to_string());

    // If user already has a pending cowork session, reuse it; otherwise start new one
    let session_id = if let Some(existing) = state.get_cowork_state(user_id).await {
        existing.session_id
    } else {
        let sid = uuid::Uuid::new_v4().to_string()[..8].to_string();
        state.start_cowork(user_id, chat_id, sid.clone()).await;
        sid
    };

    let deep_link = build_cowork_deep_link(&bot_username, &session_id);

    let keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::url(
        "🦀 Add to Group".to_string(),
        deep_link.parse().unwrap(),
    )]]);

    let text = "Tap below to add me to a Telegram group.\n\n\
        After joining, I'll check if I have admin access. \
        If not, I'll send you instructions to promote me.";

    message_in_thread(bot, ChatId(chat_id), thread_id, text)
        .reply_markup(keyboard)
        .await?;
    Ok(())
}

/// Handle the bot being added to a group via `?startgroup=cowork_<id>`.
pub async fn handle_cowork_group_join(
    bot: &Bot,
    msg: &Message,
    state: &TelegramState,
    param: &str,
    thread_id: Option<teloxide::types::ThreadId>,
) -> Result<(), teloxide::RequestError> {
    let Some(session_id) = parse_startgroup_param(param) else {
        return Ok(());
    };

    let group_chat_id = msg.chat.id.0;

    // Look up the cowork state by session_id
    let cowork = state.take_cowork_by_session(session_id).await;

    // Check if the bot is an admin in the group before attempting admin-only ops
    let bot_info = bot.get_me().await?;
    let bot_id = bot_info.id;
    let bot_member = bot.get_chat_member(ChatId(group_chat_id), bot_id).await?;
    let is_admin = matches!(
        bot_member.status(),
        teloxide::types::ChatMemberStatus::Administrator | teloxide::types::ChatMemberStatus::Owner
    );

    if !is_admin {
        let promote_msg = "🦀 I need **admin privileges** to create invite links and read messages.\n\n\
            **To fix this:**\n\
            1. Open Group Settings → Administrators → Add Admin\n\
            2. Select me and confirm\n\
            3. Disable privacy mode: send `/setprivacy` to @BotFather, choose this bot, set to Disabled\n\n\
            Then send `/cowork` in your DM with me again to generate the invite link.";

        best_effort_note(
            bot,
            ChatId(group_chat_id),
            thread_id,
            promote_msg,
            None,
            "system",
            "cowork_promote_group",
            "needs-admin notice",
        )
        .await;

        // Also notify the user in their DM
        if let Some(ref cowork_state) = cowork {
            let user_chat = ChatId(cowork_state.chat_id);
            let dm_msg = "🦀 I was added to the group but I'm **not an admin** yet.\n\n\
                **What to do:**\n\
                1. Go to the group → Group Settings → Administrators\n\
                2. Add me as admin\n\
                3. Disable privacy mode: send `/setprivacy` to @BotFather, choose this bot, set to Disabled\n\n\
                Then send `/cowork` here again to generate the invite link and QR code.";

            best_effort_note(
                bot,
                user_chat,
                None,
                dm_msg,
                None,
                "system",
                "cowork_promote_dm",
                "needs-admin DM",
            )
            .await;
        }

        return Ok(());
    }

    // Bot is admin — generate invite link
    let invite_result = bot.create_chat_invite_link(ChatId(group_chat_id)).await;

    match invite_result {
        Ok(link) => {
            let invite_url = &link.invite_link;

            if let (Some((_png_bytes, qr_path)), Some(cowork_state)) =
                (build_invite_qr(invite_url), cowork.as_ref())
            {
                let user_chat = ChatId(cowork_state.chat_id);
                // Warn-only: a QR sidecar photo; failure visibility suffices.
                if let Err(e) =
                    photo_in_thread(bot, user_chat, None, InputFile::file(qr_path)).await
                {
                    tracing::warn!(
                        "Telegram: cowork QR photo failed (chat={}): {e}",
                        user_chat.0
                    );
                }
                let invite_note = format!(
                    "🦀 **All set!**\n\n\
                     Invite link: {}\n\n\
                     Share the QR or link. \
                     Everyone auto-registers when they join and send a message.",
                    invite_url
                );
                best_effort_note(
                    bot,
                    user_chat,
                    None,
                    &invite_note,
                    None,
                    "system",
                    "cowork_invite_dm",
                    "invite ready DM",
                )
                .await;
            }

            best_effort_note(
                bot,
                ChatId(group_chat_id),
                thread_id,
                "🦀 I'm in! @mention me anytime to chat.\n\n\
                 Everyone here is auto-registered. No setup needed.",
                None,
                "system",
                "cowork_joined_group",
                "joined announcement",
            )
            .await;
        }
        Err(e) => {
            tracing::error!("Cowork: failed to create invite link: {}", e);
            best_effort_note(
                bot,
                ChatId(group_chat_id),
                thread_id,
                "Failed to generate invite link. Make sure I'm an admin in this group.",
                None,
                "system",
                "cowork_invite_failed",
                "invite failure notice",
            )
            .await;
        }
    }

    Ok(())
}
