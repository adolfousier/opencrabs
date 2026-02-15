//! TUI chat startup — provider init, tool registry, approval callbacks, Telegram spawn.

use anyhow::{Context, Result};
use std::sync::Arc;

use crate::brain::prompt_builder::RuntimeInfo;
use crate::brain::{BrainLoader, CommandLoader};

/// Start interactive chat session
pub(crate) async fn cmd_chat(
    config: &crate::config::Config,
    _session_id: Option<String>,
    force_onboard: bool,
) -> Result<()> {
    use crate::{
        db::Database,
        llm::{
            agent::AgentService,
            tools::{
                bash::BashTool, brave_search::BraveSearchTool, code_exec::CodeExecTool,
                context::ContextTool, doc_parser::DocParserTool, edit::EditTool,
                exa_search::ExaSearchTool, glob::GlobTool, grep::GrepTool,
                http::HttpClientTool, ls::LsTool, notebook::NotebookEditTool, plan_tool::PlanTool,
                read::ReadTool, registry::ToolRegistry, task::TaskTool, web_search::WebSearchTool,
                write::WriteTool,
            },
        },
        services::ServiceContext,
        tui,
    };

    println!("Starting OpenCrabs AI Orchestration Agent...");

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
    let provider = crate::llm::provider::create_provider(config)?;

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
    // EXA search: always available (free via MCP), uses direct API if key is set
    let exa_key = std::env::var("EXA_API_KEY").ok();
    let exa_mode = if exa_key.is_some() { "direct API" } else { "MCP (free)" };
    tool_registry.register(Arc::new(ExaSearchTool::new(exa_key)));
    tracing::info!("Registered EXA search tool (mode: {})", exa_mode);
    // Brave search: requires API key
    if let Ok(brave_key) = std::env::var("BRAVE_API_KEY") {
        tool_registry.register(Arc::new(BraveSearchTool::new(brave_key)));
        tracing::info!("Registered Brave search tool");
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
    let commands_section =
        CommandLoader::commands_section(&builtin_commands, &user_commands);

    let system_brain = brain_loader.build_system_brain(
        Some(&runtime_info),
        Some(&commands_section),
    );

    // Create agent service with dynamic system brain
    let agent_service = Arc::new(
        AgentService::new(provider.clone(), service_context.clone())
            .with_system_brain(system_brain.clone())
            .with_max_tool_iterations(20)
            .with_working_directory(working_directory.clone()),
    );

    // Create TUI app first (so we can get the event sender)
    tracing::debug!("Creating TUI app");
    let mut app = tui::App::new(agent_service, service_context.clone());

    // Get event sender from app
    let event_sender = app.event_sender();

    // Create approval callback that sends requests to TUI
    let approval_callback: crate::llm::agent::ApprovalCallback = Arc::new(move |tool_info| {
        let sender = event_sender.clone();
        Box::pin(async move {
            use crate::tui::events::{ToolApprovalRequest, TuiEvent};
            use tokio::sync::mpsc;

            // Create response channel
            let (response_tx, mut response_rx) = mpsc::unbounded_channel();

            // Create approval request
            let request = ToolApprovalRequest {
                request_id: uuid::Uuid::new_v4(),
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
                    crate::llm::agent::AgentError::Internal(format!(
                        "Failed to send approval request: {}",
                        e
                    ))
                })?;

            // Wait for response
            let response = response_rx.recv().await.ok_or_else(|| {
                crate::llm::agent::AgentError::Internal(
                    "Approval response channel closed".to_string(),
                )
            })?;

            Ok(response.approved)
        })
    });

    // Create progress callback that sends tool events to TUI
    let progress_sender = app.event_sender();
    let progress_callback: crate::llm::agent::ProgressCallback = Arc::new(move |event| {
        use crate::llm::agent::ProgressEvent;
        use crate::tui::events::TuiEvent;
        match event {
            ProgressEvent::ToolStarted { tool_name, tool_input } => {
                let _ = progress_sender.send(TuiEvent::ToolCallStarted { tool_name, tool_input });
            }
            ProgressEvent::ToolCompleted { tool_name, tool_input, success, summary } => {
                let _ = progress_sender.send(TuiEvent::ToolCallCompleted { tool_name, tool_input, success, summary });
            }
            ProgressEvent::IntermediateText { text } => {
                let _ = progress_sender.send(TuiEvent::IntermediateText(text));
            }
            ProgressEvent::Thinking => {} // spinner handles this already
            ProgressEvent::Compacting => {
                let _ = progress_sender.send(TuiEvent::AgentProcessing);
            }
        }
    });

    // Create message queue callback that checks for queued user messages
    let message_queue = app.message_queue.clone();
    let message_queue_callback: crate::llm::agent::MessageQueueCallback = Arc::new(move || {
        let queue = message_queue.clone();
        Box::pin(async move { queue.lock().await.take() })
    });

    // Create agent service with approval callback, progress callback, and message queue
    tracing::debug!("Creating agent service with approval, progress, and message queue callbacks");
    let shared_tool_registry = Arc::new(tool_registry);
    let shared_brain = system_brain.clone();
    let shared_brain_path = brain_path.clone();
    let agent_service = Arc::new(
        AgentService::new(provider.clone(), service_context.clone())
            .with_system_brain(system_brain)
            .with_tool_registry(shared_tool_registry.clone())
            .with_approval_callback(Some(approval_callback))
            .with_progress_callback(Some(progress_callback))
            .with_message_queue_callback(Some(message_queue_callback))
            .with_max_tool_iterations(20)
            .with_working_directory(working_directory.clone())
            .with_brain_path(brain_path),
    );

    // Update app with the configured agent service (preserve event channels!)
    app.set_agent_service(agent_service);

    // Set force onboard flag if requested
    if force_onboard {
        app.force_onboard = true;
    }

    // Spawn Telegram bot if configured
    #[cfg(feature = "telegram")]
    let _telegram_handle = {
        let tg = &config.channels.telegram;
        let tg_token = tg.token.clone().or_else(|| std::env::var("TELEGRAM_BOT_TOKEN").ok());
        if tg.enabled || tg_token.is_some() {
            if let Some(ref token) = tg_token {
                // Build a separate agent service for Telegram — same brain, tools,
                // and working directory as the TUI agent (minus TUI callbacks).
                let tg_agent = Arc::new(
                    AgentService::new(provider.clone(), service_context.clone())
                        .with_system_brain(shared_brain.clone())
                        .with_tool_registry(shared_tool_registry.clone())
                        .with_auto_approve_tools(true)
                        .with_max_tool_iterations(20)
                        .with_working_directory(working_directory.clone())
                        .with_brain_path(shared_brain_path.clone()),
                );
                // Extract OpenAI API key for TTS (from env or provider config)
                // TTS uses gpt-4o-mini-tts, NOT a text generation model
                let openai_key = std::env::var("OPENAI_API_KEY").ok()
                    .or_else(|| config.providers.openai.as_ref().and_then(|p| p.api_key.clone()));
                let bot = crate::telegram::TelegramAgent::new(
                    tg_agent,
                    service_context.clone(),
                    tg.allowed_users.clone(),
                    config.voice.clone(),
                    openai_key,
                    app.shared_session_id(),
                );
                tracing::info!("Spawning Telegram bot ({} allowed users)", tg.allowed_users.len());
                Some(bot.start(token.clone()))
            } else {
                tracing::warn!("Telegram enabled but no token configured");
                None
            }
        } else {
            None
        }
    };

    // Run TUI
    tracing::debug!("Launching TUI");
    tui::run(app).await.context("TUI error")?;

    println!("\n👋 Goodbye!");

    Ok(())
}
