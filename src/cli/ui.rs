//! TUI chat startup — provider init, tool registry, approval callbacks, Telegram spawn.

use anyhow::{Context, Result};
use std::sync::Arc;

use crate::brain::BrainLoader;
use crate::brain::prompt_builder::RuntimeInfo;

/// Register (or unregister) the tools whose availability depends on config /
/// keys: EXA, Brave, image generation, and vision/video. Idempotent — calling
/// it with the current config produces the correct set, adding a tool when its
/// key / enable flag appears and removing it when it disappears.
///
/// Shared by the startup registry build and the config-watcher reload callback,
/// so a key added to `keys.toml` (or an `enabled` flag flipped) is picked up at
/// runtime in BOTH the TUI and the daemon with no restart. The registry is an
/// `Arc<ToolRegistry>` shared with the channel agents, which list tools from it
/// per request, so the change reaches channels on their next message.
pub(crate) fn register_config_dependent_tools(
    registry: &Arc<crate::brain::tools::registry::ToolRegistry>,
    config: &crate::config::Config,
) {
    use crate::brain::tools::{
        analyze_video::AnalyzeVideoTool, brave_search::BraveSearchTool, exa_search::ExaSearchTool,
        generate_image::GenerateImageTool, provider_vision::ProviderVisionTool,
        web_search::WebSearchTool,
    };

    // EXA: always available (free via MCP; direct API when a key is set).
    let exa_key = config
        .providers
        .web_search
        .as_ref()
        .and_then(|ws| ws.exa.as_ref())
        .and_then(|p| p.api_key.clone())
        .filter(|k| !k.is_empty());
    let exa_tool = Arc::new(ExaSearchTool::new(exa_key));
    registry.register(exa_tool.clone());

    // Brave: requires `enabled = true` AND a non-empty key.
    let brave_tool = if let Some(brave_cfg) = config
        .providers
        .web_search
        .as_ref()
        .and_then(|ws| ws.brave.as_ref())
        && brave_cfg.enabled
        && let Some(brave_key) = brave_cfg.api_key.clone().filter(|k| !k.is_empty())
    {
        let bt = Arc::new(BraveSearchTool::new(brave_key));
        registry.register(bt.clone());
        Some(bt)
    } else {
        registry.unregister("brave_search");
        None
    };

    // Re-register web_search with engine references so it fans out to
    // DDG + Exa (+ Brave) in parallel instead of DDG-only.
    registry.register(Arc::new(WebSearchTool::new(Some(exa_tool), brave_tool)));

    // Image generation — active provider override or the global Gemini config.
    if let Some(tool) = GenerateImageTool::from_config(config) {
        registry.register(Arc::new(tool));
    } else {
        registry.unregister("generate_image");
    }

    // Vision (analyze_image): the session's current provider first, then the
    // configured chain, then Gemini (#1318).
    //
    // Registered UNCONDITIONALLY, because the candidate list is resolved per
    // request now. The old three-branch gate read a startup snapshot, so
    // adding a vision provider to config.toml did nothing until restart and
    // the "not configured" hint tool could be registered permanently over a
    // config that had since gained vision. The hint is emitted at request
    // time instead, when resolution genuinely finds nothing.
    {
        let mut tool = ProviderVisionTool::new();
        // Gemini stays the LAST resort, attached when configured so
        // analyze_image still works if every provider candidate fails.
        if config.image.vision.enabled
            && let Some(gkey) = config
                .image
                .vision
                .api_key
                .clone()
                .filter(|k| !k.is_empty())
        {
            tool = tool.with_gemini_fallback(gkey, config.image.vision.model.clone());
        }
        registry.register(Arc::new(tool));
    }

    // Video (analyze_video): registered whenever any vision backend exists —
    // an active provider `vision_model` or Gemini image.vision. Mirrors the
    // analyze_image gate so video vision is available wherever image vision is.
    if let Some(tool) = AnalyzeVideoTool::from_config(config) {
        registry.register(Arc::new(tool));
    } else {
        registry.unregister("analyze_video");
    }
}

/// Start the headless daemon.
///
/// Multi-profile: this one process covers EVERY profile's scheduled jobs. The
/// active profile is run in full by `cmd_chat_inner` below (which also spawns
/// its own cron scheduler); every OTHER profile under `~/.opencrabs/profiles/`
/// gets a lightweight cron-only scheduler. No need to run N separate daemons.
pub(crate) async fn cmd_daemon(config: &crate::config::Config) -> Result<()> {
    let active = crate::config::profile::active_profile()
        .unwrap_or("default")
        .to_string();
    match crate::config::profile::list_profiles() {
        Ok(entries) => {
            for entry in entries {
                // The active profile is already covered by cmd_chat_inner's
                // scheduler. Skipping it here avoids running its jobs twice.
                if entry.name == active {
                    continue;
                }
                tokio::spawn(spawn_cron_scheduler_for_profile(entry.name));
            }
        }
        Err(e) => {
            tracing::warn!("daemon: list_profiles failed, running active profile only: {e}");
        }
    }
    cmd_chat_inner(config, None, false, true).await
}

/// Spawn a cron-only scheduler for one profile, pinned to that profile's home.
///
/// Builds the minimal resources (this profile's DB, provider, brain, channel
/// factory) INSIDE the profile's task-local home scope and drives the scheduler
/// loop there, so the scheduler's own setup (cron session, config reads) and
/// every job it runs resolve to the right profile home. Logs and returns on any
/// setup failure so one half-initialized profile never takes the daemon down.
async fn spawn_cron_scheduler_for_profile(profile_name: String) {
    use crate::channels::ChannelFactory;
    use crate::db::{CronJobRepository, CronJobRunRepository, Database};
    use crate::services::ServiceContext;

    let name = profile_name.clone();
    let result: anyhow::Result<()> =
        crate::config::profile::with_profile_home_async(Some(&profile_name), async move {
            // Non-active profiles never ran the onboarding wizard — ensure
            // they still carry a brain before their cron jobs fire (#1382).
            crate::config::profile::ensure_brain_seeded();
            // One scheduler per profile machine-wide (#444). If another process
            // (a `-p <name>` daemon, or the TUI running this profile) already
            // owns this profile's scheduler, skip — polling the same cron_jobs
            // table from two schedulers double-fires every due job. The guard is
            // held across `run()` below (loops forever), released on exit/crash.
            let Some(_scheduler_lock) = crate::config::profile::acquire_scheduler_lock(&name)
            else {
                tracing::info!(
                    "Multi-profile daemon: scheduler for '{name}' already running elsewhere — skipping"
                );
                return Ok(());
            };
            // Each profile has its own config.toml, so it needs its own
            // migration pass — `load()` no longer does this implicitly (#912).
            crate::config::Config::migrate_config_files();
            let config = crate::config::Config::load()?;
            let db = Database::connect(&config.database.path).await?;
            db.run_migrations().await?;
            let service_context = ServiceContext::new(db.pool().clone());
            let provider = crate::brain::provider::create_provider(&config).await?;
            let home = crate::config::opencrabs_home();
            let mut system_brain = BrainLoader::new(home.clone()).build_core_brain(None);
            // Headless-only preamble (#129): a cron job's deliver_to carries
            // only the final message — inject the self-containedness law.
            system_brain.push_str(crate::cli::tool_setup::HEADLESS_PREAMBLE);
            // ChannelFactory wants a watch::Receiver<Config>, but every reader
            // on the cron path only calls config_rx.borrow() (never .changed()),
            // so we keep just the receiver and let the sender drop right here.
            // borrow() still returns this seeded config after the sender is gone.
            let config_rx = tokio::sync::watch::channel(config.clone()).1;
            let shared_session = Arc::new(tokio::sync::Mutex::new(None));
            let factory = Arc::new(ChannelFactory::new(
                provider,
                service_context.clone(),
                system_brain,
                home.clone(),
                home,
                shared_session.clone(),
                config_rx,
            ));
            // #129: this factory builds agents ONLY for cron execution —
            // headless backstop flag on every agent it creates.
            factory.set_headless(true);
            // Wire the tool registry into the daemon's factory — WITHOUT this the
            // cron agents got an empty registry and every job ran toolless
            // ("Tool not found: bash"). The interactive path sets this via
            // `set_tool_registry`; the daemon builds its own factory, so it must
            // populate and wire the registry here too.
            let tool_registry = Arc::new(crate::brain::tools::registry::ToolRegistry::new());
            // `true` = headless (#129): cron sessions are one-shot processes
            // whose only user-visible output is the deliver_to delivery — the
            // final message. session_notify/suggest_options stay unregistered
            // and the headless preamble rides on the factory's shared brain.
            let subagent_manager = crate::cli::tool_setup::register_core_agent_tools(&tool_registry, &db, &config, true);
            // Headless-safe runtime tools (dynamic tools.toml tools, tool_manage,
            // browser) so secondary-profile cron jobs match the primary profile's
            // functional tool set. Channel-send tools are intentionally NOT here
            // (no live channel state in the daemon — delivery uses `deliver_to`).
            crate::cli::tool_setup::register_runtime_tools(&tool_registry, &config);
            factory.set_tool_registry(tool_registry);
            factory.set_subagent_manager(subagent_manager);
            let scheduler = crate::cron::CronScheduler::new(
                CronJobRepository::new(db.pool().clone()),
                CronJobRunRepository::new(db.pool().clone()),
                factory,
                service_context,
            );
            tracing::info!("Multi-profile daemon: cron scheduler running for profile '{name}'");
            scheduler.run().await; // loops forever
            Ok(())
        })
        .await;
    if let Err(e) = result {
        tracing::error!("Cron scheduler for profile '{profile_name}' failed to start: {e}");
    }
}

