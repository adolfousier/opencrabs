//! Background maintenance service (#241).
//!
//! Coordinates periodic database and memory maintenance:
//! - Sub-agent session TTL expiration and plan file pruning
//! - Memory store GC: orphan vector embeddings, unreferenced content chunks, stale symbols/call_edges
//! - Periodic SQLite `VACUUM` on `memory.db` and `opencrabs.db`

use anyhow::Result;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::services::context::ServiceContext;
use crate::services::session::SessionService;

/// Set while a maintenance sweep is running, so concurrent ticks skip.
static RUNNING_MAINTENANCE: AtomicBool = AtomicBool::new(false);

/// Summary report of a maintenance run.
#[derive(Debug, Clone, Default)]
pub struct MaintenanceReport {
    /// Number of expired sub-agent sessions pruned.
    pub subagents_pruned: usize,
    /// Memory GC report (if memory store is available).
    pub memory_gc: Option<crate::memory::MemoryGcReport>,
    /// Whether `opencrabs.db` was vacuumed.
    pub database_vacuumed: bool,
    /// Whether `memory.db` was vacuumed.
    pub memory_vacuumed: bool,
}

/// Service managing periodic database & memory maintenance sweeps.
pub struct MaintenanceService {
    context: ServiceContext,
}

impl MaintenanceService {
    pub fn new(context: ServiceContext) -> Self {
        Self { context }
    }

    /// Run one complete maintenance cycle across databases.
    pub async fn run_maintenance(&self) -> Result<MaintenanceReport> {
        if !try_enter() {
            tracing::debug!("Maintenance sweep: already in flight, skipping");
            return Ok(MaintenanceReport::default());
        }

        let res = self.run_maintenance_inner().await;
        leave();
        res
    }

    async fn run_maintenance_inner(&self) -> Result<MaintenanceReport> {
        let mut report = MaintenanceReport::default();
        let config = crate::config::Config::load().unwrap_or_default();

        // 1. Prune expired sub-agent sessions
        let session_svc = SessionService::new(self.context.clone());
        let ttl_days = config.agent.subagent_session_ttl_days;
        match session_svc.prune_expired_subagent_sessions(ttl_days).await {
            Ok(pruned) => report.subagents_pruned = pruned,
            Err(e) => tracing::warn!("Maintenance: failed to prune subagent sessions: {e:#}"),
        }

        // 2. Memory Store GC & Vacuum
        if let Ok(store_mutex) = crate::memory::get_store() {
            let gc_res = {
                let store = store_mutex
                    .lock()
                    .map_err(|e| anyhow::anyhow!("Store lock poisoned: {e}"))?;
                store
                    .gc_orphans()
                    .map_err(|e| anyhow::anyhow!("gc_orphans error: {e}"))
            };

            match gc_res {
                Ok(gc_report) => {
                    report.memory_gc = Some(gc_report);
                }
                Err(e) => tracing::warn!("Maintenance: memory GC failed: {e:#}"),
            }

            let vac_res = {
                let store = store_mutex
                    .lock()
                    .map_err(|e| anyhow::anyhow!("Store lock poisoned: {e}"))?;
                store
                    .vacuum_memory()
                    .map_err(|e| anyhow::anyhow!("vacuum_memory error: {e}"))
            };

            match vac_res {
                Ok(()) => report.memory_vacuumed = true,
                Err(e) => tracing::warn!("Maintenance: memory vacuum failed: {e:#}"),
            }
        }

        // 3. Database Vacuum
        let db = crate::db::Database::new_with_pool(self.context.pool());
        match db.vacuum_database().await {
            Ok(()) => report.database_vacuumed = true,
            Err(e) => tracing::warn!("Maintenance: database vacuum failed: {e:#}"),
        }

        tracing::info!(
            subagents_pruned = report.subagents_pruned,
            database_vacuumed = report.database_vacuumed,
            memory_vacuumed = report.memory_vacuumed,
            "Maintenance sweep completed"
        );

        Ok(report)
    }

    /// Spawn periodic background maintenance task.
    pub fn spawn_periodic(context: ServiceContext, interval: Duration) {
        tokio::spawn(async move {
            let svc = MaintenanceService::new(context);
            // Initial post-boot warmup delay so startup is not bottlenecked
            tokio::time::sleep(Duration::from_secs(60)).await;

            loop {
                if let Err(e) = svc.run_maintenance().await {
                    tracing::warn!("Periodic maintenance run encountered error: {e:#}");
                }
                tokio::time::sleep(interval).await;
            }
        });
    }
}

pub(crate) fn try_enter() -> bool {
    RUNNING_MAINTENANCE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

pub(crate) fn leave() {
    RUNNING_MAINTENANCE.store(false, Ordering::Release);
}
