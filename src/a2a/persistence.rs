//! SQLite persistence for A2A tasks.
//!
//! Tasks are stored as JSON blobs alongside indexed state/timestamps
//! so they survive server restarts.

use super::types::Task;
use crate::db::{Pool, interact_err};
use rusqlite::params;

/// Save or update a task in the database.
pub async fn upsert_task(pool: &Pool, task: &Task) {
    let now = chrono::Utc::now().timestamp();
    let state = format!("{:?}", task.status.state).to_lowercase();
    let data = match serde_json::to_string(task) {
        Ok(d) => d,
        Err(e) => {
            tracing::error!(
                "A2A persistence: failed to serialize task {}: {}",
                task.id,
                e
            );
            return;
        }
    };

    let task_id = task.id.clone();
    let context_id = task.context_id.clone();

    let result = match pool.get().await {
        Ok(conn) => conn
            .interact(move |conn| {
                conn.execute(
                    "INSERT INTO a2a_tasks (id, context_id, state, data, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?5)
                     ON CONFLICT(id) DO UPDATE SET state = ?3, data = ?4, updated_at = ?5",
                    params![task_id, context_id, state, data, now],
                )
            })
            .await
            .map_err(interact_err),
        Err(e) => Err(anyhow::anyhow!("Failed to get connection: {}", e)),
    };

    if let Err(e) = result {
        tracing::error!("A2A persistence: failed to upsert task {}: {}", task.id, e);
    }
}

/// Load all non-terminal tasks from the database (for warm-start after restart).
pub async fn load_active_tasks(pool: &Pool) -> Vec<Task> {
    let rows = match pool.get().await {
        Ok(conn) => match conn
            .interact(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT data FROM a2a_tasks WHERE state NOT IN ('completed', 'failed', 'canceled')",
                )?;
                let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
            })
            .await
        {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                tracing::error!("A2A persistence: failed to load active tasks: {}", e);
                return vec![];
            }
            Err(e) => {
                tracing::error!("A2A persistence: interact error: {}", e);
                return vec![];
            }
        },
        Err(e) => {
            tracing::error!("A2A persistence: failed to get connection: {}", e);
            return vec![];
        }
    };

    rows.iter()
        .filter_map(|data| {
            serde_json::from_str::<Task>(data)
                .inspect_err(|e| tracing::warn!("A2A persistence: bad task JSON: {}", e))
                .ok()
        })
        .collect()
}
