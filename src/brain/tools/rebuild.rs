//! Rebuild Tool
//!
//! Lets the agent build OpenCrabs from source and reload automatically.
//! The build runs as a DETACHED background command through the same
//! `BackgroundTaskManager` every long shell command uses (#1748), so the
//! agent turn isn't blocked for the minutes a release build takes and the
//! surfaces that show detached progress (live timer, tasks_list, status
//! files) work for rebuilds too. A rebuild-specific completion hook
//! exec-restarts into the new binary when the build finishes.

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::path::PathBuf;
use uuid::Uuid;

/// Map an originating channel + chat id (+ optional forum topic) to a cron
/// `deliver_to` target so the background rebuild can report completion and
/// failure into the chat that asked (#305). Only channels with a scheduler
/// delivery arm map; the TUI has no channel target and everything else
/// returns None.
///
/// Telegram forum topics ride the #1451 grammar (`telegram:chat:thread`,
/// parsed by the scheduler's `parse_telegram_target`): when the pending
/// request carries an origin topic the report lands IN that topic (#1457),
/// otherwise the plain `telegram:chat` form keeps the historical default
/// topic delivery. Discord/Slack have no thread component here.
pub(crate) fn rebuild_deliver_target(
    channel: &str,
    chat_id: Option<&str>,
    thread_id: Option<&str>,
) -> Option<String> {
    let chat_id = chat_id?.trim();
    if chat_id.is_empty() {
        return None;
    }
    match channel {
        "telegram" => {
            let thread = thread_id.map(str::trim).filter(|t| !t.is_empty());
            match thread {
                Some(t) => Some(format!("telegram:{chat_id}:{t}")),
                None => Some(format!("telegram:{chat_id}")),
            }
        }
        "discord" | "slack" => Some(format!("{channel}:{chat_id}")),
        _ => None,
    }
}

/// The detached build command: identical compiler semantics to
/// `SelfUpdater::build` (`RUSTFLAGS="-C target-cpu=native"` release build).
/// Source-tree prep (lazy clone / `git pull --ff-only`) happens in-process
/// BEFORE the spawn, so this is a pure compile step the BackgroundTaskManager
/// can time, log and account like any detached command.
pub(crate) fn detached_build_command() -> String {
    "RUSTFLAGS='-C target-cpu=native' cargo build --release".to_string()
}

