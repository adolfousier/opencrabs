//! Discord Message Handler
//!
//! Processes incoming Discord messages: text + image attachments, allowlist enforcement,
//! session routing (owner shares TUI session, others get per-user sessions).

use super::DiscordState;
use crate::brain::agent::AgentService;
use crate::config::{Config, RespondTo};
use crate::db::ChannelMessageRepository;
use crate::db::models::ChannelMessage as DbChannelMessage;
use crate::services::SessionService;
use crate::utils::sanitize::redact_secrets;
use crate::utils::truncate_str;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

use serenity::builder::{CreateAttachment, CreateMessage};
use serenity::model::channel::Message;
use serenity::prelude::*;

/// Split a message into chunks that fit Discord's 2000 char limit.
/// Whether a chunk ending here leaves no markup open (#876).
///
/// Chunks are sent as separate messages and parsed independently, so an
/// unclosed `<code>`/`<b>` or an odd number of backticks makes that chunk
/// invalid on its own. Counts backtick runs and unclosed HTML start tags.
///
/// Conservative by design: it only ever moves a break EARLIER, never past the
/// length limit, so a false negative costs a slightly shorter chunk and a false
/// positive is the behaviour that already shipped.
fn splits_cleanly(prefix: &str) -> bool {
    if !prefix.matches('`').count().is_multiple_of(2) {
        return false;
    }
    let mut depth: i32 = 0;
    let mut rest = prefix;
    while let Some(open) = rest.find('<') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('>') else {
            return false; // a start tag left dangling mid-chunk
        };
        let tag = &after[..close];
        if !tag.starts_with('!') {
            if tag.starts_with('/') {
                depth -= 1;
            } else if !tag.ends_with('/') {
                depth += 1;
            }
        }
        rest = &after[close + 1..];
    }
    depth == 0
}

