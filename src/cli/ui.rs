//! TUI chat startup — provider init, tool registry, approval callbacks, Telegram spawn.

use anyhow::{Context, Result};
use std::sync::Arc;

use crate::brain::prompt_builder::RuntimeInfo;
use crate::brain::{BrainLoader, CommandLoader};

/// Start interactive chat session
pub(crate) async fn cmd_chat(
    config: &crate::config::Config,
    session_id: Option<String>,
    force_onboard: bool,
) -> Result<()> {
    use crate::{
        brain::{
            agent::AgentService,
            tools::{
                bash::BashTool, brave_search::BraveSearchTool, code_exec::CodeExecTool,
                config_tool::ConfigTool, context::ContextTool, doc_parser::DocParserTool,
                edit::EditTool, exa_search::ExaSearchTool, glob::GlobTool, grep::GrepTool,
                http::HttpClientTool, ls::LsTool, memory_search::MemorySearchTool,
                notebook::NotebookEditTool, plan_tool::PlanTool, read::ReadTool,
                registry::ToolRegistry, session_search::SessionSearchTool,
                slash_command::SlashCommandTool, task::TaskTool, web_search::WebSearchTool,
                write::WriteTool,
            },
        },
        db::Database,
        services::ServiceContext,
        tui,
    };

    {
        const STARTS: &[&str] = &[
            "🦀 Crabs assemble!",
            "🦀 *sideways scuttling intensifies*",
            "🦀 Booting crab consciousness...",
            "🦀 Who summoned the crabs?",
            "🦀 Crab rave initiated.",
            "🦀 The crabs have awakened.",
            "🦀 Emerging from the deep...",
            "🦀 All systems crabby.",
            "🦀 Let's get cracking.",
            "🦀 Rustacean reporting for duty.",
        ];
        let i = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos() as usize)
            % STARTS.len();
        let orange = "\x1b[38;2;215;100;20m";
        let reset = "\x1b[0m";
        println!("\n{}{}{}", orange, STARTS[i], reset);
    }

    // Initialize database
    tracing::info!("Connecting to database: {}", config.database.path.display());
    let db = Database::connect(&config.database.path)
        .await
        .context("Failed to connect to database")?;

    // Run migrations
    db.run_migrations()
        .await
        .context("Failed to run database migrations")?;

    // Select provider based on configuration using factory
    // Returns placeholder provider if none configured, so app can start and show onboarding
    let provider = crate::brain::provider::create_provider(config)?;
    tracing::info!("Using provider: {}", provider.name());

    // Create tool registry
    tracing::debug!("Setting up tool registry");
    let mut tool_registry = ToolRegistry::new();
    // Phase 1: Essential file operations
    tool_registry.register(Arc::new(ReadTool));
    tool_registry.register(Arc::new(WriteTool));
    tool_registry.register(Arc::new(EditTool));
    tool_registry.register(Arc::new(BashTool));
    tool_registry.register(Arc::new(LsTool));
    tool_registry.register(Arc::new(GlobTool));
    tool_registry.register(Arc::new(GrepTool));
    // Phase 2: Advanced features
    tool_registry.register(Arc::new(WebSearchTool));
    tool_registry.register(Arc::new(CodeExecTool));
    tool_registry.register(Arc::new(NotebookEditTool));
    tool_registry.register(Arc::new(DocParserTool));
    // Phase 3: Workflow & integration
    tool_registry.register(Arc::new(TaskTool));
    tool_registry.register(Arc::new(ContextTool));
    tool_registry.register(Arc::new(HttpClientTool));
    tool_registry.register(Arc::new(PlanTool));
    // Memory search (built-in FTS5, always available)
    tool_registry.register(Arc::new(MemorySearchTool));
    // Session search — hybrid QMD search across all session message history
    tool_registry.register(Arc::new(SessionSearchTool::new(db.pool().clone())));
    // Config management (read/write config.toml, commands.toml)
    tool_registry.register(Arc::new(ConfigTool));
    // Slash command invocation (agent can call any slash command)
    tool_registry.register(Arc::new(SlashCommandTool));
    // EXA search: always available (free via MCP), uses direct API if key is set
    let exa_key = config
        .providers
        .web_search
        .as_ref()
        .and_then(|ws| ws.exa.as_ref())
        .and_then(|p| p.api_key.clone())
        .filter(|k| !k.is_empty());
    let exa_mode = if exa_key.is_some() {
        "direct API"
    } else {
        "MCP (free)"
    };
    tool_registry.register(Arc::new(ExaSearchTool::new(exa_key)));
    tracing::info!("Registered EXA search tool (mode: {})", exa_mode);
    // Brave search: requires enabled = true in config.toml AND API key in keys.toml
    if let Some(brave_cfg) = config
        .providers
        .web_search
        .as_ref()
        .and_then(|ws| ws.brave.as_ref())
        && brave_cfg.enabled
        && let Some(brave_key) = brave_cfg.api_key.clone()
    {
        tool_registry.register(Arc::new(BraveSearchTool::new(brave_key)));
        tracing::info!("Registered Brave search tool");
    }

    // Index existing memory files and warm up embedding engine in the background
    tokio::spawn(async {
        match crate::memory::get_store() {
            Ok(store) => match crate::memory::reindex(store).await {
                Ok(n) => tracing::info!("Startup memory reindex: {n} files"),
                Err(e) => tracing::warn!("Startup memory reindex failed: {e}"),
            },
            Err(e) => tracing::warn!("Memory store init failed at startup: {e}"),
        }
        // Warm up embedding engine so first search doesn't pay model download cost.
        // reindex() already calls get_engine() during backfill, but if all docs were
        // already embedded, this ensures the engine is ready for search.
        match tokio::task::spawn_blocking(crate::memory::get_engine).await {
            Ok(Ok(_)) => tracing::info!("Embedding engine warmed up"),
            Ok(Err(e)) => tracing::warn!("Embedding engine init skipped: {e}"),
            Err(e) => tracing::warn!("Embedding engine warmup failed: {e}"),
        }
    });

    // Create service context
    let service_context = ServiceContext::new(db.pool().clone());

    // Get working directory
    let working_directory = std::env::current_dir().unwrap_or_default();

    // Build dynamic system brain from workspace files
    let brain_path = BrainLoader::resolve_path();
    let brain_loader = BrainLoader::new(brain_path.clone());
    let command_loader = CommandLoader::from_brain_path(&brain_path);
    let user_commands = command_loader.load();

    let runtime_info = RuntimeInfo {
        model: Some(provider.default_model().to_string()),
        provider: Some(provider.name().to_string()),
        working_directory: Some(working_directory.to_string_lossy().to_string()),
    };

    let builtin_commands: Vec<(&str, &str)> = crate::tui::app::SLASH_COMMANDS
        .iter()
        .map(|c| (c.name, c.description))
        .collect();
    let commands_section = CommandLoader::commands_section(&builtin_commands, &user_commands);

    let system_brain =
        brain_loader.build_system_brain(Some(&runtime_info), Some(&commands_section));

    // Create agent service with dynamic system brain
    let agent_service = Arc::new(
        AgentService::new(provider.clone(), service_context.clone())
            .with_system_brain(system_brain.clone())
            .with_working_directory(working_directory.clone()),
    );

    // Create TUI app first (so we can get the event sender)
    tracing::debug!("Creating TUI app");
    let mut app = tui::App::new(agent_service, service_context.clone());

    // Get event sender from app
    let event_sender = app.event_sender();

    // Create approval callback that sends requests to TUI
    let approval_callback: crate::brain::agent::ApprovalCallback = Arc::new(move |tool_info| {
        let sender = event_sender.clone();
        Box::pin(async move {
            use crate::tui::events::{ToolApprovalRequest, TuiEvent};
            use tokio::sync::mpsc;

            // Create response channel
            let (response_tx, mut response_rx) = mpsc::unbounded_channel();

            // Create approval request
            let request = ToolApprovalRequest {
                request_id: uuid::Uuid::new_v4(),
                session_id: tool_info.session_id,
                tool_name: tool_info.tool_name,
                tool_description: tool_info.tool_description,
                tool_input: tool_info.tool_input,
                capabilities: tool_info.capabilities,
                response_tx,
                requested_at: std::time::Instant::now(),
            };

            // Send to TUI
            sender
                .send(TuiEvent::ToolApprovalRequested(request))
                .map_err(|e| {
                    crate::brain::agent::AgentError::Internal(format!(
                        "Failed to send approval request: {}",
                        e
                    ))
                })?;

            // Wait for response with timeout to prevent indefinite hang
            let response =
                tokio::time::timeout(std::time::Duration::from_secs(120), response_rx.recv())
                    .await
                    .map_err(|_| {
                        tracing::warn!("Approval request timed out after 120s, auto-denying");
                        crate::brain::agent::AgentError::Internal(
                            "Approval request timed out (120s) — auto-denied".to_string(),
                        )
                    })?
                    .ok_or_else(|| {
                        tracing::warn!("Approval response channel closed unexpectedly");
                        crate::brain::agent::AgentError::Internal(
                            "Approval response channel closed".to_string(),
                        )
                    })?;

            Ok(response.approved)
        })
    });

    // Create progress callback that sends tool events to TUI
    let progress_sender = app.event_sender();
    let progress_callback: crate::brain::agent::ProgressCallback =
        Arc::new(move |session_id, event| {
            use crate::brain::agent::ProgressEvent;
            use crate::tui::events::TuiEvent;

            let result = match event {
                ProgressEvent::ToolStarted {
                    tool_name,
                    tool_input,
                } => progress_sender.send(TuiEvent::ToolCallStarted {
                    session_id,
                    tool_name,
                    tool_input,
                }),
                ProgressEvent::ToolCompleted {
                    tool_name,
                    tool_input,
                    success,
                    summary,
                } => progress_sender.send(TuiEvent::ToolCallCompleted {
                    session_id,
                    tool_name,
                    tool_input,
                    success,
                    summary,
                }),
                ProgressEvent::IntermediateText { text, reasoning } => {
                    progress_sender.send(TuiEvent::IntermediateText {
                        session_id,
                        text,
                        reasoning,
                    })
                }
                ProgressEvent::StreamingChunk { text } => {
                    progress_sender.send(TuiEvent::ResponseChunk { session_id, text })
                }
                ProgressEvent::Thinking => return, // spinner handles this already
                ProgressEvent::Compacting => progress_sender.send(TuiEvent::AgentProcessing),
                ProgressEvent::CompactionSummary { summary } => {
                    progress_sender.send(TuiEvent::CompactionSummary {
                        session_id,
                        summary,
                    })
                }
                ProgressEvent::RestartReady { status } => {
                    progress_sender.send(TuiEvent::RestartReady(status))
                }
                ProgressEvent::TokenCount(count) => {
                    progress_sender.send(TuiEvent::TokenCountUpdated { session_id, count })
                }
                ProgressEvent::ReasoningChunk { text } => {
                    progress_sender.send(TuiEvent::ReasoningChunk { session_id, text })
                }
            };
            if let Err(e) = result {
                tracing::error!("Progress event channel closed: {}", e);
            }
        });

    // Create message queue callback that checks for queued user messages
    let message_queue = app.message_queue.clone();
    let message_queue_callback: crate::brain::agent::MessageQueueCallback = Arc::new(move || {
        let queue = message_queue.clone();
        Box::pin(async move { queue.lock().await.take() })
    });

    // Register rebuild tool (needs the progress callback for restart signaling)
    tool_registry.register(Arc::new(crate::brain::tools::rebuild::RebuildTool::new(
        Some(progress_callback.clone()),
    )));

    // Create ChannelFactory (shared by static channel spawn + WhatsApp connect tool).
    // Tool registry is set lazily after Arc wrapping to break circular dependency.
    let openai_tts_key = config
        .providers
        .tts
        .as_ref()
        .and_then(|t| t.openai.as_ref())
        .and_then(|p| p.api_key.clone());
    let channel_factory = Arc::new(crate::channels::ChannelFactory::new(
        provider.clone(),
        service_context.clone(),
        system_brain.clone(),
        working_directory.clone(),
        brain_path.clone(),
        app.shared_session_id(),
        config.voice.clone(),
        openai_tts_key,
    ));

    // Shared Telegram state for proactive messaging
    #[cfg(feature = "telegram")]
    let telegram_state = Arc::new(crate::channels::telegram::TelegramState::new());

    // Register Telegram connect tool (agent-callable bot setup)
    #[cfg(feature = "telegram")]
    tool_registry.register(Arc::new(
        crate::brain::tools::telegram_connect::TelegramConnectTool::new(
            channel_factory.clone(),
            telegram_state.clone(),
        ),
    ));

    // Register Telegram send tool (proactive messaging)
    #[cfg(feature = "telegram")]
    tool_registry.register(Arc::new(
        crate::brain::tools::telegram_send::TelegramSendTool::new(telegram_state.clone()),
    ));

    // Shared WhatsApp state for proactive messaging (connect + send tools + static agent)
    #[cfg(feature = "whatsapp")]
    let whatsapp_state = Arc::new(crate::channels::whatsapp::WhatsAppState::new());

    // Register WhatsApp connect tool (agent-callable QR pairing)
    #[cfg(feature = "whatsapp")]
    tool_registry.register(Arc::new(
        crate::brain::tools::whatsapp_connect::WhatsAppConnectTool::new(
            Some(progress_callback.clone()),
            channel_factory.clone(),
            whatsapp_state.clone(),
        ),
    ));

    // Register WhatsApp send tool (proactive messaging)
    #[cfg(feature = "whatsapp")]
    tool_registry.register(Arc::new(
        crate::brain::tools::whatsapp_send::WhatsAppSendTool::new(whatsapp_state.clone()),
    ));

    // Shared Discord state for proactive messaging
    #[cfg(feature = "discord")]
    let discord_state = Arc::new(crate::channels::discord::DiscordState::new());

    // Register Discord connect tool (agent-callable bot setup)
    #[cfg(feature = "discord")]
    tool_registry.register(Arc::new(
        crate::brain::tools::discord_connect::DiscordConnectTool::new(
            channel_factory.clone(),
            discord_state.clone(),
        ),
    ));

    // Register Discord send tool (proactive messaging)
    #[cfg(feature = "discord")]
    tool_registry.register(Arc::new(
        crate::brain::tools::discord_send::DiscordSendTool::new(discord_state.clone()),
    ));

    // Shared Slack state for proactive messaging
    #[cfg(feature = "slack")]
    let slack_state = Arc::new(crate::channels::slack::SlackState::new());

    // Register Slack connect tool (agent-callable bot setup)
    #[cfg(feature = "slack")]
    tool_registry.register(Arc::new(
        crate::brain::tools::slack_connect::SlackConnectTool::new(
            channel_factory.clone(),
            slack_state.clone(),
        ),
    ));

    // Register Slack send tool (proactive messaging)
    #[cfg(feature = "slack")]
    tool_registry.register(Arc::new(
        crate::brain::tools::slack_send::SlackSendTool::new(slack_state.clone()),
    ));

    // Create sudo password callback that sends requests to TUI
    let sudo_sender = app.event_sender();
    let sudo_callback: crate::brain::agent::SudoCallback = Arc::new(move |command| {
        let sender = sudo_sender.clone();
        Box::pin(async move {
            use crate::tui::events::{SudoPasswordRequest, SudoPasswordResponse, TuiEvent};
            use tokio::sync::mpsc;

            let (response_tx, mut response_rx) = mpsc::unbounded_channel::<SudoPasswordResponse>();

            let request = SudoPasswordRequest {
                request_id: uuid::Uuid::new_v4(),
                command,
                response_tx,
            };

            sender
                .send(TuiEvent::SudoPasswordRequested(request))
                .map_err(|e| {
                    crate::brain::agent::AgentError::Internal(format!(
                        "Failed to send sudo request: {}",
                        e
                    ))
                })?;

            // Wait for user response with timeout
            let response =
                tokio::time::timeout(std::time::Duration::from_secs(120), response_rx.recv())
                    .await
                    .map_err(|_| {
                        crate::brain::agent::AgentError::Internal(
                            "Sudo password request timed out (120s)".to_string(),
                        )
                    })?
                    .ok_or_else(|| {
                        crate::brain::agent::AgentError::Internal(
                            "Sudo password channel closed".to_string(),
                        )
                    })?;

            Ok(response.password)
        })
    });

    // Create agent service with approval callback, progress callback, and message queue
    tracing::debug!("Creating agent service with approval, progress, and message queue callbacks");
    let shared_tool_registry = Arc::new(tool_registry);

    // Now that the registry is Arc'd, give it to the channel factory
    channel_factory.set_tool_registry(shared_tool_registry.clone());

    let agent_service = Arc::new(
        AgentService::new(provider.clone(), service_context.clone())
            .with_system_brain(system_brain)
            .with_tool_registry(shared_tool_registry.clone())
            .with_approval_callback(Some(approval_callback))
            .with_progress_callback(Some(progress_callback))
            .with_message_queue_callback(Some(message_queue_callback))
            .with_sudo_callback(Some(sudo_callback))
            .with_working_directory(working_directory.clone())
            .with_brain_path(brain_path),
    );

    // Update app with the configured agent service (preserve event channels!)
    app.set_agent_service(agent_service);

    // Set force onboard flag if requested
    if force_onboard {
        app.force_onboard = true;
    }

    // Resume a specific session (e.g. after /rebuild restart)
    if let Some(ref sid) = session_id
        && let Ok(uuid) = uuid::Uuid::parse_str(sid)
    {
        app.resume_session_id = Some(uuid);
    }

    // Spawn A2A gateway if configured
    if config.a2a.enabled {
        let a2a_agent = channel_factory.create_agent_service();
        let a2a_ctx = service_context.clone();
        let a2a_config = config.a2a.clone();
        tokio::spawn(async move {
            if let Err(e) = crate::a2a::server::start_server(&a2a_config, a2a_agent, a2a_ctx).await
            {
                tracing::error!("A2A gateway error: {}", e);
            }
        });
    }

    // Spawn Telegram bot if configured
    #[cfg(feature = "telegram")]
    let _telegram_handle = {
        let tg = &config.channels.telegram;
        let tg_token = tg.token.clone();
        let has_valid_token = tg_token
            .as_ref()
            .map(|t| {
                if t.is_empty() || !t.contains(':') {
                    return false;
                }
                let parts: Vec<&str> = t.splitn(2, ':').collect();
                parts.len() == 2 && parts[0].parse::<u64>().is_ok() && parts[1].len() >= 30
            })
            .unwrap_or(false);

        tracing::debug!(
            "[Telegram] enabled={}, has_token={}, has_valid_token={}",
            tg.enabled,
            tg_token.is_some(),
            has_valid_token
        );

        if tg.enabled && has_valid_token {
            if let Some(ref token) = tg_token {
                let tg_agent = channel_factory.create_agent_service();
                // Extract OpenAI API key for TTS (from providers.tts.openai)
                let openai_key = config
                    .providers
                    .tts
                    .as_ref()
                    .and_then(|t| t.openai.as_ref())
                    .and_then(|p| p.api_key.clone());
                // Extract STT provider config from providers.stt.*
                let mut voice_cfg = config.voice.clone();
                voice_cfg.stt_provider = config.providers.stt.as_ref().and_then(|s| s.groq.clone());
                voice_cfg.tts_provider =
                    config.providers.tts.as_ref().and_then(|t| t.openai.clone());
                let bot = crate::channels::telegram::TelegramAgent::new(
                    tg_agent,
                    service_context.clone(),
                    tg.allowed_users.clone(),
                    voice_cfg,
                    openai_key,
                    app.shared_session_id(),
                    telegram_state.clone(),
                    tg.respond_to.clone(),
                    tg.allowed_channels.clone(),
                );
                tracing::info!(
                    "Spawning Telegram bot ({} allowed users)",
                    tg.allowed_users.len()
                );
                Some(bot.start(token.clone()))
            } else {
                tracing::debug!("Telegram enabled but no valid token configured");
                None
            }
        } else {
            None
        }
    };

    // Spawn WhatsApp agent if configured (already paired via session.db)
    #[cfg(feature = "whatsapp")]
    let _whatsapp_handle = {
        let wa = &config.channels.whatsapp;
        if wa.enabled {
            let wa_agent = crate::channels::whatsapp::WhatsAppAgent::new(
                channel_factory.create_agent_service(),
                service_context.clone(),
                wa.allowed_phones.clone(),
                config.voice.clone(),
                app.shared_session_id(),
                whatsapp_state.clone(),
            );
            tracing::info!(
                "Spawning WhatsApp agent ({} allowed phones)",
                wa.allowed_phones.len()
            );
            Some(wa_agent.start())
        } else {
            None
        }
    };

    // Spawn Discord bot if configured (token-based, like Telegram)
    #[cfg(feature = "discord")]
    let _discord_handle = {
        let dc = &config.channels.discord;
        let dc_token = dc.token.clone();
        // Discord tokens are typically ~70 chars, base64-like
        let has_valid_token = dc_token
            .as_ref()
            .map(|t| !t.is_empty() && t.len() > 50)
            .unwrap_or(false);
        if dc.enabled && has_valid_token {
            if let Some(ref token) = dc_token {
                // Extract OpenAI API key for TTS (from providers.tts.openai in keys.toml)
                let openai_key = config
                    .providers
                    .tts
                    .as_ref()
                    .and_then(|t| t.openai.as_ref())
                    .and_then(|p| p.api_key.clone());
                let dc_agent = crate::channels::discord::DiscordAgent::new(
                    channel_factory.create_agent_service(),
                    service_context.clone(),
                    dc.allowed_users.clone(),
                    config.voice.clone(),
                    openai_key,
                    app.shared_session_id(),
                    discord_state.clone(),
                    dc.respond_to.clone(),
                    dc.allowed_channels.clone(),
                );
                tracing::info!(
                    "Spawning Discord bot ({} allowed users)",
                    dc.allowed_users.len()
                );
                Some(dc_agent.start(token.clone()))
            } else {
                tracing::debug!("Discord enabled but no valid token configured");
                None
            }
        } else {
            None
        }
    };

    // Spawn Slack bot if configured (needs both bot token + app token for Socket Mode)
    #[cfg(feature = "slack")]
    let _slack_handle = {
        let sl = &config.channels.slack;
        let sl_token = sl.token.clone();
        let sl_app_token = sl.app_token.clone();
        let has_valid_tokens = sl_token
            .as_ref()
            .map(|t| !t.is_empty() && t.starts_with("xoxb-"))
            .unwrap_or(false)
            && sl_app_token
                .as_ref()
                .map(|t| !t.is_empty() && t.starts_with("xapp-"))
                .unwrap_or(false);
        if sl.enabled && has_valid_tokens {
            if let (Some(bot_tok), Some(app_tok)) = (sl_token, sl_app_token) {
                let sl_agent = crate::channels::slack::SlackAgent::new(
                    channel_factory.create_agent_service(),
                    service_context.clone(),
                    sl.allowed_ids.clone(),
                    app.shared_session_id(),
                    slack_state.clone(),
                    sl.respond_to.clone(),
                    sl.allowed_channels.clone(),
                );
                tracing::info!("Spawning Slack bot ({} allowed IDs)", sl.allowed_ids.len());
                Some(sl_agent.start(bot_tok, app_tok))
            } else {
                tracing::debug!("Slack enabled but missing valid tokens");
                None
            }
        } else {
            None
        }
    };

    // Run TUI
    tracing::debug!("Launching TUI");
    tui::run(app).await.context("TUI error")?;

    // Print shutdown logo and rolling message
    {
        const BYES: &[&str] = &[
            "🦀 Back to the ocean...",
            "🦀 *scuttles into the sunset*",
            "🦀 Until next tide!",
            "🦀 Gone crabbing. BRB never.",
            "🦀 The crabs retreat... for now.",
            "🦀 Shell ya later!",
            "🦀 Logging off. Don't forget to hydrate.",
            "🦀 Peace out, landlubber.",
            "🦀 Crab rave: paused.",
            "🦀 See you on the other tide.",
        ];
        let i = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos() as usize)
            % BYES.len();

        // Print logo
        let logo_style = "\x1b[38;2;215;100;20m"; // Muted orange
        let reset = "\x1b[0m";
        let logo = r"   ___                    ___           _
  / _ \ _ __  ___ _ _    / __|_ _ __ _| |__  ___
 | (_) | '_ \/ -_) ' \  | (__| '_/ _` | '_ \(_-<
  \___/| .__/\___|_||_|  \___|_| \__,_|_.__//__/
       |_|";
        println!();
        println!("{}{}{}", logo_style, logo, reset);
        println!();
        println!("{}{}{}", logo_style, BYES[i], reset);
        println!();
    }

    Ok(())
}