/// Completion hook for the detached rebuild (#1748). Runs INSTEAD of the
/// generic background-task delivery:
///
/// - success: persist the completion report (the post-exec wake-up turn
///   reads it back, #1105), fan the status out to the originating channel
///   (#305) and AWAIT those sends before exec() replaces the process
///   (#1105), then exec-restart into the fresh binary.
/// - failure: deliver the compiler-output tail into the originating session
///   through the one gated route (TUI and channel sessions alike; replaces
///   the old TUI-only `SessionNotifier`, #304) and into the channel target.
///   Nothing restarts.
pub(crate) fn rebuild_completion_hook(
    session_id: Uuid,
    deliver_to: Option<String>,
    project_root: PathBuf,
    fallback_binary: PathBuf,
    msg_service: crate::services::MessageService,
    mgr: std::sync::Arc<crate::brain::agent::service::background_tasks::BackgroundTaskManager>,
) -> crate::brain::agent::service::background_tasks::CompletionHook {
    use crate::brain::agent::service::background_tasks::HookContext;

    Box::new(move |ctx: HookContext| {
        Box::pin(async move {
            let result = ctx.result;
            if result.success {
                // Same binary resolution as SelfUpdater::build_streaming:
                // prefer the freshly built artifact, fall back to the
                // running exe path.
                let built = project_root
                    .join("target")
                    .join("release")
                    .join("opencrabs");
                let built_path = if built.exists() {
                    built
                } else {
                    fallback_binary
                };

                // Persist the completion report to the session DB so the
                // agent sees context about what triggered the reload on the
                // next turn after the exec restart (#1105).
                let report = format!(
                    "✅ Background rebuild succeeded — binary at {}. Hot-reloading now.",
                    built_path.display()
                );
                if let Err(e) = msg_service
                    .create_message(session_id, "assistant".to_string(), report)
                    .await
                {
                    tracing::error!("rebuild: failed to persist completion report: {e}");
                }

                // Pre-exec warning (#1748 bonus): exec kills every in-flight
                // detached task. The rebuild itself is already marked
                // finished by the manager, so anything still running for
                // this session is OTHER work about to be interrupted; say
                // so before it happens, not just in the post-restart report.
                let others = mgr.running_tasks(session_id);
                if !others.is_empty() {
                    let names = others
                        .iter()
                        .map(|t| t.label.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    tracing::warn!(
                        "rebuild: exec will interrupt in-flight detached tasks for session \
                         {session_id}: {names}"
                    );
                    let note = format!(
                        "⚠️ Reloading now: these detached tasks were still running and \
                         get interrupted by the restart: {names}"
                    );
                    if let Err(e) = msg_service
                        .create_message(session_id, "assistant".to_string(), note)
                        .await
                    {
                        tracing::error!("rebuild: failed to persist pre-exec warning: {e}");
                    }
                }

                // Channel fan-out, awaited BEFORE exec() so the detached
                // sends aren't killed mid-flight (#1105).
                if let Some(ref target) = deliver_to
                    && let Some(h) = crate::cron::scheduler::deliver_result(
                        target,
                        "opencrabs rebuild",
                        "✅ Rebuilt from source — reloading into the new binary now.",
                        None,
                        None,
                    )
                    .await
                    && let Err(e) = h.await
                {
                    tracing::warn!("rebuild: completion notice task failed: {e}");
                }

                // exec() replaces the entire process, this hook included.
                if let Err(e) = crate::brain::SelfUpdater::restart_into(&built_path, session_id) {
                    tracing::error!("rebuild: restart into {} failed: {e}", built_path.display());
                    // The user was told a reload is coming; a failed restart
                    // must not leave them waiting on one (#304 spirit).
                    let note = format!("⚠️ Rebuild finished but the reload failed: {e}");
                    crate::brain::agent::service::session_routes::deliver_to_session(
                        session_id,
                        crate::brain::agent::service::QueuedUserMessage::plain(note),
                        true,
                    );
                }
            } else {
                // Cap the compiler dump: chats don't want 2 MB of cargo
                // output, the last stretch carries the actual errors.
                let mut tail = result.output.as_str();
                if tail.len() > 4000 {
                    let mut cut = tail.len() - 4000;
                    while cut < tail.len() && !tail.is_char_boundary(cut) {
                        cut += 1;
                    }
                    tail = tail.get(cut..).unwrap_or(tail);
                }
                let msg = format!("⚠️ Background rebuild failed:\n{tail}");

                // Session route: reaches the originating TUI session AND
                // channel-owned sessions (replaces the old TUI-only notifier).
                crate::brain::agent::service::session_routes::deliver_to_session(
                    session_id,
                    crate::brain::agent::service::QueuedUserMessage::plain(msg.clone()),
                    true,
                );

                // Channel target (parity with the old deliver_rebuild_status):
                // detached send, no exec follows a failure.
                if let Some(ref target) = deliver_to
                    && let Some(h) = crate::cron::scheduler::deliver_result(
                        target,
                        "opencrabs rebuild",
                        &msg,
                        None,
                        None,
                    )
                    .await
                    && let Err(e) = h.await
                {
                    tracing::warn!("rebuild: completion notice task failed: {e}");
                }
            }
        })
    })
}

/// Shared rebuild launch path (#1748): prep the source tree synchronously,
/// resolve the originating channel, then hand a pure compile to the detached
/// manager with the exec-restart hook attached. Used by the agent tool AND
/// the TUI /rebuild command so both get identical behavior.
pub(crate) async fn run_detached_rebuild(
    mgr: &std::sync::Arc<crate::brain::agent::service::background_tasks::BackgroundTaskManager>,
    session_id: Uuid,
    service_context: &crate::services::ServiceContext,
) -> anyhow::Result<()> {
    // Prepare the source tree BEFORE detaching: auto_detect + lazy
    // clone / `git pull --ff-only` are quick when the source exists, and
    // their failures should reach the requester synchronously instead of as
    // a detached command that dies seconds in.
    let updater =
        crate::brain::SelfUpdater::auto_detect().map_err(|e| anyhow::anyhow!("rebuild: {e}"))?;
    updater
        .ensure_source_tree()
        .map_err(|e| anyhow::anyhow!("rebuild: failed to prepare source tree: {e}"))?;
    let project_root = updater.project_root().to_path_buf();
    let fallback_binary = updater.binary_path().to_path_buf();

    // Resolve WHERE this turn came from so the build's completion and
    // failure notices land back in that chat (#305). The pending-request
    // row for the current turn is alive while this runs and carries
    // channel + chat id. TUI turns map to None: the session route in the
    // hook covers TUI delivery.
    let deliver_to = match crate::db::PendingRequestRepository::new(service_context.pool().clone())
        .find_latest_for_session(session_id)
        .await
    {
        Ok(Some(req)) => {
            let target = rebuild_deliver_target(
                &req.channel,
                req.channel_chat_id.as_deref(),
                req.channel_thread_id.as_deref(),
            );
            match &target {
                Some(t) => tracing::info!("rebuild: status will be delivered to {t}"),
                None => tracing::debug!(
                    "rebuild: no channel delivery target for channel '{}'",
                    req.channel
                ),
            }
            target
        }
        Ok(None) => None,
        Err(e) => {
            tracing::warn!("rebuild: pending-request lookup failed (no delivery): {e}");
            None
        }
    };

    let msg_service = crate::services::MessageService::new(service_context.clone());
    mgr.clone().spawn_command_with_hook(
        session_id,
        project_root.clone(),
        "opencrabs rebuild".to_string(),
        detached_build_command(),
        rebuild_completion_hook(
            session_id,
            deliver_to,
            project_root,
            fallback_binary,
            msg_service,
            std::sync::Arc::clone(mgr),
        ),
    );
    Ok(())
}

/// Agent-callable tool that runs a background rebuild from source.
pub struct RebuildTool;

impl RebuildTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for RebuildTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for RebuildTool {
    fn name(&self) -> &str {
        "rebuild"
    }

    fn description(&self) -> &str {
        "Build OpenCrabs from source (cargo build --release) in the BACKGROUND and auto-reload \
         when it's done. MAINTAINER path, rare: only for applying LOCAL SOURCE EDITS. To \
         upgrade to the latest published release use the `evolve` tool instead (rebuild \
         compiles what is on disk; evolve fetches what was released). Returns \
         immediately: the build runs detached (it does not block you) with a live \
         timer on your surface, and OpenCrabs \
         exec-restarts into the new binary automatically when the build finishes, resuming \
         this session. On build failure a message is delivered; nothing restarts."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::SystemModification]
    }

    async fn execute(&self, _input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        // The build runs DETACHED via the shared BackgroundTaskManager (#1748):
        // live timer, status file and DB accounting come free, and a
        // rebuild-specific completion hook exec-restarts into the new binary
        // when the compile finishes.
        let Some(mgr) = context.background_manager.as_ref() else {
            return Ok(ToolResult::error(
                "rebuild: no background task manager available in this context".to_string(),
            ));
        };
        let Some(service_context) = context.service_context.as_ref() else {
            return Ok(ToolResult::error(
                "rebuild: no service context available to schedule the background build"
                    .to_string(),
            ));
        };

        match run_detached_rebuild(mgr, context.session_id, service_context).await {
            Ok(()) => Ok(ToolResult::success(
                "🔨 Rebuild running detached: the live timer shows progress and it does not \
                 block you. OpenCrabs exec-restarts into the new binary automatically when \
                 it's done; keep working."
                    .to_string(),
            )),
            Err(e) => Ok(ToolResult::error(format!("{e:#}"))),
        }
    }
}