pub(crate) async fn cmd_chat(
    config: &crate::config::Config,
    session_id: Option<String>,
    force_onboard: bool,
) -> Result<()> {
    cmd_chat_inner(config, session_id, force_onboard, false).await
}

/// Claim this profile, or refuse to boot (#1072).
///
/// Branches on `headless` because BOOT.md documents that the TUI takes
/// priority: opening it shuts down a running daemon for the profile. A blanket
/// exit would mean the TUI refuses to start whenever systemd has the daemon up,
/// which is the normal state on a daemonised box, so the interactive path
/// preempts first and only gives up if the lock survives that.
///
/// The daemon path never preempts, or two daemons would take turns killing each
/// other.
async fn acquire_profile_instance(
    headless: bool,
) -> Result<(
    Option<crate::config::profile::InstanceLock>,
    Vec<crate::config::profile::PreemptedInstance>,
)> {
    use crate::config::profile::{InstanceGuard, acquire_instance_lock, active_profile};

    let profile = active_profile().unwrap_or("default").to_string();
    let claim = {
        let p = profile.clone();
        tokio::task::spawn_blocking(move || acquire_instance_lock(&p)).await?
    };

    let held_by = match claim {
        InstanceGuard::Acquired(lock) => return Ok((Some(lock), Vec::new())),
        InstanceGuard::Unavailable => return Ok((None, Vec::new())),
        InstanceGuard::Held { pid } => pid,
    };

    if headless {
        let who = held_by
            .map(|p| format!("PID {p}"))
            .unwrap_or_else(|| "another process".to_string());
        tracing::error!(
            "Profile '{profile}' is already running ({who}); refusing to start a second instance"
        );
        eprintln!("Error: OpenCrabs is already running for profile '{profile}' ({who}).");
        eprintln!("Stop it first, or start this one with a different profile: opencrabs -p <name>");
        std::process::exit(1);
    }

    // Interactive: the running daemon loses. Preemption also releases the
    // channel token locks, which is the "I had to reconnect Telegram" bug it
    // was written for; taking the instance lock here means it now also stops
    // the duplicate reindex that used to run alongside the daemon's.
    let preempted =
        tokio::task::spawn_blocking(crate::config::profile::preempt_other_profile_instances)
            .await
            .unwrap_or_default();
    if !preempted.is_empty() {
        tracing::info!(
            "TUI priority: preempted {} background instance(s) of profile '{profile}'",
            preempted.len()
        );
    }

    // One retry. The kernel releases the flock when the preempted process dies,
    // which is not instant, so a single immediate retry would race the SIGTERM.
    for attempt in 0..20u32 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let p = profile.clone();
        match tokio::task::spawn_blocking(move || acquire_instance_lock(&p)).await? {
            InstanceGuard::Acquired(lock) => {
                tracing::info!("Instance lock acquired after preemption ({attempt} retries)");
                return Ok((Some(lock), preempted));
            }
            InstanceGuard::Unavailable => return Ok((None, preempted)),
            InstanceGuard::Held { .. } => continue,
        }
    }

    let who = held_by
        .map(|p| format!("PID {p}"))
        .unwrap_or_else(|| "another process".to_string());
    tracing::error!("Profile '{profile}' still held by {who} two seconds after preemption");
    eprintln!(
        "Error: OpenCrabs is already running for profile '{profile}' ({who}) and did not stop."
    );
    eprintln!("Stop it manually, or start this one with a different profile: opencrabs -p <name>");
    std::process::exit(1);
}

