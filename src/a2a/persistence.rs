//! SQLite persistence for A2A tasks.
//!
//! Tasks are stored as JSON blobs alongside indexed state/timestamps
//! so they survive server restarts.

use super::types::Task;
use sqlx::SqlitePool;

/// Save or update a task in the database.
pub async fn upsert_task(pool: &SqlitePool, task: &Task) {
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

    let result = sqlx::query(
        "INSERT INTO a2a_tasks (id, context_id, state, data, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)
         ON CONFLICT(id) DO UPDATE SET state = ?3, data = ?4, updated_at = ?5",
    )
    .bind(&task.id)
    .bind(task.context_id.as_deref())
    .bind(&state)
    .bind(&data)
    .bind(now)
    .execute(pool)
    .await;

    if let Err(e) = result {
        tracing::error!("A2A persistence: failed to upsert task {}: {}", task.id, e);
    }
}

/// Load all non-terminal tasks from the database (for warm-start after restart).
pub async fn load_active_tasks(pool: &SqlitePool) -> Vec<Task> {
    let rows: Vec<(String,)> = match sqlx::query_as(
        "SELECT data FROM a2a_tasks WHERE state NOT IN ('completed', 'failed', 'canceled')",
    )
    .fetch_all(pool)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("A2A persistence: failed to load active tasks: {}", e);
            return vec![];
        }
    };

    rows.iter()
        .filter_map(|(data,)| {
            serde_json::from_str::<Task>(data)
                .inspect_err(|e| tracing::warn!("A2A persistence: bad task JSON: {}", e))
                .ok()
        })
        .collect()
}