pub fn split_message(text: &str, max_len: usize) -> Vec<&str> {
    if text.len() <= max_len {
        return vec![text];
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + max_len).min(text.len());
        // Ensure end falls on a char boundary (back up if inside a multi-byte char)
        while end < text.len() && !text.is_char_boundary(end) {
            end -= 1;
        }
        let break_at = if end < text.len() {
            // Prefer a newline near the limit, as before. Then, among the
            // newlines in the window, prefer one that does NOT sit inside an
            // open code span or HTML tag (#876): breaking there leaves an
            // unclosed `<code>` or a lone backtick, which Telegram rejects when
            // it parses each chunk independently.
            //
            // Strictly an improvement: when no markup spans the boundary the
            // first candidate is already safe and the result is byte-identical
            // to the old behaviour. Only the previously-broken case moves.
            let window = &text[start..end];
            let floor = end - start - 200;
            let mut chosen = None;
            for (pos, _) in window.char_indices().rev().filter(|(_, c)| *c == '\n') {
                if pos <= floor {
                    break;
                }
                if splits_cleanly(&window[..pos]) {
                    chosen = Some(start + pos + 1);
                    break;
                }
            }
            chosen.unwrap_or_else(|| {
                window
                    .rfind('\n')
                    .filter(|&pos| pos > floor)
                    .map(|pos| start + pos + 1)
                    .unwrap_or(end)
            })
        } else {
            end
        };
        chunks.push(&text[start..break_at]);
        start = break_at;
    }
    chunks
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_message(
    ctx: &Context,
    msg: &Message,
    agent: Arc<AgentService>,
    session_svc: SessionService,
    shared_session: Arc<Mutex<Option<Uuid>>>,
    discord_state: Arc<DiscordState>,
    config_rx: tokio::sync::watch::Receiver<Config>,
    channel_msg_repo: ChannelMessageRepository,
) {
    // Read latest config from watch channel — single source of truth
    let cfg = config_rx.borrow().clone();
    let dc_cfg = &cfg.channels.discord;
    let allowed: HashSet<i64> = dc_cfg
        .allowed_users
        .iter()
        .filter_map(|s| s.parse().ok())
        .collect();
    let respond_to = &dc_cfg.respond_to;
    let allowed_channels: HashSet<String> = dc_cfg.allowed_channels.iter().cloned().collect();
    let idle_timeout_hours = dc_cfg.session_idle_hours;
    let voice_config = cfg.voice_config();

    let user_id = msg.author.id.get() as i64;

    // Helper: passively capture a channel message for history
    let store_channel_msg = |text: String| {
        let repo = channel_msg_repo.clone();
        let channel_chat_id = msg.channel_id.get().to_string();
        let guild_name = msg
            .guild_id
            .map(|g| g.get().to_string())
            .unwrap_or_else(|| "DM".to_string());
        let sender_id = msg.author.id.get().to_string();
        let sender_name = msg.author.name.clone();
        let msg_id = msg.id.get().to_string();
        async move {
            if text.is_empty() {
                return;
            }
            let cm = DbChannelMessage::new(
                "discord".into(),
                channel_chat_id,
                Some(guild_name),
                sender_id,
                sender_name,
                text,
                "text".into(),
                Some(msg_id),
            );
            if let Err(e) = repo.insert(&cm).await {
                tracing::warn!("Failed to store Discord channel message: {e}");
            }
        }
    };

    // Deny-by-default allowlist (OC-02). An empty allowlist used to accept
    // everyone, unlike Telegram, which denies an unconfigured channel. Now a
    // channel with no allowed_users, no allowed_roles, and no bot_owner denies;
    // a configured one admits only allowlisted users, holders of an allowed
    // role (#387, evaluated per message), or the owner. Roles are evaluated
    // even when allowed_users is empty, and a DM is never treated as
    // role-granted (guild roles do not apply to a DM).
    let is_owner = crate::config::owner::is_owner(
        &dc_cfg.allowed_users,
        &dc_cfg.bot_owner,
        &user_id.to_string(),
    );
    let in_allowlist = allowed.contains(&user_id);
    let role_granted = msg.guild_id.is_some()
        && !dc_cfg.allowed_roles.is_empty()
        && msg.member.as_ref().is_some_and(|m| {
            m.roles.iter().any(|r| {
                dc_cfg
                    .allowed_roles
                    .iter()
                    .any(|ar| ar == &r.get().to_string())
            })
        });
    let unconfigured =
        allowed.is_empty() && dc_cfg.allowed_roles.is_empty() && dc_cfg.bot_owner.is_empty();
    if unconfigured || !(is_owner || in_allowlist || role_granted) {
        tracing::debug!(
            "Discord: ignoring message from non-allowed user {} (deny-by-default, OC-02)",
            user_id
        );
        return;
    }

    // respond_to / allowed_channels filtering — DMs always pass
    let is_dm = msg.guild_id.is_none();
    if !is_dm {
        let channel_str = msg.channel_id.get().to_string();

        // Check allowed_channels (empty = all channels allowed). Threads and
        // forum posts (#384) carry their own channel id, so a miss falls back
        // to the PARENT channel: allow-listing a forum allows every post in
        // it, each post keeping its own per-thread session.
        if !allowed_channels.is_empty() && !allowed_channels.contains(&channel_str) {
            let parent_allowed = match msg.channel_id.to_channel(&ctx.http).await {
                Ok(serenity::model::channel::Channel::Guild(gc)) => gc
                    .parent_id
                    .is_some_and(|p| allowed_channels.contains(&p.get().to_string())),
                _ => false,
            };
            if !parent_allowed {
                tracing::debug!(
                    "Discord: ignoring message in non-allowed channel {} (parent not allowed either)",
                    channel_str
                );
                store_channel_msg(msg.content.clone()).await;
                return;
            }
        }

        match respond_to {
            RespondTo::DmOnly => {
                tracing::debug!("Discord: respond_to=dm_only, ignoring channel message");
                store_channel_msg(msg.content.clone()).await;
                return;
            }
            RespondTo::Mention => {
                let bot_id = discord_state.bot_user_id().await;
                let mentioned =
                    bot_id.is_some_and(|bid| msg.mentions.iter().any(|u| u.id.get() == bid));
                if !mentioned {
                    tracing::debug!("Discord: respond_to=mention, bot not mentioned — ignoring");
                    store_channel_msg(msg.content.clone()).await;
                    return;
                }
            }
            RespondTo::All => {} // pass through
            RespondTo::Auto => {
                // Active sender tracking not implemented for Discord yet;
                // fall back to mention-only behaviour (#244).
                let bot_id = discord_state.bot_user_id().await;
                let mentioned =
                    bot_id.is_some_and(|bid| msg.mentions.iter().any(|u| u.id.get() == bid));
                if !mentioned {
                    tracing::debug!("Discord: respond_to=auto, bot not mentioned — ignoring");
                    store_channel_msg(msg.content.clone()).await;
                    return;
                }
            }
        }
    }

    // Also store directed channel messages for complete history
    if !is_dm {
        store_channel_msg(msg.content.clone()).await;
    }

    // Check for audio attachments → STT
    let audio_attachment = msg.attachments.iter().find(|a| {
        a.content_type
            .as_ref()
            .is_some_and(|ct| ct.starts_with("audio/"))
    });

    let mut is_voice = false;
    let mut content = msg.content.clone();

    // Show typing immediately when processing voice
    if audio_attachment.is_some()
        && voice_config.stt_enabled
        && let Err(e) = msg.channel_id.broadcast_typing(&ctx.http).await
    {
        tracing::warn!(error = %e, "failed to broadcast Discord typing");
    }

    if let Some(audio) = audio_attachment
        && voice_config.stt_enabled
        && let Ok(resp) = reqwest::get(&audio.url).await
        && let Ok(bytes) = resp.bytes().await
    {
        match crate::channels::voice::transcribe(bytes.to_vec(), &voice_config).await {
            Ok(transcript) => {
                tracing::info!(
                    "Discord: transcribed voice: {}",
                    truncate_str(&transcript, 80)
                );
                content = transcript;
                is_voice = true;
            }
            Err(e) => tracing::error!("Discord: STT error: {e}"),
        }
    }

    // Strip bot @mention from content when responding to a mention
    if !is_dm
        && respond_to == &RespondTo::Mention
        && let Some(bot_id) = discord_state.bot_user_id().await
    {
        let mention_tag = format!("<@{}>", bot_id);
        content = content.replace(&mention_tag, "").trim().to_string();
    }
    if content.is_empty() && msg.attachments.is_empty() {
        return;
    }

    // Handle attachments — vision-first pipeline
    if !is_voice {
        use crate::utils::{inject_file_content, process_file_with_vision};
        for attachment in &msg.attachments {
            let mime = attachment.content_type.as_deref().unwrap_or("");
            let fname = &attachment.filename;

            if mime.starts_with("image/") {
                if content.is_empty() {
                    content = "Describe this image.".to_string();
                }
                content.push_str(&format!(" <<IMG:{}>>", attachment.url));
            } else if !mime.starts_with("audio/")
                && let Ok(resp) = reqwest::get(attachment.url.as_str()).await
                && let Ok(bytes) = resp.bytes().await
            {
                let cfg = config_rx.borrow();
                let fc = process_file_with_vision(&bytes, mime, fname, &cfg);
                let injected = inject_file_content(&fc).0;
                if !injected.is_empty() {
                    content.push_str(&format!("\n\n{injected}"));
                }
            }
        }
    }

    if content.is_empty() {
        return;
    }

    let text_preview = truncate_str(&content, 50);
    tracing::info!(
        "Discord: message from {} ({}): {}",
        msg.author.name,
        user_id,
        text_preview
    );

    // Track owner's channel for proactive messaging
    let is_owner = dc_cfg.is_owner(&user_id.to_string());

    if is_owner {
        discord_state.set_owner_channel(msg.channel_id.get()).await;
    }

    // Track guild ID for guild-scoped actions (kick, ban, roles, list_channels)
    if let Some(guild_id) = msg.guild_id {
        discord_state.set_guild_id(guild_id.get()).await;
    }

    // Sessions are ALWAYS isolated per chat — owner DMs no longer share the
    // TUI session. DMs keyed by author user_id; guild channels by channel_id.
    // Title carries a stable `[chat:discord-…]` suffix so auto-rename rewrites
    // the visible label but `find_session_by_title_suffix` still resolves the
    // same row (issue #121, pre-fix every renamed session was orphaned).
    let session_id = {
        use crate::channels::session_resolve;
        let (id_str, legacy_title) = if is_dm {
            (
                format!("discord-dm-{}", msg.author.id.get()),
                format!("Discord: DM {} ({})", msg.author.name, msg.author.id.get()),
            )
        } else {
            (
                format!("discord-{}", msg.channel_id.get()),
                format!("Discord: #{}", msg.channel_id.get()),
            )
        };
        let suffix = session_resolve::chat_id_suffix(&id_str);
        let session_title = format!("{legacy_title} {suffix}");

        match session_resolve::resolve_or_create_channel_session(
            &session_svc,
            &suffix,
            &legacy_title,
            &session_title,
            idle_timeout_hours,
            "Discord",
        )
        .await
        {
            Ok(id) => id,
            Err(e) => {
                tracing::error!("Discord: failed to resolve session: {e:#} (#442)");
                if let Err(send_err) = msg
                    .channel_id
                    .say(
                        &ctx.http,
                        format!(
                            "⚠️ Could not load this chat's session ({e}). Your history is \
                             intact and this message was NOT processed. Try again, or send \
                             /new if you deliberately want a fresh session."
                        ),
                    )
                    .await
                {
                    tracing::warn!(error = %send_err, "failed to send Discord session error message");
                }
                return;
            }
        }
    };

    // Session gate (#1051, ADR-003): mark group sessions so memory_search
    // keeps external index content out of them by default.
    if !is_dm {
        crate::memory::mark_session_shared(session_id);
    }

    // Fast-cancel: any recognised stop intent, in any supported language (#965).
    //
    // Cancellation is scoped to explicit stop requests and genuine follow-up
    // messages (handled at dispatch by store_cancel_token, which cancels the
    // prior token before starting new work). Channel commands like /models,
    // /help, /usage, /new must NEVER abort an in-flight task: switching models
    // applies to the next run, it does not drop current work (#266).
    if crate::utils::stop_intent::is_stop_command_or_intent(&msg.content) {
        discord_state.cancel_session(session_id).await;
        if let Err(e) = msg.channel_id.say(&ctx.http, "Operation cancelled.").await {
            tracing::warn!(error = %e, "failed to send Discord message");
        }
        return;
    }

    // Restore session's own provider (each session keeps its provider independently)
    let session_meta = session_svc.get_session(session_id).await.ok().flatten();
    crate::channels::commands::sync_provider_for_session(
        &agent,
        session_id,
        session_meta
            .as_ref()
            .and_then(|s| s.provider_name.as_deref()),
        session_meta.as_ref().and_then(|s| s.model.as_deref()),
    )
    .await;

    // ── Channel commands (/help, /usage, /models) ──────────────────────────
    {
        use crate::channels::commands::{self, ChannelCommand};
        let cmd =
            commands::handle_command(&content, session_id, &agent, &session_svc, is_owner, None)
                .await;

        // Handle simple text-response commands (Help, Usage, Evolve, Doctor, etc.)
        if let Some(reply) = commands::try_execute_text_command(&cmd).await {
            if let Err(e) = msg.channel_id.say(&ctx.http, &reply).await {
                tracing::warn!(error = %e, "failed to send Discord message");
            }
            return;
        }

        match cmd {
            ChannelCommand::Models(resp) => {
                use serenity::builder::{CreateActionRow, CreateButton, CreateMessage};
                use serenity::model::application::ButtonStyle;
                // Show provider buttons (step 1 of two-step flow)
                let rows: Vec<CreateActionRow> = resp
                    .providers
                    .chunks(5)
                    .take(5)
                    .map(|chunk| {
                        CreateActionRow::Buttons(
                            chunk
                                .iter()
                                .map(|(name, label, configured)| {
                                    let marker = crate::channels::commands::provider_marker(
                                        name,
                                        &resp.current_provider,
                                        *configured,
                                    );
                                    let display = match marker {
                                        "🔒" => format!("🔒 {} (setup)", label),
                                        "✓" => format!("✓ {}", label),
                                        _ => label.clone(),
                                    };
                                    let display = if display.len() > 80 {
                                        format!("{}…", display.chars().take(79).collect::<String>())
                                    } else {
                                        display
                                    };
                                    let cb = if *configured {
                                        format!("provider:{}", name)
                                    } else {
                                        format!("setup:{}", name)
                                    };
                                    CreateButton::new(cb)
                                        .label(display)
                                        .style(ButtonStyle::Secondary)
                                })
                                .collect(),
                        )
                    })
                    .collect();
                let builder = CreateMessage::new().content(&resp.text).components(rows);
                // Never silent (#1019): this IS the reply. A swallowed failure
                // here is indistinguishable from the agent choosing not to
                // answer, and the user has no way to tell or report it.
                if let Err(e) = msg.channel_id.send_message(&ctx.http, builder).await {
                    tracing::error!(
                        "Discord: reply with components failed in channel {}: {e}",
                        msg.channel_id
                    );
                }
                return;
            }
            ChannelCommand::NewSession => {
                // MUST match the per-message resolver format above —
                // DM titles include the author id so /new and the next
                // typed message land on the same row (issue #89).
                let session_title = if is_dm {
                    format!("Discord: DM {} ({})", msg.author.name, msg.author.id.get())
                } else {
                    format!("Discord: #{}", msg.channel_id.get())
                };
                // The new session inherits its working directory from the
                // session that received this /new (same chat), not the global
                // most-recent session (#263).
                let prior_session = session_svc
                    .find_session_by_title(&session_title)
                    .await
                    .unwrap_or_else(|e| {
                        // /new means a fresh session IS the intent; the
                        // failure is logged, never silent (#442).
                        tracing::error!("Discord: /new prior-session lookup failed: {e:#}");
                        None
                    });
                // Archive the previous session on /new, except for the owner —
                // owner sessions stay non-archived so they remain visible in
                // /sessions for history review. Guest sessions get archived
                // so the next title lookup resolves cleanly to the new row.
                if !is_owner
                    && let Some(old) = prior_session.as_ref()
                    && let Err(e) = session_svc.archive_session(old.id).await
                {
                    tracing::error!("Discord: failed to archive old session {}: {}", old.id, e);
                }
                match crate::channels::session_init::create_channel_session(
                    &session_svc,
                    Some(session_title),
                    prior_session.as_ref(),
                )
                .await
                {
                    Ok(new_session) => {
                        if is_owner && is_dm {
                            *shared_session.lock().await = Some(new_session.id);
                        }
                        discord_state
                            .register_session_channel(new_session.id, msg.channel_id.get())
                            .await;
                        // Sync provider for the new session so baseline is accurate
                        let new_meta = session_svc.get_session(new_session.id).await.ok().flatten();
                        crate::channels::commands::sync_provider_for_session(
                            &agent,
                            new_session.id,
                            new_meta.as_ref().and_then(|s| s.provider_name.as_deref()),
                            new_meta.as_ref().and_then(|s| s.model.as_deref()),
                        )
                        .await;
                        let baseline = agent.base_context_tokens();
                        let ctx_max = agent.context_limit_for_session(new_session.id);
                        let footer = crate::utils::format_ctx_footer(baseline, ctx_max, None);
                        let msg_text = format!("✅ New session started.\n\n{footer}");
                        if let Err(e) = msg.channel_id.say(&ctx.http, &msg_text).await {
                            tracing::warn!(error = %e, "failed to send Discord message");
                        }
                        tracing::info!(
                            "Discord /new: sent ctx footer='{}' (baseline={}, ctx_max={})",
                            footer,
                            baseline,
                            ctx_max,
                        );
                    }
                    Err(e) => {
                        tracing::error!("Discord: failed to create session: {}", e);
                        if let Err(send_err) = msg
                            .channel_id
                            .say(&ctx.http, "Failed to create session.")
                            .await
                        {
                            tracing::warn!(error = %send_err, "failed to send Discord session creation error");
                        }
                    }
                }
                return;
            }
            ChannelCommand::Sessions(resp) => {
                use serenity::builder::{CreateActionRow, CreateButton, CreateMessage};
                use serenity::model::application::ButtonStyle;
                let rows: Vec<CreateActionRow> = resp
                    .sessions
                    .chunks(5)
                    .take(5)
                    .map(|chunk| {
                        CreateActionRow::Buttons(
                            chunk
                                .iter()
                                .map(|(id, label)| {
                                    let display = if *id == resp.current_session_id {
                                        format!("▸ {} ← current", label)
                                    } else {
                                        label.clone()
                                    };
                                    let display = if display.len() > 80 {
                                        format!("{}…", display.chars().take(79).collect::<String>())
                                    } else {
                                        display
                                    };
                                    CreateButton::new(format!("session:{}", id))
                                        .label(display)
                                        .style(ButtonStyle::Secondary)
                                })
                                .collect(),
                        )
                    })
                    .collect();
                let builder = CreateMessage::new().content(&resp.text).components(rows);
                // Never silent (#1019): this IS the reply. A swallowed failure
                // here is indistinguishable from the agent choosing not to
                // answer, and the user has no way to tell or report it.
                if let Err(e) = msg.channel_id.send_message(&ctx.http, builder).await {
                    tracing::error!(
                        "Discord: reply with components failed in channel {}: {e}",
                        msg.channel_id
                    );
                }
                return;
            }
            ChannelCommand::Stop => {
                let cancelled = discord_state.cancel_session(session_id).await;
                let reply = if cancelled {
                    "Operation cancelled."
                } else {
                    "No operation in progress."
                };
                if let Err(e) = msg.channel_id.say(&ctx.http, reply).await {
                    tracing::warn!(error = %e, "failed to send Discord message");
                }
                return;
            }
            ChannelCommand::Compact => {
                if let Err(e) = msg
                    .channel_id
                    .say(&ctx.http, "⏳ Compacting context...")
                    .await
                {
                    tracing::warn!(error = %e, "failed to send Discord compact notification");
                }
                content =
                    "[SYSTEM: Compact context now. Summarize this conversation for continuity.]"
                        .to_string();
            }
            ChannelCommand::UserPrompt(prompt) => {
                content = prompt;
                // fall through to agent with the prompt as the message
            }
            ChannelCommand::NotACommand => {}
            // Help, Usage, Evolve, Doctor, UserSystem handled by try_execute_text_command above
            ChannelCommand::Profiles(resp) => {
                if let Err(e) = msg.channel_id.say(&ctx.http, &resp.text).await {
                    tracing::warn!(error = %e, "failed to send Discord message");
                }
                return;
            }
            _ => {}
        }
    }

    // Extract replied-to message context so the agent knows what the user is referencing.
    let reply_context = msg.referenced_message.as_ref().and_then(|reply| {
        let reply_text = crate::utils::strip_ctx_footer(reply.content.trim());
        if reply_text.is_empty() {
            return None;
        }
        let reply_sender = if reply.author.bot {
            "assistant".to_string()
        } else {
            reply.author.name.clone()
        };
        Some(format!("[Replying to {reply_sender}: \"{reply_text}\"]"))
    });

    // Build the human-readable display text (used for DB persistence + TUI).
    // Owner DMs show the bare text; everything else gets a `Sender: text`
    // prefix so multi-user channels stay readable in OpenCrabs without
    // surfacing the LLM-only metadata brackets.
    let display_text = if is_owner && msg.guild_id.is_none() {
        content.clone()
    } else {
        format!("{}: {}", msg.author.name, content)
    };

    // For non-owner users, prepend sender identity so the agent knows who
    // it's talking to and doesn't assume it's the owner.
    let agent_input = if !is_owner {
        let name = &msg.author.name;
        let uid = msg.author.id.get();
        if msg.guild_id.is_some() {
            let channel = msg.channel_id.get();
            format!("[Discord message from {name} (ID {uid}) in channel {channel}]\n{content}")
        } else {
            format!("[Discord DM from {name} (ID {uid})]\n{content}")
        }
    } else {
        content
    };

    // Prepend reply context if the user is replying to a specific message.
    let agent_input = if let Some(ref ctx) = reply_context {
        format!("{ctx}\n{agent_input}")
    } else {
        agent_input
    };

    // Inject recent channel history so the agent has full conversation context.
    let agent_input = if msg.guild_id.is_some() {
        let chat_id_str = msg.channel_id.get().to_string();
        match channel_msg_repo
            .recent(Some("discord"), &chat_id_str, 30, None, None)
            .await
        {
            Ok(messages) if !messages.is_empty() => {
                let history: Vec<String> = messages
                    .iter()
                    .rev()
                    .map(|m| {
                        let ts = m.created_at.format("%H:%M");
                        format!("[{}] {}: {}", ts, m.sender_name, m.content)
                    })
                    .collect();
                format!(
                    "[Recent channel history ({} messages):\n{}\n--- end history ---]\n{}",
                    history.len(),
                    history.join("\n"),
                    agent_input
                )
            }
            _ => agent_input,
        }
    } else {
        agent_input
    };

    // Tell the LLM its text response is automatically delivered to the chat,
    // so it should NOT use discord_send for simple text replies. Surface the
    // channel_id so the agent can target THIS channel for cron reports /
    // cross-surface sends without guessing (#533, mirror of upstream #510).
    let channel_id = msg.channel_id.get();
    let agent_input = format!(
        "[Channel: Discord (channel_id: {channel_id}) — your text response is automatically sent to this channel. \
         Do NOT call discord_send to deliver your answer. Only use discord_send for: \
         sending to a different channel, embeds, reactions, threads, files, or moderation.]\n{agent_input}"
    );

    // Register channel for approval routing, then send with approval callback
    discord_state
        .register_session_channel(session_id, msg.channel_id.get())
        .await;

    // Claim this session's background-task completions for Discord: a completion
    // must be delivered by the surface that OWNS the session, not by whichever
    // service happened to run the command (#940).
    crate::brain::agent::service::session_routes::claim_for_channel(
        session_id,
        agent.message_enqueue_callback(),
    );
    let approval_cb = make_approval_callback(discord_state.clone());

    let cancel_token = tokio_util::sync::CancellationToken::new();
    discord_state
        .store_cancel_token(session_id, cancel_token.clone())
        .await;

    // Sustained typing for the turn, continuing while the session has detached
    // work (#812). Discord had no turn-long pinger at all, so an ordinary turn
    // showed the dots briefly and a background command showed nothing: spawning
    // one ENDS the turn. Its own token, not `cancel_token`, which only fires on
    // abort — this must stop when the turn FINISHES, however it finishes.
    let typing_cancel = tokio_util::sync::CancellationToken::new();
    super::typing::spawn_typing(
        ctx.http.clone(),
        msg.channel_id,
        typing_cancel.clone(),
        agent.background_manager(),
        session_id,
    );
    let _typing_guard = super::typing::TypingGuard(typing_cancel);

    let sent_intermediates: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    // Build progress callback — sends tool call status as Discord messages
    let progress_cb: crate::brain::agent::ProgressCallback = {
        use crate::brain::agent::ProgressEvent;
        use serenity::builder::EditMessage;
        use serenity::model::id::MessageId;

        use super::tool_group::{GroupEntry, GroupState};

        let tools: Arc<Mutex<Vec<GroupEntry>>> = Arc::new(Mutex::new(Vec::new()));
        let group_msg_id: Arc<Mutex<Option<MessageId>>> = Arc::new(Mutex::new(None));
        let group_state_cb = discord_state.clone();
        let http = ctx.http.clone();
        let channel = msg.channel_id;

        Arc::new(move |session_id, event| {
            let tools = tools.clone();
            let http = http.clone();

            match event {
                // Auto-compaction produces zero streaming chunks for
                // 10-60s and Discord has no continuous typing pinger
                // like Telegram. Ping broadcast_typing every 8s for up
                // to 90s so the channel shows the "is typing" dots
                // through the silent window. No text — just the native
                // indicator. The loop self-terminates after 90s; if
                // compaction finishes earlier, real streaming chunks
                // resume the indicator naturally.
                ProgressEvent::Compacting { .. } => {
                    let http = http.clone();
                    tokio::spawn(async move {
                        for _ in 0..12 {
                            if let Err(e) = channel.broadcast_typing(&http).await {
                                tracing::warn!(error = %e, "failed to broadcast Discord typing");
                            }
                            tokio::time::sleep(std::time::Duration::from_secs(8)).await;
                        }
                    });
                }
                ProgressEvent::ToolStarted {
                    tool_name,
                    tool_input,
                } => {
                    let ctx_hint = crate::utils::tool_context_hint(&tool_name, &tool_input);
                    let gmid = group_msg_id.clone();
                    let dstate = group_state_cb.clone();
                    tokio::spawn(async move {
                        // One grouped message per turn (#380), collapsed by
                        // default with an Expand toggle; edited in place.
                        let entries = {
                            let mut t = tools.lock().await;
                            t.push(GroupEntry {
                                name: tool_name,
                                context: ctx_hint,
                                status: None,
                            });
                            t.clone()
                        };
                        let mut mid_guard = gmid.lock().await;
                        match *mid_guard {
                            Some(mid) => {
                                let group = dstate
                                    .upsert_tool_group(
                                        mid.get(),
                                        GroupState {
                                            entries,
                                            expanded: false,
                                        },
                                    )
                                    .await;
                                let edit = EditMessage::new()
                                    .content(super::tool_group::render_content(&group))
                                    .components(super::tool_group::render_components(
                                        &group,
                                        mid.get(),
                                    ));
                                if let Err(e) = channel.edit_message(&http, mid, edit).await {
                                    tracing::warn!("Discord: tool group edit failed (append): {e}");
                                }
                            }
                            None => {
                                let group = GroupState {
                                    entries,
                                    expanded: false,
                                };
                                let content = super::tool_group::render_content(&group);
                                match channel.say(&http, &content).await {
                                    Ok(sent) => {
                                        let comps = super::tool_group::render_components(
                                            &group,
                                            sent.id.get(),
                                        );
                                        if !comps.is_empty()
                                            && let Err(e) = channel
                                                .edit_message(
                                                    &http,
                                                    sent.id,
                                                    EditMessage::new().components(comps),
                                                )
                                                .await
                                        {
                                            tracing::warn!(
                                                "Discord: tool group component fixup failed: {e}"
                                            );
                                        }
                                        dstate.upsert_tool_group(sent.id.get(), group).await;
                                        *mid_guard = Some(sent.id);
                                    }
                                    Err(e) => tracing::warn!(
                                        "Discord: failed to post tool group message: {e}"
                                    ),
                                }
                            }
                        }
                    });
                }
                ProgressEvent::ToolCompleted {
                    tool_name, success, ..
                } => {
                    let gmid = group_msg_id.clone();
                    let dstate = group_state_cb.clone();
                    tokio::spawn(async move {
                        let entries = {
                            let mut t = tools.lock().await;
                            if let Some(entry) = t
                                .iter_mut()
                                .rev()
                                .find(|e| e.name == tool_name && e.status.is_none())
                            {
                                entry.status = Some(success);
                            }
                            t.clone()
                        };
                        if let Some(mid) = *gmid.lock().await {
                            let group = dstate
                                .upsert_tool_group(
                                    mid.get(),
                                    GroupState {
                                        entries,
                                        expanded: false,
                                    },
                                )
                                .await;
                            let edit = EditMessage::new()
                                .content(super::tool_group::render_content(&group))
                                .components(super::tool_group::render_components(
                                    &group,
                                    mid.get(),
                                ));
                            if let Err(e) = channel.edit_message(&http, mid, edit).await {
                                tracing::warn!("Discord: tool group edit failed (status): {e}");
                            }
                        }
                    });
                }
                ProgressEvent::SelfHealingAlert { message } => {
                    tokio::spawn(async move {
                        let text = format!("🔧 {}", message);
                        if let Err(e) = channel.say(&http, &text).await {
                            tracing::warn!(error = %e, "failed to send Discord message");
                        }
                    });
                }
                ProgressEvent::IntermediateText { text, .. } => {
                    // Strip LLM artifacts, secrets, and media markers
                    // the same way the final-response path does.
                    let clean = crate::utils::sanitize::strip_llm_artifacts(&text);
                    let clean = redact_secrets(&clean);
                    let (clean, _) = crate::utils::extract_img_markers(&clean);
                    let (clean, _) = crate::utils::extract_vid_markers(&clean);
                    if clean.trim().is_empty() {
                        return;
                    }
                    let sent = sent_intermediates.clone();
                    let http = http.clone();
                    let channel = channel;
                    tokio::spawn(async move {
                        // Pre-send dedup: Discord doesn't support edit-
                        // in-place dedup across messages, so skip if
                        // this exact body was already posted.
                        {
                            let mut prev = sent.lock().await;
                            if prev.iter().any(|s| s == &clean) {
                                return;
                            }
                            prev.push(clean.clone());
                        }
                        for chunk in split_message(&clean, 2000) {
                            if let Err(e) = channel.say(&http, chunk).await {
                                tracing::debug!("Discord: intermediate text send failed: {}", e);
                            }
                        }
                    });
                }
                ProgressEvent::RetryAttempt {
                    attempt,
                    max,
                    reason,
                } => {
                    let http = http.clone();
                    let channel = channel;
                    tokio::spawn(async move {
                        let text = format!("⏳ Retry {}/{} — {}", attempt, max, reason);
                        if let Err(e) = channel.say(&http, &text).await {
                            tracing::warn!(error = %e, "failed to send Discord message");
                        }
                    });
                }
                ProgressEvent::ProviderSwitched {
                    to_name, to_model, ..
                } => {
                    let http = http.clone();
                    let channel = channel;
                    tokio::spawn(async move {
                        let text = format!("🔄 Now using {}/{}", to_name, to_model);
                        if let Err(e) = channel.say(&http, &text).await {
                            tracing::warn!(error = %e, "failed to send Discord message");
                        }
                    });
                }
                // Optional follow-up suggestions (#598): post tap-to-send
                // buttons under the response. A tap injects the suggestion as a
                // new turn via route_interaction_turn.
                ProgressEvent::SuggestedOptions(options) => {
                    let http = http.clone();
                    let state = group_state_cb.clone();
                    tokio::spawn(async move {
                        super::suggest_options::render_suggestions(
                            &http, &state, session_id, options,
                        )
                        .await;
                    });
                }
                _ => {}
            }
        })
    };

    let discord_chat_id = msg.channel_id.get().to_string();
    let result = agent
        .send_message_with_tools_and_display(
            session_id,
            agent_input,
            Some(display_text),
            None,
            Some(cancel_token),
            Some(approval_cb),
            Some(progress_cb),
            "discord",
            Some(&discord_chat_id),
            None, // Discord threads: not tracked yet
        )
        .await;

    discord_state.remove_cancel_token(session_id).await;

    match result {
        Ok(response) => {
            // Extract <<IMG:path>> markers — send each as a Discord file attachment.
            let (response_content, react_emoji) =
                crate::utils::extract_react_marker(&response.content);
            // React-back (#381): fire the reaction on the user's message
            // instead of leaking the marker into Discord text.
            if let Some(ref em) = react_emoji {
                use serenity::model::channel::ReactionType;
                let em = em.trim().to_string();
                if let Err(e) = msg
                    .react(&ctx.http, ReactionType::Unicode(em.clone()))
                    .await
                {
                    tracing::warn!("Discord: react-back {em} failed: {e}");
                }
            }
            let (text_only, img_paths) = crate::utils::extract_img_markers(&response_content);
            let text_only = crate::utils::sanitize::strip_llm_artifacts(&text_only);
            let text_only = redact_secrets(&text_only);

            // Context budget footer appended to last display chunk, never stored in DB
            let ctx_max = agent.context_limit_for_session(session_id);
            let footer = crate::utils::format_ctx_footer(
                response.context_tokens,
                ctx_max,
                response.tokens_per_second,
            );

            // Media gallery (#385): batch all generated files into ONE
            // multi-attachment message (Discord caps 10 per message; the
            // remainder rolls into follow-up batches) instead of one
            // message per file.
            let mut attachments: Vec<CreateAttachment> = Vec::new();
            for img_path in &img_paths {
                match tokio::fs::read(img_path).await {
                    Ok(bytes) => {
                        let fname = std::path::Path::new(img_path)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("image.png")
                            .to_string();
                        attachments.push(CreateAttachment::bytes(bytes, fname));
                    }
                    Err(e) => {
                        tracing::error!("Discord: failed to read image {}: {}", img_path, e);
                    }
                }
            }
            for batch in attachments.chunks(10) {
                let mut message = CreateMessage::new();
                for file in batch {
                    message = message.add_file(file.clone());
                }
                if let Err(e) = msg.channel_id.send_message(&ctx.http, message).await {
                    tracing::error!("Discord: failed to send media gallery batch: {}", e);
                }
            }

            let mut chunks: Vec<String> = split_message(&text_only, 2000)
                .into_iter()
                .map(|s| s.to_string())
                .collect();
            // Append footer to last display chunk so it's inline, not a separate message
            if let Some(last) = chunks.last_mut() {
                last.push_str("\n\n");
                last.push_str(&footer);
            } else if !footer.is_empty() {
                chunks.push(footer);
            }
            for chunk in &chunks {
                if let Err(e) = msg.channel_id.say(&ctx.http, chunk).await {
                    tracing::error!("Discord: failed to send reply: {}", e);
                }
            }

            // Record the bot's reply in channel_messages so the recent() query
            // used for group context on the next guild turn sees both sides of
            // the conversation. Without this, the bot loads only user messages
            // and responds blind to its own prior replies. Skip for DMs — the
            // session's messages table already carries full history there.
            if !is_dm && !text_only.trim().is_empty() {
                let bot_id = discord_state.bot_user_id().await;
                let bot_sender_id = bot_id
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "bot:opencrabs".to_string());
                let guild_name = msg
                    .guild_id
                    .map(|g| g.get().to_string())
                    .unwrap_or_else(|| "DM".to_string());
                let cm = DbChannelMessage::new(
                    "discord".into(),
                    msg.channel_id.get().to_string(),
                    Some(guild_name),
                    bot_sender_id,
                    "OpenCrabs".into(),
                    text_only.clone(),
                    "text".into(),
                    None,
                );
                if let Err(e) = channel_msg_repo.insert(&cm).await {
                    tracing::warn!(
                        "Discord: failed to record bot reply in channel_messages: {}",
                        e
                    );
                }
            }

            // TTS: send voice reply if input was audio and TTS is enabled
            if is_voice && voice_config.tts_enabled {
                match crate::channels::voice::synthesize(&response.content, &voice_config).await {
                    Ok(audio_bytes) => {
                        let file = CreateAttachment::bytes(audio_bytes.as_slice(), "response.ogg");
                        if let Err(e) = msg
                            .channel_id
                            .send_message(&ctx.http, CreateMessage::new().add_file(file))
                            .await
                        {
                            tracing::error!("Discord: failed to send TTS voice: {e}");
                        }
                    }
                    Err(e) => tracing::error!("Discord: TTS error: {e}"),
                }
            }
        }
        Err(ref e) if matches!(e, crate::brain::agent::AgentError::Cancelled) => {
            tracing::info!("Discord: agent call cancelled for session {}", session_id);
        }
        Err(e) => {
            tracing::error!("Discord: agent error: {}", e);
            // Shared helper translates the raw error into something
            // the user can act on (5xx exhausted, rate limit, context
            // too large, stream broken, repetition loop). Same wording
            // as the TUI + Telegram + Slack + WhatsApp paths.
            let error_msg = format!("❌ Error\n\n{}", crate::brain::agent::format_user_error(&e));
            if let Err(e) = msg.channel_id.say(&ctx.http, error_msg).await {
                tracing::warn!(error = %e, "failed to send Discord message");
            }
        }
    }
}

