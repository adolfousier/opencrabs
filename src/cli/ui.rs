//! TUI chat startup — provider init, tool registry, approval callbacks, Telegram spawn.

use anyhow::{Context, Result};
use std::sync::Arc;

use crate::brain::prompt_builder::RuntimeInfo;
use crate::brain::{BrainLoader, CommandLoader};

/// Start interactive chat session
pub(crate) async fn cmd_daemon(config: &crate::config::Config) -> Result<()> {
    cmd_chat_inner(config, None, false, true).await
}

pub(crate) async fn cmd_chat(
    config: &crate::config::Config,
    session_id: Option<String>,
    force_onboard: bool,
) -> Result<()> {
    cmd_chat_inner(config, session_id, force_onboard, false).await
}

async fn cmd_chat_inner(
    config: &crate::config::Config,
    session_id: Option<String>,
    force_onboard: bool,
    headless: bool,
) -> Result<()> {
    use crate::{
        brain::{
            agent::AgentService,
            tools::{
                analyze_image::AnalyzeImageTool, bash::BashTool, brave_search::BraveSearchTool,
                code_exec::CodeExecTool, config_tool::ConfigTool, context::ContextTool,
                doc_parser::DocParserTool, edit::EditTool, exa_search::ExaSearchTool,
                generate_image::GenerateImageTool, glob::GlobTool, grep::GrepTool,
                http::HttpClientTool, load_brain_file::LoadBrainFileTool, ls::LsTool,
                memory_search::MemorySearchTool, notebook::NotebookEditTool, plan_tool::PlanTool,
                provider_vision::ProviderVisionTool, read::ReadTool, registry::ToolRegistry,
                session_search::SessionSearchTool, slash_command::SlashCommandTool, task::TaskTool,
                web_search::WebSearchTool, write::WriteTool,
                write_opencrabs_file::WriteOpenCrabsFileTool,
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
    let provider = match crate::brain::provider::create_provider(config) {
        Ok(p) => {
            tracing::info!(
                "Provider ready: {} (model: {})",
                p.name(),
                p.default_model()
            );
            p
        }
        Err(e) => {
            tracing::error!("Failed to create provider: {}", e);
            eprintln!("Error: failed to create provider: {}", e);
            return Err(e);
        }
    };

    // Create tool registry (Arc-wrapped early so SpawnAgentTool can reference it)
    tracing::debug!("Setting up tool registry");
    let tool_registry = Arc::new(ToolRegistry::new());
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
    // On-demand brain file loader — agent fetches USER.md, MEMORY.md etc. only when needed
    tool_registry.register(Arc::new(LoadBrainFileTool));
    // OpenCrabs file writer — agent can edit/append/overwrite any file in ~/.opencrabs/
    tool_registry.register(Arc::new(WriteOpenCrabsFileTool));
    // Session search — hybrid QMD search across all session message history
    tool_registry.register(Arc::new(SessionSearchTool::new(db.pool().clone())));
    // Channel search — search passively captured channel messages (Telegram groups, etc.)
    use crate::brain::tools::channel_search::ChannelSearchTool;
    tool_registry.register(Arc::new(ChannelSearchTool::new(
        crate::db::ChannelMessageRepository::new(db.pool().clone()),
    )));
    // Cron job management — agent can create/list/delete/enable/disable scheduled jobs
    use crate::brain::tools::cron_manage::CronManageTool;
    tool_registry.register(Arc::new(CronManageTool::new(
        crate::db::CronJobRepository::new(db.pool().clone()),
    )));
    // A2A send — agent can communicate with remote A2A agents
    use crate::brain::tools::a2a_send::A2aSendTool;
    tool_registry.register(Arc::new(A2aSendTool::new()));
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

    // Image generation tool (requires image.generation.enabled + api_key in config)
    if config.image.generation.enabled
        && let Some(ref key) = config.image.generation.api_key
    {
        tool_registry.register(Arc::new(GenerateImageTool::new(
            key.clone(),
            config.image.generation.model.clone(),
        )));
        tracing::info!("Registered generate_image tool");
    }
    // Image vision tool — prefer Gemini, fall back to provider's vision_model
    if config.image.vision.enabled
        && let Some(ref key) = config.image.vision.api_key
    {
        tool_registry.register(Arc::new(AnalyzeImageTool::new(
            key.clone(),
            config.image.vision.model.clone(),
        )));
        tracing::info!("Registered analyze_image tool (Gemini)");
    } else if let Some((api_key, base_url, vision_model)) =
        crate::brain::provider::factory::active_provider_vision(config)
    {
        tool_registry.register(Arc::new(ProviderVisionTool::new(
            api_key,
            base_url,
            vision_model,
        )));
        tracing::info!("Registered analyze_image tool (provider vision model)");
    }

    // Phase 5: Multi-agent orchestration
    let subagent_manager = Arc::new(crate::brain::tools::subagent::SubAgentManager::new());
    tool_registry.register(Arc::new(
        crate::brain::tools::subagent::SpawnAgentTool::new(
            subagent_manager.clone(),
            tool_registry.clone(),
        ),
    ));
    tool_registry.register(Arc::new(crate::brain::tools::subagent::WaitAgentTool::new(
        subagent_manager.clone(),
    )));
    tool_registry.register(Arc::new(crate::brain::tools::subagent::SendInputTool::new(
        subagent_manager.clone(),
    )));
    tool_registry.register(Arc::new(
        crate::brain::tools::subagent::CloseAgentTool::new(subagent_manager.clone()),
    ));
    tool_registry.register(Arc::new(
        crate::brain::tools::subagent::ResumeAgentTool::new(
            subagent_manager.clone(),
            tool_registry.clone(),
        ),
    ));

    // Phase 6: Team orchestration
    let team_manager = Arc::new(crate::brain::tools::subagent::TeamManager::new());
    tool_registry.register(Arc::new(
        crate::brain::tools::subagent::TeamCreateTool::new(
            subagent_manager.clone(),
            team_manager.clone(),
            tool_registry.clone(),
        ),
    ));
    tool_registry.register(Arc::new(
        crate::brain::tools::subagent::TeamDeleteTool::new(
            subagent_manager.clone(),
            team_manager.clone(),
        ),
    ));
    tool_registry.register(Arc::new(
        crate::brain::tools::subagent::TeamBroadcastTool::new(
            subagent_manager.clone(),
            team_manager.clone(),
        ),
    ));
    tracing::info!("Registered 8 sub-agent + team orchestration tools");

    // Index existing memory files and warm up embedding engine in the background.
    // Delay startup to avoid concurrent FFI access with resumed agent tasks
    // and channel connections — llama-cpp GGML can segfault under contention.
    tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
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

    // Preload local whisper model before the TUI starts so candle's
    // "Running on CPU..." println! fires on the raw terminal, not inside
    // the alternate screen where it would bleed into the TUI layout.
    #[cfg(feature = "local-stt")]
    {
        let vc = config.voice_config();
        if vc.stt_mode == crate::config::SttMode::Local
            && crate::channels::voice::local_stt_available()
        {
            let model_id = vc.local_stt_model.clone();
            tracing::info!("Preloading local STT model '{}'", model_id);
            match crate::channels::voice::preload_local_whisper(&model_id).await {
                Ok(()) => tracing::info!("Local STT model preloaded"),
                Err(e) => tracing::warn!("Local STT preload failed (will retry on use): {}", e),
            }
        }
    }

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

    let system_brain = brain_loader.build_core_brain(Some(&runtime_info), Some(&commands_section));

    // Create agent service with dynamic system brain
    let agent_service = Arc::new(
        AgentService::new(provider.clone(), service_context.clone(), config)
            .with_system_brain(system_brain.clone())
            .with_working_directory(working_directory.clone()),
    );

    // Shared WhatsApp state — single bot instance, shared between agent + onboarding
    #[cfg(feature = "whatsapp")]
    let whatsapp_state = Arc::new(crate::channels::whatsapp::WhatsAppState::new());

    // Create TUI app first (so we can get the event sender)
    tracing::debug!("Creating TUI app");
    let mut app = tui::App::new(
        agent_service,
        service_context.clone(),
        #[cfg(feature = "whatsapp")]
        whatsapp_state.clone(),
    );

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

            Ok((response.approved, false))
        })
    });

    // Create progress callback that sends tool events to TUI
    let progress_sender = app.event_sender();

    // Last confirmed context size from the API (set by TokenCount event).
    let last_ctx_tokens = Arc::new(std::sync::atomic::AtomicU32::new(0));

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
                    // Count output tokens in this chunk via tiktoken for per-response display.
                    // Send per-chunk count — the TUI accumulates and controls reset timing.
                    let chunk_tokens = crate::brain::tokenizer::count_tokens(&text) as u32;
                    let _ = progress_sender.send(TuiEvent::StreamingOutputTokens {
                        session_id,
                        tokens: chunk_tokens,
                    });
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
                ProgressEvent::BuildLine(line) => progress_sender.send(TuiEvent::BuildLine(line)),
                ProgressEvent::RestartReady { status } => {
                    progress_sender.send(TuiEvent::RestartReady(status))
                }
                ProgressEvent::TokenCount(count) => {
                    // Real count from the API — update baseline.
                    last_ctx_tokens.store(count as u32, std::sync::atomic::Ordering::Relaxed);
                    progress_sender.send(TuiEvent::TokenCountUpdated { session_id, count })
                }
                ProgressEvent::ReasoningChunk { text } => {
                    progress_sender.send(TuiEvent::ReasoningChunk { session_id, text })
                }
                ProgressEvent::QueuedUserMessage { text } => {
                    progress_sender.send(TuiEvent::QueuedUserMessage { session_id, text })
                }
                ProgressEvent::SelfHealingAlert { message } => {
                    progress_sender.send(TuiEvent::SystemMessage(format!("🔧 {}", message)))
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

    // Register evolve tool (binary self-update from GitHub releases)
    tool_registry.register(Arc::new(crate::brain::tools::evolve::EvolveTool::new(
        Some(progress_callback.clone()),
    )));

    // Create config watch channel — single source of truth for all hot-reloadable config.
    // All channel agents receive a Receiver and read the latest config per-message.
    let (config_tx, config_rx) = tokio::sync::watch::channel(config.clone());

    // Create ChannelFactory (shared by static channel spawn + WhatsApp connect tool).
    // Tool registry is set lazily after Arc wrapping to break circular dependency.
    let channel_factory = Arc::new(crate::channels::ChannelFactory::new(
        provider.clone(),
        service_context.clone(),
        system_brain.clone(),
        working_directory.clone(),
        brain_path.clone(),
        app.shared_session_id(),
        config_rx,
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

    // Register WhatsApp connect tool (agent-callable QR pairing)
    #[cfg(feature = "whatsapp")]
    tool_registry.register(Arc::new(
        crate::brain::tools::whatsapp_connect::WhatsAppConnectTool::new(
            Some(progress_callback.clone()),
            whatsapp_state.clone(),
        ),
    ));

    // Register WhatsApp send tool (proactive messaging)
    #[cfg(feature = "whatsapp")]
    tool_registry.register(Arc::new(
        crate::brain::tools::whatsapp_send::WhatsAppSendTool::new(
            whatsapp_state.clone(),
            channel_factory.config_rx(),
        ),
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

    // Shared Trello state for proactive card operations
    #[cfg(feature = "trello")]
    let trello_state = Arc::new(crate::channels::trello::TrelloState::new());

    // Register Trello connect tool (agent-callable board setup)
    #[cfg(feature = "trello")]
    tool_registry.register(Arc::new(
        crate::brain::tools::trello_connect::TrelloConnectTool::new(
            channel_factory.clone(),
            trello_state.clone(),
        ),
    ));

    // Register Trello send tool (proactive card operations)
    #[cfg(feature = "trello")]
    tool_registry.register(Arc::new(
        crate::brain::tools::trello_send::TrelloSendTool::new(trello_state.clone()),
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

    // Create session-updated notification channel — remote channels fire this so the TUI
    // reloads in real-time when Telegram/WhatsApp/Discord/Slack messages are processed.
    let (session_updated_tx, mut session_updated_rx) =
        tokio::sync::mpsc::unbounded_channel::<uuid::Uuid>();
    {
        let event_sender = app.event_sender();
        tokio::spawn(async move {
            while let Some(session_id) = session_updated_rx.recv().await {
                let _ = event_sender.send(crate::tui::events::TuiEvent::SessionUpdated(session_id));
            }
        });
    }

    // Create agent service with approval callback, progress callback, and message queue
    tracing::debug!("Creating agent service with approval, progress, and message queue callbacks");
    let shared_tool_registry = tool_registry;

    // Load dynamic tools from ~/.opencrabs/tools.toml
    let tools_toml_path = crate::brain::tools::dynamic::DynamicToolLoader::default_path()
        .unwrap_or_else(|| std::path::PathBuf::from("tools.toml"));
    let dynamic_count = crate::brain::tools::dynamic::DynamicToolLoader::load(
        &tools_toml_path,
        &shared_tool_registry,
    );
    if dynamic_count > 0 {
        tracing::info!("Loaded {dynamic_count} dynamic tool(s) from tools.toml");
    }

    // Register tool_manage — agent can add/remove/reload dynamic tools at runtime
    shared_tool_registry.register(Arc::new(
        crate::brain::tools::tool_manage::ToolManageTool::new(
            shared_tool_registry.clone(),
            tools_toml_path,
        ),
    ));

    // Browser automation tools (headless Chrome via CDP)
    #[cfg(feature = "browser")]
    {
        let browser_manager = Arc::new(crate::brain::tools::browser::BrowserManager::new());
        shared_tool_registry.register(Arc::new(
            crate::brain::tools::browser::BrowserNavigateTool::new(browser_manager.clone()),
        ));
        shared_tool_registry.register(Arc::new(
            crate::brain::tools::browser::BrowserScreenshotTool::new(browser_manager.clone()),
        ));
        shared_tool_registry.register(Arc::new(
            crate::brain::tools::browser::BrowserClickTool::new(browser_manager.clone()),
        ));
        shared_tool_registry.register(Arc::new(
            crate::brain::tools::browser::BrowserTypeTool::new(browser_manager.clone()),
        ));
        shared_tool_registry.register(Arc::new(
            crate::brain::tools::browser::BrowserEvalTool::new(browser_manager.clone()),
        ));
        shared_tool_registry.register(Arc::new(
            crate::brain::tools::browser::BrowserContentTool::new(browser_manager.clone()),
        ));
        shared_tool_registry.register(Arc::new(
            crate::brain::tools::browser::BrowserWaitTool::new(browser_manager),
        ));
        tracing::info!("Browser automation tools registered (7 tools)");
    }

    // Now that the registry is Arc'd, give it to the channel factory
    channel_factory.set_tool_registry(shared_tool_registry.clone());

    // Share session_updated_tx with the factory so channel agents (WhatsApp, Telegram, etc.)
    // trigger real-time TUI refresh when they complete a response.
    channel_factory.set_session_updated_tx(session_updated_tx.clone());

    let agent_service = Arc::new(
        AgentService::new(provider.clone(), service_context.clone(), config)
            .with_system_brain(system_brain)
            .with_tool_registry(shared_tool_registry.clone())
            .with_approval_callback(Some(approval_callback))
            .with_progress_callback(Some(progress_callback))
            .with_message_queue_callback(Some(message_queue_callback))
            .with_sudo_callback(Some(sudo_callback))
            .with_working_directory(working_directory.clone())
            .with_brain_path(brain_path)
            .with_session_updated_tx(session_updated_tx),
    );

    // Update app with the configured agent service (preserve event channels!)
    app.set_agent_service(agent_service);

    // Resume any in-flight requests that were interrupted by a restart/rebuild/evolve.
    // Rows only exist if the process died mid-request (normal completions delete them).
    // Instead of replaying the original message, we send a continuation prompt so the
    // agent reads context and picks up naturally — no loops, no leaking restart signals.
    // Routes responses back to the originating channel (TUI, Telegram, Discord, etc.).
    let resume_event_sender = app.event_sender();
    {
        let pending_repo = crate::db::PendingRequestRepository::new(db.pool().clone());
        match pending_repo.get_interrupted().await {
            Ok(requests) if !requests.is_empty() => {
                tracing::info!(
                    "Found {} interrupted request(s) — resuming on startup",
                    requests.len()
                );
                // Clear the table so these don't resume again if THIS run also crashes
                let _ = pending_repo.clear_all().await;
                let agent = app.agent_service().clone();
                // Dedup by session_id — only resume each session once
                let mut seen = std::collections::HashSet::new();
                for req in requests {
                    if let Ok(session_id) = uuid::Uuid::parse_str(&req.session_id) {
                        if !seen.insert(session_id) {
                            continue;
                        }
                        let agent = agent.clone();
                        let ev_tx = resume_event_sender.clone();
                        let channel = req.channel.clone();
                        let channel_chat_id = req.channel_chat_id.clone();
                        tracing::info!(
                            "Resuming session {} (channel: {}, chat_id: {:?})",
                            &req.session_id[..8.min(req.session_id.len())],
                            channel,
                            channel_chat_id,
                        );

                        // TUI: wire cancel token and send response via TuiEvent
                        // Non-TUI: send response back to the originating channel
                        let tg = telegram_state.clone();
                        let dc = discord_state.clone();
                        let wa = whatsapp_state.clone();
                        let sk = slack_state.clone();
                        let token = tokio_util::sync::CancellationToken::new();
                        if channel == "tui" {
                            let _ = resume_event_sender.send(
                                crate::tui::events::TuiEvent::PendingResumed {
                                    session_id,
                                    cancel_token: token.clone(),
                                },
                            );
                        }
                        tokio::spawn(async move {
                            let prompt = "[System: A restart just occurred while you were \
                                processing a request. Read the conversation context and continue \
                                where you left off naturally. Do not mention the restart or \
                                any interruption — just pick up seamlessly.]"
                                .to_string();
                            match agent
                                .send_message_with_tools_and_mode(
                                    session_id,
                                    prompt,
                                    None,
                                    Some(token),
                                )
                                .await
                            {
                                Ok(response) => {
                                    tracing::info!(
                                        "Resume completed for session {} ({}): {} chars",
                                        session_id,
                                        channel,
                                        response.content.len()
                                    );
                                    match channel.as_str() {
                                        "tui" => {
                                            let _ = ev_tx.send(
                                                crate::tui::events::TuiEvent::ResponseComplete {
                                                    session_id,
                                                    response,
                                                },
                                            );
                                        }
                                        "telegram" => {
                                            if let Some(ref cid) = channel_chat_id
                                                && let Ok(chat_id) = cid.parse::<i64>()
                                                && let Some(bot) = tg.bot().await
                                            {
                                                use teloxide::prelude::*;
                                                let clean =
                                                    crate::utils::sanitize::strip_llm_artifacts(
                                                        &response.content,
                                                    );
                                                let html =
                                                    crate::channels::telegram::handler::md_to_html(
                                                        &clean,
                                                    );
                                                let _ = bot
                                                    .send_message(
                                                        teloxide::types::ChatId(chat_id),
                                                        html,
                                                    )
                                                    .parse_mode(teloxide::types::ParseMode::Html)
                                                    .await;
                                            }
                                        }
                                        "discord" => {
                                            if let Some(ref cid) = channel_chat_id
                                                && let Ok(ch_id) = cid.parse::<u64>()
                                                && let Some(http) = dc.http().await
                                            {
                                                let channel =
                                                    serenity::model::id::ChannelId::new(ch_id);
                                                let _ = channel.say(&http, &response.content).await;
                                            }
                                        }
                                        #[cfg(feature = "whatsapp")]
                                        "whatsapp" => {
                                            if let Some(ref cid) = channel_chat_id
                                                && let Some(client) = wa.client().await
                                                && let Ok(jid) =
                                                    cid.parse::<wacore_binary::jid::Jid>()
                                            {
                                                let msg = waproto::whatsapp::Message {
                                                    conversation: Some(response.content.clone()),
                                                    ..Default::default()
                                                };
                                                let _ = client.send_message(jid, msg).await;
                                            }
                                        }
                                        "slack" => {
                                            if let Some(ref cid) = channel_chat_id
                                                && let (Some(token_val), Some(client)) =
                                                    (sk.bot_token().await, sk.client().await)
                                            {
                                                let api_token = slack_morphism::prelude::SlackApiToken::new(
                                                    slack_morphism::prelude::SlackApiTokenValue::from(token_val),
                                                );
                                                let session = client.open_session(&api_token);
                                                let req = slack_morphism::prelude::SlackApiChatPostMessageRequest::new(
                                                    cid.clone().into(),
                                                    slack_morphism::prelude::SlackMessageContent::new()
                                                        .with_text(response.content.clone()),
                                                );
                                                let _ = session.chat_post_message(&req).await;
                                            }
                                        }
                                        other => {
                                            tracing::warn!(
                                                "No recovery routing for channel '{}' — response saved to DB only",
                                                other
                                            );
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::error!(
                                        "Resume failed for session {}: {}",
                                        session_id,
                                        e
                                    );
                                    if channel == "tui" {
                                        let _ = ev_tx.send(crate::tui::events::TuiEvent::Error {
                                            session_id,
                                            message: e.to_string(),
                                        });
                                    }
                                }
                            }
                        });
                    }
                }
            }
            Ok(_) => {}
            Err(e) => tracing::warn!("Failed to check for interrupted requests: {}", e),
        }
    }

    // Channel manager — handles dynamic spawn/stop of channel agents on config reload
    let channel_manager = Arc::new(crate::channels::ChannelManager::new(
        channel_factory.clone(),
        db.pool().clone(),
        #[cfg(feature = "telegram")]
        telegram_state.clone(),
        #[cfg(feature = "whatsapp")]
        whatsapp_state.clone(),
        #[cfg(feature = "discord")]
        discord_state.clone(),
        #[cfg(feature = "slack")]
        slack_state.clone(),
        #[cfg(feature = "trello")]
        trello_state.clone(),
    ));

    // Initial channel spawn — reconcile against current config
    channel_manager.reconcile(config);

    // Spawn config hot-reload watcher — fires on any change to config.toml, keys.toml,
    // or commands.toml without requiring a restart.
    {
        use crate::tui::events::TuiEvent;
        use crate::utils::config_watcher::{self, ReloadCallback};

        let mut callbacks: Vec<ReloadCallback> = Vec::new();

        // Unified config broadcast — push new config to watch channel so ALL
        // channel agents see the latest values on next message (allowlists,
        // voice, respond_to, allowed_channels, idle_timeout, TTS keys, etc.)
        {
            let agent = app.agent_service().clone();
            let sender = app.event_sender();
            callbacks.push(Arc::new(move |cfg: crate::config::Config| {
                // Broadcast full config to all channels via watch channel
                let _ = config_tx.send(cfg.clone());

                // Provider swap still needs explicit call
                let agent = agent.clone();
                tokio::spawn(async move {
                    match crate::brain::provider::create_provider(&cfg) {
                        Ok(new_provider) => {
                            agent.swap_provider(new_provider);
                            tracing::info!("ConfigWatcher: LLM provider reloaded from new keys");
                        }
                        Err(e) => {
                            tracing::warn!(
                                "ConfigWatcher: provider rebuild failed, keeping current: {}",
                                e
                            );
                        }
                    }
                });

                // TUI refresh — commands autocomplete + approval policy
                let _ = sender.send(TuiEvent::ConfigReloaded);
            }));
        }

        // Channel lifecycle — spawn/stop channels when enabled flag changes
        {
            let channel_mgr = channel_manager.clone();
            callbacks.push(Arc::new(move |cfg: crate::config::Config| {
                channel_mgr.reconcile(&cfg);
            }));
        }

        let _config_watcher = config_watcher::spawn(callbacks);
    }

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

    // Spawn cron scheduler — polls every 60s, executes jobs in the user's active session
    {
        let cron_repo = crate::db::CronJobRepository::new(db.pool().clone());
        let cron_run_repo = crate::db::CronJobRunRepository::new(db.pool().clone());
        let cron_scheduler = crate::cron::CronScheduler::new(
            cron_repo,
            cron_run_repo,
            channel_factory.clone(),
            service_context.clone(),
            app.shared_session_id(),
        );
        let _cron_handle = cron_scheduler.spawn();
        tracing::info!("Cron scheduler spawned");
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

    // Channel spawning is handled by channel_manager.reconcile() above (line ~669).
    // On config reload, reconcile() is called again to spawn/stop channels dynamically.

    // Run TUI or block in headless daemon mode
    if headless {
        // Spawn health endpoint if configured (for systemd watchdog / uptime monitors)
        if let Some(port) = config.daemon.health_port {
            tokio::spawn(async move {
                if let Err(e) = crate::cli::daemon_health::serve(port).await {
                    tracing::error!("Daemon health server failed: {}", e);
                }
            });
        }

        tracing::info!("OpenCrabs daemon started — press Ctrl+C to stop");
        println!("🦀 OpenCrabs daemon running. Press Ctrl+C to stop.");
        tokio::signal::ctrl_c()
            .await
            .context("Failed to listen for ctrl_c")?;
        tracing::info!("OpenCrabs daemon shutting down");
        crate::config::profile::release_all_locks();
        return Ok(());
    }
    tracing::debug!("Launching TUI");
    let tui_result = tui::run(app).await;

    // Release all token locks on exit (normal or crash)
    crate::config::profile::release_all_locks();

    if let Err(ref e) = tui_result {
        // TUI crashed or failed to start — offer crash recovery dialog.
        // This runs on the raw terminal (not alternate screen) so the user
        // can see the error and pick an older version to roll back to.
        tracing::error!("TUI crashed: {}", e);

        // Make sure raw mode is off and alternate screen is exited before showing dialog
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen);

        let error_msg = format!("{}", e);
        match super::crash_recovery::show_crash_recovery(&error_msg).await {
            Ok(super::crash_recovery::CrashRecoveryAction::Retry) => {
                // User wants to retry — return the original error so the process
                // exits, and they can relaunch manually. A full retry would need
                // re-initializing everything which is not practical here.
                println!("\n  Relaunch OpenCrabs to try again.\n");
            }
            Ok(super::crash_recovery::CrashRecoveryAction::Installed(v)) => {
                println!("\n  Installed v{}. Relaunch to use it.\n", v);
                return Ok(());
            }
            Ok(super::crash_recovery::CrashRecoveryAction::Quit) | Err(_) => {}
        }
        return tui_result.context("TUI error");
    }

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