async fn cmd_chat_inner(
    config: &crate::config::Config,
    session_id: Option<String>,
    force_onboard: bool,
    headless: bool,
) -> Result<()> {
    // Single-instance guard (#1072), before the database, the provider, the
    // tool registry or the memory reindex. A second instance of the same
    // profile used to run all of that in parallel with the first: two full
    // reindexes over one memory.db, two provider factories, two sets of channel
    // connect attempts. The scheduler and token locks caught it later and only
    // partially, by which point the duplicate work was already done.
    //
    // Held for the whole function so the kernel releases it on exit, crash
    // included.
    let (_instance_lock, preempted_instances) = acquire_profile_instance(headless).await?;

    // Startup config diagnostics (#477): once per process, never on hot
    // reload. Non-fatal — each line is a hint for silent-config-drift
    // mistakes (ignored keys, defaults that only affect new sessions).
    {
        let raw = std::fs::read_to_string(crate::config::opencrabs_home().join("config.toml")).ok();
        for w in crate::config::startup_checks::startup_warnings(config, raw.as_deref()) {
            tracing::warn!("{w}");
        }
    }
    use crate::{
        brain::{
            agent::AgentService,
            // Core tool types now live in `tool_setup::register_core_agent_tools`;
            // this path only needs the registry handle.
            tools::registry::ToolRegistry,
        },
        db::Database,
        services::ServiceContext,
        tui,
    };

    // Initialize database
    tracing::info!("Connecting to database: {}", config.database.path.display());
    let db = Database::connect(&config.database.path)
        .await
        .context("Failed to connect to database")?;

    // Run migrations
    db.run_migrations()
        .await
        .context("Failed to run database migrations")?;

    // #1114: optional startup self-repair (kill-switch: [doctor] auto_fix).
    // Repairs stuck cron rows, stale pre-init plan markers, loose brain/log
    // permissions; every action lands in the log as the audit trail.
    if config.doctor.auto_fix {
        let roots = crate::cli::commands::marker_roots(db.pool()).await;
        match crate::cli::doctor_fix::run_all(db.pool(), &roots, &crate::config::opencrabs_home())
            .await
        {
            Ok(reports) => {
                for r in &reports {
                    tracing::info!(action = r.action, detail = %r.detail, "startup auto-fix");
                }
            }
            Err(e) => tracing::warn!("startup auto-fix failed: {e:#}"),
        }
    }

    // Auto-categorize uncategorized sessions using keyword heuristics
    if let Err(e) = crate::usage::categorizer::categorize_with_heuristic(db.pool()).await {
        tracing::warn!("Session auto-categorization failed: {}", e);
    }

    // Ensure usage_pricing.toml exists (first-run only, copies from example)
    crate::usage::pricing::PricingConfig::seed_from_example();

    // Select provider based on configuration using factory
    // Returns placeholder provider if none configured, so app can start and show onboarding
    let provider = match crate::brain::provider::create_provider(config).await {
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
    // Core agent tools (file ops, shell, search, workflow, memory/brain,
    // session/channel/cron/a2a/config/slash, follow-up, discovery, sub-agents,
    // RSI) live in one place so the headless cron daemon shares the exact same
    // set. Browser/channel-send/media/rebuild/evolve are added below.
    // `false` = interactive (#129): TUI + channel users see mid-task output,
    // so session_notify/suggest_options stay registered.
    let subagent_manager =
        crate::cli::tool_setup::register_core_agent_tools(&tool_registry, &db, config, false);

    // Auto-detect VPS/cloud and disable vector embeddings if needed.
    crate::config::MemoryConfig::auto_apply_vps_defaults();

    // Index existing memory files and warm up embedding engine in the background.
    // Delay startup to avoid concurrent FFI access with resumed agent tasks
    // and channel connections — llama-cpp GGML can segfault under contention.
    // When vector_enabled = false, only FTS reindex runs (no model download).
    tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        match crate::memory::get_store() {
            Ok(store) => {
                match crate::memory::reindex(store).await {
                    Ok(n) => tracing::info!("Startup memory reindex: {n} files"),
                    Err(e) => tracing::warn!("Startup memory reindex failed: {e}"),
                }
                // Started only after the startup reindex has been awaited, so
                // the first sweep tick can never race the backfill reindex
                // already runs (#1069).
                crate::memory::backfill_sweep::spawn(store);
            }
            Err(e) => tracing::warn!("Memory store init failed at startup: {e}"),
        }
        // Warm up embedding engine so first search doesn't pay model download cost.
        // Skipped entirely when vector_enabled = false.
        match tokio::task::spawn_blocking(crate::memory::get_engine).await {
            Ok(Ok(_)) => tracing::info!("Embedding engine warmed up"),
            Ok(Err(e)) => tracing::info!("Embedding engine init skipped: {e}"),
            Err(e) => tracing::warn!("Embedding engine warmup failed: {e}"),
        }
        // Tier-2 external sweep (#1051): periodic incremental walk of the
        // configured extra paths. Boot reindex above is sweep #0; this loop
        // keeps the external collection current afterwards (ADR-002).
        crate::memory::freshness::spawn_external_sweep();
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

    // Link sessions that predate the project they belong to, or that were
    // created before anything linked them at all (#1445). One sweep of the
    // unassigned rows, idempotent, so a later run only picks up what this one
    // could not. Backgrounded because it touches every unassigned session and
    // nothing on the startup path is waiting on the answer.
    {
        let project_svc = crate::services::ProjectService::new(service_context.clone());
        tokio::spawn(async move {
            if let Err(e) = project_svc.backfill_unassigned_sessions().await {
                tracing::warn!(error = %e, "session/project backfill failed");
            }
        });
    }

    // Spawn RSI background engine (digest + periodic analysis). #1063: the
    // engine task always spawns and gates itself per cycle from the live
    // config mirror (headless daemons default OFF, TUI default ON).
    let (rsi_tx, mut rsi_rx) = tokio::sync::mpsc::unbounded_channel();
    crate::brain::rsi::spawn_rsi_engine(db.pool().clone(), config, rsi_tx, headless);

    // Resolve RTK in the background (auto-downloads on first use if missing) so
    // the first bash command never blocks on it.
    crate::rtk::warm_up();

    // Get working directory
    let working_directory = std::env::current_dir().unwrap_or_default();

    // Build dynamic system brain from workspace files
    let brain_path = BrainLoader::resolve_path();
    let brain_loader = BrainLoader::new(brain_path.clone());

    let runtime_info = RuntimeInfo {
        model: Some(provider.default_model().to_string()),
        provider: Some(provider.name().to_string()),
        // Collapse $HOME → ~ so the system prompt doesn't leak the
        // local username into every request and burn cache slots
        // varying per-machine.
        working_directory: Some(crate::brain::tools::error::collapse_home(
            &working_directory,
        )),
    };

    // The feedback/performance digest is a maintenance WARNING surface — it
    // lives in ~/.opencrabs/rsi/digest.md (written by write_startup_digest),
    // NOT in the LLM context. Injecting it here put unrelated tool-failure
    // stats into every session's system prompt (and into the context token
    // count) for no benefit to the conversation.
    let mut system_brain = brain_loader.build_core_brain(Some(&runtime_info));

    // Lazy-tools mode: the model only sees the CORE tool schemas + `tool_search`.
    // Tell it explicitly so it reaches for `tool_search` instead of assuming a
    // capability is missing. Without this nudge the model can give up on a task
    // whose tool simply wasn't injected.
    if config.agent.lazy_tools {
        system_brain.push_str(&crate::brain::tools::catalog::tool_access_prompt(headless));
    }

    // Propagate persisted auto-always approval policy to the agent service so
    // the tool loop bypasses approval entirely. Without this, the TUI silently
    // approves in its callback but `tool_loop.auto_approve_tools` stays false,
    // and `context.rs` injects "AUTO-APPROVE OFF — tool approval is REQUIRED"
    // into the compaction/continuation system prompt — which the LLM then
    // echoes back telling the user to enable auto-approve.
    let auto_approve_tools = config.agent.approval_policy.as_str() == "auto-always";

    // Create agent service with dynamic system brain
    let agent_service = Arc::new(
        AgentService::new(provider.clone(), service_context.clone(), config)
            .await
            .with_system_brain(system_brain.clone())
            .with_working_directory(working_directory.clone())
            .with_auto_approve_tools(auto_approve_tools),
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

    // Forward RSI notifications to TUI as system messages
    {
        let rsi_event_sender = event_sender.clone();
        tokio::spawn(async move {
            use crate::tui::events::TuiEvent;
            while let Some(notification) = rsi_rx.recv().await {
                // Secrets in the notification's free-text fields are redacted
                // inside format_rsi_notification before this ever reaches the
                // screen (2026-06-07: RSI alerts were exposing keys).
                let text = crate::brain::rsi::format_rsi_notification(&notification);
                // RSI alerts apply to the whole agent (not any single
                // session) — Uuid::nil() bypasses the session-scope filter
                // so the alert renders in whichever pane the user is on.
                let _ = rsi_event_sender.send(TuiEvent::SystemMessage {
                    session_id: uuid::Uuid::nil(),
                    text,
                });
            }
        });
    }

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
                // Compaction is now fully silent — summary goes to memory log only
                ProgressEvent::Compacting { .. } => return,
                ProgressEvent::CompactionSummary { .. } => return,
                ProgressEvent::BuildLine(line) => progress_sender.send(TuiEvent::BuildLine(line)),
                ProgressEvent::RestartReady {
                    status,
                    binary_path,
                } => progress_sender.send(TuiEvent::RestartReady {
                    status,
                    binary_path,
                }),
                ProgressEvent::TokenCount(count) => {
                    // Real count from the API — update baseline.
                    last_ctx_tokens.store(count as u32, std::sync::atomic::Ordering::Relaxed);
                    progress_sender.send(TuiEvent::TokenCountUpdated { session_id, count })
                }
                ProgressEvent::ReasoningChunk { text } => {
                    // Reasoning/thinking chunks ARE model output and must count
                    // toward the live tok/s footer. 2026-05-29 user report:
                    // a 38s reasoning-heavy turn showed 65 tok total / 3 tok/s
                    // because only StreamingChunk (final completion text)
                    // emitted token-count events, while the 1000+ tokens of
                    // visible thinking went silent on the meter. Per the
                    // user's spec: "should count only when its outputting
                    // tokens like thinking or completion or tool calls".
                    let chunk_tokens = crate::brain::tokenizer::count_tokens(&text) as u32;
                    let _ = progress_sender.send(TuiEvent::StreamingOutputTokens {
                        session_id,
                        tokens: chunk_tokens,
                    });
                    progress_sender.send(TuiEvent::ReasoningChunk { session_id, text })
                }
                ProgressEvent::QueuedUserMessage { text } => {
                    progress_sender.send(TuiEvent::QueuedUserMessage { session_id, text })
                }
                ProgressEvent::SelfHealingAlert { message } => {
                    progress_sender.send(TuiEvent::SystemMessage {
                        session_id,
                        text: format!("🔧 {}", message),
                    })
                }
                ProgressEvent::StripStreamedContent { bytes, reason } => {
                    progress_sender.send(TuiEvent::StripStreamedContent {
                        session_id,
                        bytes,
                        reason,
                    })
                }
                ProgressEvent::ProviderSwitched {
                    to_name,
                    to_model,
                    reason,
                    ..
                } => progress_sender.send(TuiEvent::ProviderSwitched {
                    session_id,
                    to_name,
                    to_model,
                    reason,
                }),
                ProgressEvent::RetryAttempt {
                    attempt,
                    max,
                    reason,
                } => progress_sender.send(TuiEvent::SystemMessage {
                    session_id,
                    text: format!("⏳ Retry {}/{} — {}", attempt, max, reason),
                }),
                ProgressEvent::SuggestedOptions(options) => {
                    progress_sender.send(TuiEvent::SuggestedOptions {
                        session_id,
                        options,
                    })
                }
            };
            if let Err(e) = result {
                tracing::error!("Progress event channel closed: {}", e);
            }
        });

    // Create message queue callback that checks for queued user messages
    // FOR THE CALLER'S session_id specifically — the agent task in pane B
    // can't accidentally drain pane A's queue because the lookup key is
    // the caller's own session id (the bug fix from 2026-04-27).
    let queued_messages = app.queued_messages.clone();
    let message_queue_callback: crate::brain::agent::MessageQueueCallback =
        Arc::new(move |session_id| {
            let queued_messages = queued_messages.clone();
            Box::pin(async move {
                let mut map = match queued_messages.lock() {
                    Ok(g) => g,
                    Err(_) => return None,
                };
                let entry = map.get_mut(&session_id)?;
                if entry.is_empty() {
                    map.remove(&session_id);
                    return None;
                }
                // Drain the entire stack and join with newlines — same
                // semantics as the prior shared-slot implementation,
                // which also joined accumulated sends.
                //
                // The two halves are joined SEPARATELY. Collapsing them here
                // (the old `plain(joined)`) meant a synthetic entry's full
                // context became its display text, publishing a background
                // task's whole `[System: ...]` block into the transcript
                // (#765). A typed message carries the same text in both, so
                // this is identical for the common case.
                let drained = std::mem::take(entry);
                map.remove(&session_id);
                crate::brain::agent::QueuedUserMessage::join(&drained)
            })
        });

    // Register rebuild tool (schedules a background build via cron)
    tool_registry.register(Arc::new(crate::brain::tools::rebuild::RebuildTool::new()));

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

    // Give channel agents the runtime info so their live-rebuilt brain (#213)
    // keeps the model/provider/working-dir lines after a brain-file edit.
    channel_factory.set_runtime_info(runtime_info.clone());

    // Shared Telegram state for proactive messaging
    #[cfg(feature = "telegram")]
    let telegram_state = Arc::new(crate::channels::telegram::TelegramState::new());
    // Durable plan-card tracking (#809): without it a restart loses which
    // message carries the card, so the next turn posts a second one below the
    // stale card instead of updating it, and the old one can never be removed.
    #[cfg(feature = "telegram")]
    telegram_state
        .set_plan_card_store(crate::db::repository::PlanCardRepository::new(
            db.pool().clone(),
        ))
        .await;
    // Durable follow-up suggestion stash (#1226 item 3): without it a
    // restart orphans every live picker keyboard — buttons stay rendered
    // but taps can only hit the unknown-token strip path.
    #[cfg(feature = "telegram")]
    telegram_state
        .set_followup_store(crate::db::repository::PendingFollowupRepository::new(
            db.pool().clone(),
        ))
        .await;

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

    // Re-register session_search carrying live channel state so discovery
    // rows report turn activity (running/idle, #1203). Registry insert
    // replaces the stateless core instance by name; the daemon path keeps
    // that stateless one.
    #[cfg(feature = "telegram")]
    tool_registry.register(Arc::new(
        crate::brain::tools::session_search::SessionSearchTool::with_telegram(
            db.pool().clone(),
            telegram_state.clone(),
        ),
    ));

    // Register cowork tool — launch a Telegram cowork workspace from anywhere
    // (TUI included); generates the deep link + QR and registers the session.
    #[cfg(feature = "telegram")]
    tool_registry.register(Arc::new(
        crate::brain::tools::cowork_connect::CoworkConnectTool::new(telegram_state.clone()),
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

    // SSH password callback — same shape as the sudo callback, different
    // event variant so the dialog can show "SSH password required" instead
    // of "sudo password required".
    let ssh_sender = app.event_sender();
    let ssh_callback: crate::brain::agent::SshPasswordCallback = Arc::new(move |target| {
        let sender = ssh_sender.clone();
        Box::pin(async move {
            use crate::tui::events::{SshPasswordRequest, SshPasswordResponse, TuiEvent};
            use tokio::sync::mpsc;

            let (response_tx, mut response_rx) = mpsc::unbounded_channel::<SshPasswordResponse>();

            let request = SshPasswordRequest {
                request_id: uuid::Uuid::new_v4(),
                target,
                response_tx,
            };

            sender
                .send(TuiEvent::SshPasswordRequested(request))
                .map_err(|e| {
                    crate::brain::agent::AgentError::Internal(format!(
                        "Failed to send SSH password request: {}",
                        e
                    ))
                })?;

            let response =
                tokio::time::timeout(std::time::Duration::from_secs(120), response_rx.recv())
                    .await
                    .map_err(|_| {
                        crate::brain::agent::AgentError::Internal(
                            "SSH password request timed out (120s)".to_string(),
                        )
                    })?
                    .ok_or_else(|| {
                        crate::brain::agent::AgentError::Internal(
                            "SSH password channel closed".to_string(),
                        )
                    })?;

            Ok(response.password)
        })
    });

    // Create session-updated notification channel — remote channels fire this so the TUI
    // reloads in real-time when Telegram/WhatsApp/Discord/Slack messages are processed.
    let (session_updated_tx, mut session_updated_rx) =
        tokio::sync::mpsc::unbounded_channel::<crate::brain::agent::ChannelSessionEvent>();
    {
        let event_sender = app.event_sender();
        tokio::spawn(async move {
            use crate::brain::agent::ChannelSessionEvent;
            while let Some(event) = session_updated_rx.recv().await {
                let tui_event = match event {
                    ChannelSessionEvent::Updated(id) => {
                        crate::tui::events::TuiEvent::SessionUpdated(id)
                    }
                    ChannelSessionEvent::ProcessingStarted(id) => {
                        crate::tui::events::TuiEvent::ChannelProcessingStarted(id)
                    }
                    ChannelSessionEvent::ProcessingFinished(id) => {
                        crate::tui::events::TuiEvent::ChannelProcessingFinished(id)
                    }
                    ChannelSessionEvent::TitleUpdated(id, title) => {
                        crate::tui::events::TuiEvent::SessionTitleUpdated {
                            session_id: id,
                            title,
                        }
                    }
                };
                if let Err(e) = event_sender.send(tui_event) {
                    tracing::warn!(
                        "ChannelSessionEvent bridge: TUI event channel closed, dropping event: {}",
                        e
                    );
                }
            }
        });
    }

    // Create agent service with approval callback, progress callback, and message queue
    tracing::debug!("Creating agent service with approval, progress, and message queue callbacks");
    let shared_tool_registry = tool_registry;

    // Headless-safe runtime tools (dynamic tools.toml tools, tool_manage,
    // browser). Shared with the cron daemon via tool_setup so every entry point
    // exposes the same set.
    crate::cli::tool_setup::register_runtime_tools(&shared_tool_registry, config);

    // Now that the registry is Arc'd, give it to the channel factory
    channel_factory.set_tool_registry(shared_tool_registry.clone());

    // Sub-agent manager for every channel agent (#1170): the factory is the
    // ONLY sub-agent wiring chat channels get. Without this, tasks_list reads
    // an empty registry in Telegram/WhatsApp/Discord/Slack sessions while the
    // TUI (wired at service build) and the cron daemon work fine.
    channel_factory.set_subagent_manager(subagent_manager.clone());

    // Share session_updated_tx with the factory so channel agents (WhatsApp, Telegram, etc.)
    // trigger real-time TUI refresh when they complete a response.
    channel_factory.set_session_updated_tx(session_updated_tx.clone());

    // Background-task resume producer (#722): when a detached long command
    // finishes, push the completion into the session as a TuiEvent so the TUI
    // injects it mid-turn (if the session is processing) or starts a fresh turn
    // (if idle).
    //
    // A daemon has no TUI to inject into. It builds `app` all the same, but
    // never runs its event loop, so pushing a completion into that channel
    // delivered it nowhere and the discarded send error kept it silent — the
    // whole of #1206. Its only surfaces are channels, so an unclaimed session
    // parks and leaves the moment its channel claims it.
    let bg_event_sender = app.event_sender();
    let message_enqueue_callback: crate::brain::agent::service::MessageEnqueueCallback = if headless
    {
        crate::brain::agent::service::restart_recovery::parking_route()
    } else {
        Arc::new(move |session_id, msg| {
            if let Err(e) = bg_event_sender.send(crate::tui::events::TuiEvent::BackgroundTaskDone {
                session_id,
                context_text: msg.context_text,
                display_text: msg.display_text,
            }) {
                // The completion is gone at this point. Say so: a silently
                // discarded send is what made this class of loss invisible.
                tracing::error!(
                    target: "background_task",
                    "Could not hand session {session_id}'s completion to the TUI: {e}"
                );
            }
        })
    };

    // Fallback destination for a session no channel claims. A sub-agent is
    // reached from a tool with no service context, so unlike a detached
    // command it carries no callback of its own and resolves this instead
    // (#1036).
    crate::brain::agent::service::session_routes::register_local_route(
        message_enqueue_callback.clone(),
    );

    // Anything a previous process was doing died with it: detached commands
    // and sub-agents alike. Account for both and report each into the session
    // that owns it, instead of leaving that session waiting on a result that
    // can never arrive (#763, #1037, #1038).
    let interrupted = crate::brain::agent::service::restart_recovery::recover(
        (!headless).then(|| message_enqueue_callback.clone()),
    )
    .await;
    if interrupted > 0 {
        tracing::info!("Recovered {interrupted} item(s) interrupted by restart");
    }

    // Retire sub-agent sessions nobody will revisit (#931). Startup-only: they
    // are created per `spawn_agent` and read by nothing afterwards, so there is
    // no window where sweeping sooner would help. Never fatal — a failed sweep
    // costs disk, not correctness.
    match crate::services::SessionService::new(service_context.clone())
        .prune_expired_subagent_sessions(config.agent.subagent_session_ttl_days)
        .await
    {
        Ok(0) => {}
        Ok(n) => tracing::info!("Pruned {n} expired sub-agent session(s)"),
        Err(e) => tracing::warn!("Sub-agent session sweep failed: {e:#}"),
    }

    let agent_service = Arc::new(
        AgentService::new(provider.clone(), service_context.clone(), config)
            .await
            .with_system_brain(system_brain)
            .with_brain_rebuild(
                brain_loader.clone(),
                Some(runtime_info.clone()),
                true,
                config.agent.lazy_tools,
            )
            .with_tool_registry(shared_tool_registry.clone())
            .with_approval_callback(Some(approval_callback))
            .with_progress_callback(Some(progress_callback))
            .with_message_queue_callback(Some(message_queue_callback))
            .with_message_enqueue_callback(Some(message_enqueue_callback))
            .with_sudo_callback(Some(sudo_callback))
            .with_ssh_callback(Some(ssh_callback))
            .with_working_directory(working_directory.clone())
            .with_brain_path(brain_path)
            .with_subagent_manager(subagent_manager)
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
    // Sessions roused by the resume path below (see the wake pass, #1227) so
    // they are not nudged a second time by it.
    let mut resumed_session_ids: std::collections::HashSet<uuid::Uuid> =
        std::collections::HashSet::new();

    // #12/#1242 boot accounting: interrupted-row counts, user vs system
    // origin split, and parked (route-unclaimed) sessions, read once by the
    // end-of-boot summary line emitted further down.
    let boot_found = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let boot_parked = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let boot_found_system = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    {
        use crate::brain::agent::service::boot_report;
        let pending_repo = crate::db::PendingRequestRepository::new(db.pool().clone());
        // Say it here, once, loudly: a table that cannot take a row means
        // nothing in this run survives the next restart, and the scan below
        // finding zero rows is then not health, it is blindness (#1401).
        if let Err(e) = pending_repo.probe().await {
            tracing::warn!(
                "restart recovery DISABLED for this run: {e:#}. Interrupted turns will not be \
                 resumed after the next restart."
            );
            boot_report::record_tracking_disabled(format!("{e:#}"));
        }
        match pending_repo.get_interrupted().await {
            Ok(requests) if !requests.is_empty() => {
                tracing::info!(
                    "Found {} interrupted request(s) — resuming on startup",
                    requests.len()
                );
                boot_found.store(requests.len(), std::sync::atomic::Ordering::Relaxed);
                // Clear the table so these don't resume again if THIS run also crashes
                if let Err(e) = pending_repo.clear_all().await {
                    tracing::warn!(error = %e, "failed to clear pending items");
                }
                let agent = app.agent_service().clone();
                let session_repo = crate::db::SessionRepository::new(db.pool().clone());
                // Dedup by session_id — only resume each session once
                let mut seen = std::collections::HashSet::new();
                for req in requests {
                    // #12 AC3: account system-origin rows at ROW level (found
                    // counts rows; the dedup below may skip duplicates, so the
                    // user/system split is taken before it).
                    if req.origin == "system" {
                        boot_found_system.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    if let Ok(session_id) = uuid::Uuid::parse_str(&req.session_id) {
                        // Every row is a session that was mid-turn when the
                        // previous process died, duplicate rows included; the
                        // ledger dedups, so the summary counts sessions.
                        boot_report::record_interrupted(session_id);
                        if !seen.insert(session_id) {
                            continue;
                        }
                        resumed_session_ids.insert(session_id);
                        // #12: a system-origin row is a PUSH-initiated turn
                        // (session_notify / background-task completion) killed
                        // mid-tool. Boot must NOT replay the LLM turn here —
                        // re-running the interrupted tool call could
                        // double-execute side effects (installs, binary swaps,
                        // sends). Re-deliver the ORIGINAL push text through the
                        // normal wake path instead: deliver_or_park → the #1224
                        // route claim → the enqueue callback → a fresh tracked
                        // push turn, deleted at exit and re-captured if killed
                        // again — no perpetual rows (#729 holds).
                        if req.origin == "system" {
                            crate::brain::agent::service::restart_recovery::deliver_or_park(
                                session_id,
                                crate::brain::agent::QueuedUserMessage {
                                    context_text: req.user_message.clone(),
                                    display_text: format!(
                                        "⚠️ A push-initiated turn was interrupted by a \
                                         restart — re-delivering the push (session {})",
                                        &session_id.simple().to_string()[..8]
                                    ),
                                    origin: crate::brain::agent::PushOrigin::Recovery,
                                    bg_meta: None,
                                },
                            );
                            continue;
                        }
                        boot_report::record_resumed(session_id);
                        // Restore the session's saved working directory before
                        // resuming so the agent runs tools in the right CWD.
                        // Note: agent_service shares one global WD lock across
                        // sessions, so concurrent multi-session resumes with
                        // different WDs can race — resolving that needs
                        // per-session WD threading through the request pipeline
                        // and is out of scope here.
                        if let Ok(Some(s)) = session_repo.find_by_id(session_id).await
                            && let Some(ref dir_str) = s.working_directory
                        {
                            let p = std::path::PathBuf::from(dir_str);
                            if p.is_dir() {
                                // Per-session (#703): seed the resumed session's
                                // own handle so a concurrent resume with a
                                // different wd can't clobber it via the global.
                                agent.set_working_directory_for_session(session_id, p);
                            }
                        }
                        let agent = agent.clone();
                        let ev_tx = resume_event_sender.clone();
                        let channel = req.channel.clone();
                        let channel_chat_id = req.channel_chat_id.clone();
                        // This revive runs a turn without passing through the
                        // ingress handler that would claim the session, so
                        // until its channel sees another inbound message the
                        // route map says nothing about it. Record who it
                        // belongs to now, so a detached command or sub-agent
                        // finishing during that window parks for the right
                        // channel instead of falling back to whatever this
                        // process booted on (#1206).
                        if channel != "tui" {
                            crate::brain::agent::service::restart_recovery::expect_channel_route(
                                session_id,
                            );
                        }
                        tracing::info!(
                            "Resuming session {} (channel: {}, chat_id: {:?})",
                            &req.session_id[..8.min(req.session_id.len())],
                            channel,
                            channel_chat_id,
                        );

                        // TUI: wire cancel token and send response via TuiEvent
                        // Non-TUI: send response back to the originating channel
                        let tg = telegram_state.clone();
                        #[cfg(feature = "discord")]
                        let dc = discord_state.clone();
                        #[cfg(feature = "whatsapp")]
                        let wa = whatsapp_state.clone();
                        #[cfg(feature = "slack")]
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
                        // Register cancel token for channel sessions so incoming
                        // messages cancel the resume (prevents concurrent agent calls).
                        // Also send a visible status message so the user knows work
                        // is resuming (otherwise they send new messages that cancel it).
                        // Telegram: use full streaming pipeline (typing, tool msgs, edit loop).
                        // The bot may not be authenticated yet at startup, so we spawn a
                        // task that waits for it before calling resume_session.
                        if channel == "telegram"
                            && let Some(ref cid) = channel_chat_id
                            && let Ok(chat_id) = cid.parse::<i64>()
                        {
                            let chat = teloxide::types::ChatId(chat_id);
                            let agent = agent.clone();
                            let tg = tg.clone();
                            let boot_parked_tg = boot_parked.clone();
                            tokio::spawn(async move {
                                // This path always knew the bot might not be
                                // up yet and waited for it. The bg-resume
                                // paths did not, and dropped the wake instead
                                // (#1242). One definition now, so the two
                                // startup flush paths cannot drift back into
                                // disagreeing about what "ready" means.
                                // Gate only: the re-delivery below goes through
                                // deliver_or_park, which does not need the bot
                                // handle — the await exists so a boot window
                                // that never opens is counted, not silent.
                                let Some(_bot) = crate::channels::transport_ready::await_transport(
                                    "telegram",
                                    session_id,
                                    || tg.bot(),
                                )
                                .await
                                else {
                                    // The channel never came up inside the
                                    // grace window: this wake is not slow,
                                    // it is gone (#1242). Count it so the
                                    // boot summary names the session that
                                    // never resumed instead of silence.
                                    boot_report::record_failed();
                                    return;
                                };
                                let prompt = "[System: A restart just occurred while you were \
                                        processing a request. Read the conversation context and continue \
                                        where you left off naturally. Do not mention the restart or \
                                        any interruption — just pick up seamlessly.]"
                                        .to_string();
                                // Wait up to READY_WAIT_SECS for the bot to authenticate.
                                // #1242: this used to give up silently — the pending rows
                                // were already cleared above, so that was permanent loss.
                                // Past the bound the prompt is PARKED: it rides
                                // deliver_or_park until the #1224 route restore claims the
                                // session; the enqueue callback then runs it through the
                                // same full streaming pipeline.
                                let Some(bot) = crate::channels::bg_resume::wait_ready(
                                    || tg.bot(),
                                    "startup resume: telegram bot",
                                )
                                .await
                                else {
                                    boot_parked_tg
                                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    crate::brain::agent::service::restart_recovery::deliver_or_park(
                                        session_id,
                                        crate::brain::agent::QueuedUserMessage {
                                            context_text: prompt,
                                            display_text: format!(
                                                "🔁 Startup replay parked until its route \
                                                 claim (#1242): session {}",
                                                &session_id.simple().to_string()[..8]
                                            ),
                                            origin: crate::brain::agent::PushOrigin::Recovery,
                                            bg_meta: None,
                                        },
                                    );
                                    return;
                                };
                                // Resumed turns must land in the originating
                                // forum topic, not the group's General channel
                                // (issue #130 proactive path). Prefer the
                                // topic bound to THIS session; the chat-wide
                                // "most recent message" lookup is only a
                                // fallback, because in a forum it resolves to
                                // whichever topic spoke last (#1200).
                                //
                                // At startup the in-memory binding is usually
                                // still empty, so this mostly falls back here.
                                // That is today's behaviour, not a regression:
                                // it can only improve once a topic is bound.
                                let thread_id = match tg.session_topic(session_id).await {
                                    // Through the delivery boundary: a
                                    // General-bound session has no thread,
                                    // not thread 1 (#1319).
                                    Some(topic) => {
                                        crate::channels::telegram::session_resolve::delivery_thread_id(
                                            Some(topic),
                                        )
                                    }
                                    None => {
                                        crate::channels::telegram::send::latest_thread_id_for_chat(
                                            chat.0,
                                        )
                                        .await
                                    }
                                };
                                match crate::channels::telegram::handler::resume_session(
                                    bot, chat, thread_id, session_id, prompt, agent, tg,
                                    false, // boot replay of an EXISTING row: resume-of-resume must stay untracked (#729/#12)
                                )
                                .await
                                {
                                    Ok(()) => boot_report::record_delivered(),
                                    Err(e) => {
                                        tracing::error!(
                                            "Telegram resume failed for session {}: {}",
                                            session_id,
                                            e
                                        );
                                        boot_report::record_failed();
                                    }
                                }
                            });
                            continue;
                        }
                        tokio::spawn(async move {
                            // Do NOT swap the shared agent's provider here.
                            // The agent_service is shared across all sessions —
                            // swapping it for one session's saved provider
                            // contaminates every other session. The FallbackProvider
                            // handles model remapping automatically.

                            let prompt = "[System: A restart just occurred while you were \
                                processing a request. Read the conversation context and continue \
                                where you left off naturally. Do not mention the restart or \
                                any interruption — just pick up seamlessly.]"
                                .to_string();
                            match agent
                                .resume_interrupted_turn(
                                    session_id,
                                    prompt,
                                    None,
                                    Some(token),
                                    None,
                                    None,
                                    &channel,
                                    channel_chat_id.as_deref(),
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
                                    boot_report::record_delivered();
                                    // A revived sub-agent session (#110): its
                                    // result belongs to the session that spawned
                                    // it, not to the surface-less default — route
                                    // it to the parent, finalize the status file,
                                    // and skip the surface event nobody reads.
                                    if let Some(mut agent_status) = crate::brain::agent::service::work_status::WorkStatus::find_agent_by_session(&session_id.to_string()) {
                                        crate::brain::agent::service::restart_recovery::deliver_revived_agent_outcome(&mut agent_status, Ok(&response.content));
                                        return;
                                    }
                                    match channel.as_str() {
                                        "tui" => {
                                            let _ = ev_tx.send(
                                                crate::tui::events::TuiEvent::ResponseComplete {
                                                    session_id,
                                                    response,
                                                },
                                            );
                                        }
                                        #[cfg(feature = "discord")]
                                        "discord" => {
                                            if let Some(ref cid) = channel_chat_id
                                                && let Ok(ch_id) = cid.parse::<u64>()
                                                && let Some(http) = dc.http().await
                                            {
                                                let channel =
                                                    serenity::model::id::ChannelId::new(ch_id);
                                                if let Err(e) =
                                                    channel.say(&http, &response.content).await
                                                {
                                                    tracing::warn!(error = %e, "failed to send Discord response");
                                                }
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
                                                if let Err(e) = client.send_message(jid, msg).await
                                                {
                                                    tracing::warn!(error = %e, "failed to send WhatsApp response");
                                                }
                                            }
                                        }
                                        #[cfg(feature = "slack")]
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
                                                if let Err(e) =
                                                    session.chat_post_message(&req).await
                                                {
                                                    tracing::warn!(error = %e, "failed to send Slack response");
                                                }
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
                                    boot_report::record_failed();
                                    // A revived sub-agent session whose resume
                                    // failed (#110): the parent is waiting on
                                    // this outcome either way — report and
                                    // finalize instead of dropping it.
                                    if let Some(mut agent_status) = crate::brain::agent::service::work_status::WorkStatus::find_agent_by_session(&session_id.to_string()) {
                                        crate::brain::agent::service::restart_recovery::deliver_revived_agent_outcome(&mut agent_status, Err(&e.to_string()));
                                        return;
                                    }
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

    // One grace window past the last dispatch, every bounded wait in the pass
    // above has resolved, so a resume still unaccounted for is genuinely
    // missing rather than slow (#1242). The line logs even on boots with
    // nothing to recover: that is the fact that makes its absence meaningful
    // on the boots that did.
    crate::brain::agent::service::boot_report::schedule_summary(
        crate::brain::agent::service::restart_recovery::ROUTE_GRACE,
    );

    // #1242 (AC2): one end-of-boot line answering "did everything resume?" —
    // emitted EVERY boot, zero counts included, delayed past READY_WAIT_SECS
    // so parked outcomes settle before it fires.
    {
        let found = boot_found.clone();
        let parked = boot_parked.clone();
        let found_system = boot_found_system.clone();
        let resumed: Vec<String> = resumed_session_ids
            .iter()
            .map(|id| id.simple().to_string()[..8].to_string())
            .collect();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(
                crate::channels::bg_resume::READY_WAIT_SECS + 1,
            ))
            .await;
            tracing::info!(
                target: "boot-resume",
                "Boot resume summary (#1242/#12): interrupted={} (user={} system={}) resumed={} [{}] parked_awaiting_route={}",
                found.load(std::sync::atomic::Ordering::Relaxed),
                found.load(std::sync::atomic::Ordering::Relaxed)
                    - found_system.load(std::sync::atomic::Ordering::Relaxed),
                found_system.load(std::sync::atomic::Ordering::Relaxed),
                resumed.len(),
                resumed.join(","),
                parked.load(std::sync::atomic::Ordering::Relaxed)
            );
        });
    }

    // Wake (#1227): recently-active bound sessions that were NOT mid-turn at
    // boot have no journal row, so they look dead until someone pokes one.
    // #1224 re-registers their routes (draining parked reports) at connect,
    // but quietly. Log the stranded set so it is auditable; purely
    // informational — it runs no turn, re-executes nothing, sends no message
    // (the former "I'm back" bubble was removed per #34).
    #[cfg(feature = "telegram")]
    crate::channels::telegram::resume::wake_recently_active(
        db.pool().clone(),
        &resumed_session_ids,
    )
    .await;

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

    // Report what the instance guard preempted on the way in. The preemption
    // itself moved to the top of boot (#1072): it used to run here, after the
    // database, the provider and the memory reindex had already been set up
    // alongside the daemon's, which is the duplicate work the guard exists to
    // stop. Only the user-facing message is left at this point, because it
    // needs the TUI event channel that does not exist that early.
    {
        let preempted = &preempted_instances;
        if !preempted.is_empty() {
            use crate::tui::events::TuiEvent;
            let mut lines = Vec::new();
            for inst in preempted {
                let chans = if inst.channels.is_empty() {
                    "channels".to_string()
                } else {
                    inst.channels.join(", ")
                };
                if inst.stopped {
                    lines.push(format!(
                        "Shut down a background instance (PID {}) that was holding {} so this session can own them. It will stay down until you start the daemon again.",
                        inst.pid, chans
                    ));
                } else {
                    lines.push(format!(
                        "A background instance (PID {}) holding {} did not stop (likely running as another user) — it may still contend for the connection. Stop it manually so this session can take over.",
                        inst.pid, chans
                    ));
                }
            }
            let _ = app.event_sender().send(TuiEvent::SystemMessage {
                session_id: uuid::Uuid::nil(),
                text: lines.join("\n"),
            });
        }
    }

    // Initial channel spawn — reconcile against current config
    channel_manager.reconcile(config).await;

    // Spawn config hot-reload watcher — fires on any change to config.toml, keys.toml,
    // or commands.toml without requiring a restart.
    {
        use crate::tui::events::TuiEvent;
        use crate::utils::config_watcher::{self, ReloadCallback};

        let mut callbacks: Vec<ReloadCallback> = Vec::new();

        // Tool availability — re-register config/key-gated tools (Brave, EXA,
        // image generation, vision/video) on the shared registry so a key added
        // to keys.toml (or a flipped `enabled` flag) is picked up at runtime,
        // with no restart. Channels list tools from this same registry per
        // message, so the change reaches the daemon's channels immediately.
        {
            let registry = shared_tool_registry.clone();
            callbacks.push(Arc::new(move |cfg: crate::config::Config| {
                register_config_dependent_tools(&registry, &cfg);
                tracing::info!("ConfigWatcher: re-registered config-dependent tools");
            }));
        }

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
                let sender = sender.clone();
                tokio::spawn(async move {
                    match crate::brain::provider::create_provider(&cfg).await {
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
                    // #1249: the chain half of `[providers.fallback]` reloads
                    // here too. Swapping only the primary above left a chain
                    // frozen at process start, so a provider deleted from
                    // `fallback_chain` kept receiving live traffic. Runs even
                    // when the primary rebuild failed — the two are
                    // independent, and a stale chain is exactly the state
                    // being fixed.
                    agent.reload_fallback_providers(&cfg).await;
                    // Fire AFTER the swap so the TUI refresh (commands, approval
                    // policy, and the context-budget footer) reads the new
                    // provider's context window, not the old one.
                    let _ = sender.send(TuiEvent::ConfigReloaded);
                });
            }));
        }

        // Debug logging toggle (#678) — apply agent.debug_logs live, preserving
        // the launch-time --debug flag. Lets a non-technical user (or the agent)
        // flip file logging on/off by editing config.toml with no restart.
        callbacks.push(Arc::new(move |cfg: crate::config::Config| {
            crate::logging::apply_debug_logs_from_config(cfg.agent.debug_logs);
        }));

        // Channel lifecycle — spawn/stop channels when enabled flag changes
        {
            let channel_mgr = channel_manager.clone();
            callbacks.push(Arc::new(move |cfg: crate::config::Config| {
                let mgr = channel_mgr.clone();
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(mgr.reconcile(&cfg));
                });
            }));
        }

        // force_default push (#466): when the active default provider's
        // section opts in, a reload broadcasts its pair to every
        // non-archived session. Live sessions apply it on their next
        // message through the existing sync path.
        {
            let svc_ctx = crate::services::ServiceContext::new(db.pool().clone());
            callbacks.push(Arc::new(move |cfg: crate::config::Config| {
                let session_svc = crate::services::SessionService::new(svc_ctx.clone());
                tokio::spawn(async move {
                    match crate::services::force_default::apply_force_default(&cfg, &session_svc)
                        .await
                    {
                        Ok(0) => {}
                        Ok(n) => tracing::info!("force_default: {n} session(s) switched"),
                        Err(e) => tracing::error!("force_default push failed: {e:#}"),
                    }
                });
            }));
        }

        // Telegram command refresh — re-register bot commands when commands.toml changes
        #[cfg(feature = "telegram")]
        {
            let tg_state = telegram_state.clone();
            callbacks.push(Arc::new(move |_cfg: crate::config::Config| {
                let state = tg_state.clone();
                tokio::spawn(async move {
                    if let Some(bot) = state.bot().await {
                        crate::channels::telegram::register_bot_commands(&bot).await;
                        tracing::info!("ConfigWatcher: refreshed Telegram bot commands");
                    }
                });
            }));
        }

        // Out-of-band alert when a hot reload does NOT cleanly apply the
        // on-disk config (#534, mirror of upstream #517): the old path was
        // log-only, so an operator whose config.toml edit failed to parse got
        // no signal that the process kept serving the previous provider set.
        // Surface it as a global TUI notice AND, best-effort, a Telegram DM to
        // the owner so a headless daemon (the real-world case) is not silent.
        let notify_sender = app.event_sender();
        #[cfg(feature = "telegram")]
        let notify_tg = telegram_state.clone();
        let notify: config_watcher::ReloadNotify = Arc::new(move |msg: String| {
            let _ = notify_sender.send(TuiEvent::SystemMessage {
                session_id: uuid::Uuid::nil(),
                text: msg.clone(),
            });
            #[cfg(feature = "telegram")]
            {
                let state = notify_tg.clone();
                tokio::spawn(async move {
                    use teloxide::prelude::Requester;
                    if let (Some(bot), Some(owner)) =
                        (state.bot().await, state.owner_chat_id().await)
                    {
                        // Review F10: failure warn had no chat id and success
                        // was invisible — the audit's named WEAK site.
                        let len = msg.len();
                        let hash8 = crate::channels::telegram::telemetry::content_hash8(&msg);
                        match bot.send_message(teloxide::types::ChatId(owner), msg).await {
                            Ok(m) => {
                                crate::channels::telegram::telemetry::log_send_success(
                                    "system",
                                    "config_alert",
                                    "-",
                                    "owner_dm",
                                    "send",
                                    owner,
                                    None,
                                    m.id.0,
                                    len,
                                    &hash8,
                                );
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "ConfigWatcher: owner DM of config alert failed (chat={owner}): {e}"
                                );
                            }
                        }
                    }
                });
            }
        });

        // spawn() returns a JoinHandle; the watcher runs detached, so there is
        // nothing to hold. Don't bind a handle we never await or abort.
        config_watcher::spawn(callbacks, Some(notify));
    }

    // Set force onboard flag if requested
    if force_onboard {
        app.force_onboard = true;
    }

    // Resume a specific session (e.g. after /rebuild restart). Accepts a full
    // UUID or the prefix that `session list` shows, routed through the shared
    // resolver so every CLI session-id entry point resolves identically.
    if let Some(ref sid) = session_id {
        match uuid::Uuid::parse_str(sid) {
            Ok(uuid) => app.resume_session_id = Some(uuid),
            Err(_) => {
                let session_svc = crate::services::SessionService::new(service_context.clone());
                match session_svc
                    .list_sessions(crate::db::repository::SessionListOptions {
                        include_archived: true,
                        ..Default::default()
                    })
                    .await
                {
                    Ok(sessions) => {
                        match crate::cli::session_resolve::resolve_session_id(&sessions, sid) {
                            Ok(uuid) => app.resume_session_id = Some(uuid),
                            Err(e) => eprintln!("could not resolve --session '{sid}': {e}"),
                        }
                    }
                    Err(e) => {
                        eprintln!("could not list sessions to resolve --session '{sid}': {e}")
                    }
                }
            }
        }
    }

    // Spawn cron scheduler — polls every 60s, executes jobs in the user's active session.
    // One scheduler per profile machine-wide (#444): if another process (e.g. a
    // multi-profile `daemon` that also covers this profile) already owns the
    // scheduler, don't spawn a second or every due job double-fires. The guard
    // is bound at function scope so it lives for the whole session; the OS
    // releases it on process exit.
    let active_profile = crate::config::profile::active_profile()
        .unwrap_or("default")
        .to_string();
    let _scheduler_lock = crate::config::profile::acquire_scheduler_lock(&active_profile);
    if _scheduler_lock.is_some() {
        let cron_repo = crate::db::CronJobRepository::new(db.pool().clone());
        let cron_run_repo = crate::db::CronJobRunRepository::new(db.pool().clone());
        // Rebuild outcomes must reach the session that asked (#304): a
        // failed background build used to be log-only while the TUI waited
        // for a reload that never came.
        let rebuild_notify_tx = app.event_sender();
        let session_notifier: crate::cron::SessionNotifier =
            std::sync::Arc::new(move |session_id, text| {
                if rebuild_notify_tx
                    .send(crate::tui::events::TuiEvent::SystemMessage { session_id, text })
                    .is_err()
                {
                    tracing::warn!("rebuild notifier: TUI event channel closed");
                }
            });
        let cron_scheduler = crate::cron::CronScheduler::new(
            cron_repo,
            cron_run_repo,
            channel_factory.clone(),
            service_context.clone(),
        )
        .with_session_notifier(session_notifier);
        // Detached task; the JoinHandle isn't awaited or aborted anywhere.
        cron_scheduler.spawn();
        tracing::info!("Cron scheduler spawned");
    } else {
        tracing::info!(
            "Cron scheduler for '{active_profile}' already running elsewhere — not spawning a second"
        );
    }

    // Spawn A2A gateway if configured
    if config.a2a.enabled {
        let a2a_agent = channel_factory.create_agent_service().await;
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
    // Capture provider/model/tools/skills for the banner; we need these again on
    // exit after `app` is moved into `tui::run`.
    let banner_provider = provider.name().to_string();
    let banner_model = provider.default_model().to_string();
    let banner_tools = shared_tool_registry.list_tools();
    let banner_skills: Vec<String> = crate::brain::skills::load_all_skills()
        .iter()
        .map(|s| format!("/{}", s.name))
        .collect();

    print_terminal_banner(
        &banner_provider,
        &banner_model,
        &banner_tools,
        &banner_skills,
        BannerKind::Start,
    );

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

    print_terminal_banner(
        &banner_provider,
        &banner_model,
        &banner_tools,
        &banner_skills,
        BannerKind::Exit,
    );

    Ok(())
}

#[derive(Copy, Clone)]
enum BannerKind {
    Start,
    Exit,
}

/// Print the OpenCrabs banner (logo + tagline + version/provider/model +
/// tools + quick commands + tips) to the terminal before the TUI takes
/// over and again after it exits. Mirrors the in-TUI header card so the
/// user has a persistent record in their terminal scrollback.
fn print_terminal_banner(
    provider: &str,
    model: &str,
    tools: &[String],
    skills: &[String],
    kind: BannerKind,
) {
    // ANSI color codes (24-bit).
    const ORANGE: &str = "\x1b[38;2;215;100;20m";
    const ORANGE_IT: &str = "\x1b[3;38;2;215;100;20m";
    const CYAN: &str = "\x1b[1;36m";
    const DIM: &str = "\x1b[2;37m";
    const BOLD: &str = "\x1b[1m";
    const RESET: &str = "\x1b[0m";

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

    let pool: &[&str] = match kind {
        BannerKind::Start => STARTS,
        BannerKind::Exit => BYES,
    };
    let idx = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as usize)
        % pool.len();
    let tagline_msg = pool[idx];

    let logo = r"   ___                    ___           _
  / _ \ _ __  ___ _ _    / __|_ _ __ _| |__  ___
 | (_) | '_ \/ -_) ' \  | (__| '_/ _` | '_ \(_-<
  \___/| .__/\___|_||_|  \___|_| \__,_|_.__//__/
       |_|";

    let version = env!("CARGO_PKG_VERSION");

    println!();
    println!("{}{}{}{}", BOLD, ORANGE, logo, RESET);
    println!();
    println!(
        "{}🦀 The autonomous AI agent. Self-improving. Every channel.{}",
        ORANGE_IT, RESET
    );
    println!();
    println!(
        "  {bold}{orange}v{version}{reset}  {dim}·{reset}  {cyan}{provider}{reset}  {dim}·{reset}  {cyan}{model}{reset}",
        bold = BOLD,
        orange = ORANGE,
        cyan = CYAN,
        dim = DIM,
        reset = RESET,
    );
    println!();

    if !tools.is_empty() {
        let mut sorted: Vec<&str> = tools.iter().map(|s| s.as_str()).collect();
        sorted.sort();
        println!("  {}Available Tools{}", CYAN, RESET);
        // Word-wrap tools list to ~80 visible columns.
        let joined = sorted.join(", ");
        for line in wrap_plain(&joined, 80) {
            println!("  {}{}{}", DIM, line, RESET);
        }
        println!();
    }

    if !skills.is_empty() {
        let mut sorted: Vec<&str> = skills.iter().map(|s| s.as_str()).collect();
        sorted.sort();
        println!("  {}Skills{}", CYAN, RESET);
        let joined = sorted.join(", ");
        for line in wrap_plain(&joined, 80) {
            println!("  {}{}{}", DIM, line, RESET);
        }
        println!();
    }

    println!("  {}Quick Commands{}", CYAN, RESET);
    println!(
        "  {}/help  /sessions  /models  /skills  /usage  /approve  /rebuild  /doctor{}",
        DIM, RESET
    );
    println!();

    println!("  {}Tips{}", CYAN, RESET);
    println!(
        "  {}@ for files  ·  ! for shell  ·  Shift+Enter for newline  ·  Ctrl+O for older messages{}",
        DIM, RESET
    );
    println!();

    println!("{}{}{}", ORANGE, tagline_msg, RESET);
    println!();
}

/// Whitespace word-wrap to `width` visible columns.
fn wrap_plain(text: &str, width: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in text.split(' ') {
        if cur.is_empty() {
            cur.push_str(word);
        } else if cur.len() + 1 + word.len() <= width {
            cur.push(' ');
            cur.push_str(word);
        } else {
            out.push(std::mem::take(&mut cur));
            cur.push_str(word);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}