/// Build an `ApprovalCallback` that sends a Discord message with 3 buttons
/// (Yes / Always / No) and waits up to 5 min for a click.
pub(crate) fn make_approval_callback(
    state: Arc<super::DiscordState>,
) -> crate::brain::agent::ApprovalCallback {
    use crate::brain::agent::ToolApprovalInfo;
    use crate::utils::{check_approval_policy, persist_auto_session_policy};
    use serenity::builder::{CreateActionRow, CreateButton, CreateMessage, EditMessage};
    use serenity::model::application::ButtonStyle;
    use serenity::model::id::ChannelId;
    use tokio::sync::oneshot;

    Arc::new(move |info: ToolApprovalInfo| {
        let state = state.clone();
        Box::pin(async move {
            if let Some(result) = check_approval_policy() {
                return Ok(result);
            }

            let http = match state.http().await {
                Some(h) => h,
                None => {
                    tracing::warn!("Discord approval: bot not connected");
                    return Ok((false, false));
                }
            };

            let channel_id = match state.session_channel(info.session_id).await {
                Some(id) => id,
                None => match state.owner_channel_id().await {
                    Some(id) => id,
                    None => {
                        tracing::warn!(
                            "Discord approval: no channel_id for session {}",
                            info.session_id
                        );
                        return Ok((false, false));
                    }
                },
            };

            let approval_id = uuid::Uuid::new_v4().to_string();
            let safe_input = crate::utils::redact_tool_input(&info.tool_input);
            let input_pretty = serde_json::to_string_pretty(&safe_input)
                .unwrap_or_else(|_| safe_input.to_string());
            let text = format!(
                "🔐 **Tool Approval Required**\n\nTool: `{}`\nInput:\n```json\n{}\n```",
                info.tool_name,
                truncate_str(&input_pretty, 1800),
            );

            let row = CreateActionRow::Buttons(vec![
                CreateButton::new(format!("approve:{}", approval_id))
                    .label("✅ Yes")
                    .style(ButtonStyle::Success),
                CreateButton::new(format!("always:{}", approval_id))
                    .label("🔁 Always (session)")
                    .style(ButtonStyle::Primary),
                CreateButton::new(format!("yolo:{}", approval_id))
                    .label("🔥 YOLO")
                    .style(ButtonStyle::Secondary),
                CreateButton::new(format!("deny:{}", approval_id))
                    .label("❌ No")
                    .style(ButtonStyle::Danger),
            ]);

            // Register BEFORE sending to prevent race condition
            let (tx, rx) = oneshot::channel();
            state
                .register_pending_approval(approval_id.clone(), tx)
                .await;
            tracing::info!(
                "Discord approval: registered pending id={}, sending to channel={}",
                approval_id,
                channel_id
            );

            let mut sent_msg = match ChannelId::new(channel_id)
                .send_message(
                    &http,
                    CreateMessage::new().content(&text).components(vec![row]),
                )
                .await
            {
                Ok(m) => m,
                Err(e) => {
                    tracing::error!("Discord approval: failed to send message: {}", e);
                    return Ok((false, false));
                }
            };

            tracing::info!(
                "Discord approval: message sent, waiting for response (id={})",
                approval_id
            );

            match tokio::time::timeout(std::time::Duration::from_secs(300), rx).await {
                Ok(Ok((approved, always))) => {
                    tracing::info!(
                        "Discord approval: user responded id={}, approved={}, always={}",
                        approval_id,
                        approved,
                        always
                    );
                    if always {
                        persist_auto_session_policy();
                    }
                    let label = if always {
                        "🔁 Always approved (session)"
                    } else if approved {
                        "✅ Approved"
                    } else {
                        "❌ Denied"
                    };
                    if let Err(e) = sent_msg
                        .edit(&http, EditMessage::new().content(label).components(vec![]))
                        .await
                    {
                        tracing::warn!(error = %e, "failed to edit Discord approval button");
                    }
                    Ok((approved, always))
                }
                Ok(Err(_)) => {
                    tracing::warn!(
                        "Discord approval: oneshot channel closed (id={})",
                        approval_id
                    );
                    Ok((false, false))
                }
                Err(_) => {
                    tracing::warn!(
                        "Discord approval: 5-minute timeout — auto-denying (id={})",
                        approval_id
                    );
                    if let Err(e) = sent_msg
                        .edit(
                            &http,
                            EditMessage::new()
                                .content("⏱️ Approval timed out — denied")
                                .components(vec![]),
                        )
                        .await
                    {
                        tracing::warn!(error = %e, "failed to edit Discord timeout message");
                    }
                    Ok((false, false))
                }
            }
        })
    })
}
