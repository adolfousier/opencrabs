//! Database Models
//!
//! Data structures representing database entities.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ─── Row helpers ─────────────────────────────────────────────────────────────

/// Parse a UUID string column from a rusqlite row.
pub fn uuid_col(row: &rusqlite::Row, col: &str) -> rusqlite::Result<Uuid> {
    let s: String = row.get(col)?;
    Uuid::parse_str(&s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

/// Parse a Unix-timestamp column into `DateTime<Utc>`.
pub fn timestamp_col(row: &rusqlite::Row, col: &str) -> rusqlite::Result<DateTime<Utc>> {
    let ts: i64 = row.get(col)?;
    DateTime::from_timestamp(ts, 0).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Integer,
            format!("Invalid timestamp for {col}").into(),
        )
    })
}

/// Parse an optional Unix-timestamp column.
pub fn opt_timestamp_col(
    row: &rusqlite::Row,
    col: &str,
) -> rusqlite::Result<Option<DateTime<Utc>>> {
    let ts: Option<i64> = row.get(col)?;
    Ok(ts.and_then(|t| DateTime::from_timestamp(t, 0)))
}

/// Parse an RFC-3339 string column into `DateTime<Utc>`.
pub fn rfc3339_col(row: &rusqlite::Row, col: &str) -> rusqlite::Result<DateTime<Utc>> {
    let s: String = row.get(col)?;
    DateTime::parse_from_rfc3339(&s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })
}

/// Parse an optional RFC-3339 string column.
pub fn opt_rfc3339_col(row: &rusqlite::Row, col: &str) -> rusqlite::Result<Option<DateTime<Utc>>> {
    let s: Option<String> = row.get(col)?;
    Ok(s.and_then(|v| {
        DateTime::parse_from_rfc3339(&v)
            .ok()
            .map(|d| d.with_timezone(&Utc))
    }))
}

// ─── Session ─────────────────────────────────────────────────────────────────

/// Session model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: Uuid,
    pub title: Option<String>,
    pub model: Option<String>,
    pub provider_name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub token_count: i32,
    pub total_cost: f64,
    pub working_directory: Option<String>,
}

impl Session {
    pub fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(Session {
            id: uuid_col(row, "id")?,
            title: row.get("title")?,
            model: row.get("model")?,
            provider_name: row.get("provider_name")?,
            created_at: timestamp_col(row, "created_at")?,
            updated_at: timestamp_col(row, "updated_at")?,
            archived_at: opt_timestamp_col(row, "archived_at")?,
            token_count: row.get("token_count")?,
            total_cost: row.get("total_cost")?,
            working_directory: row.get("working_directory")?,
        })
    }

    /// Create a new session
    pub fn new(
        title: Option<String>,
        model: Option<String>,
        provider_name: Option<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            title,
            model,
            provider_name,
            created_at: now,
            updated_at: now,
            archived_at: None,
            token_count: 0,
            total_cost: 0.0,
            working_directory: None,
        }
    }

    /// Check if the session is archived
    pub fn is_archived(&self) -> bool {
        self.archived_at.is_some()
    }
}

// ─── Message ─────────────────────────────────────────────────────────────────

/// Message model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: Uuid,
    pub session_id: Uuid,
    pub role: String,
    pub content: String,
    pub sequence: i32,
    pub created_at: DateTime<Utc>,
    /// Output tokens (completion) reported by the provider. For assistant
    /// messages this is `usage.completion_tokens` / `output_tokens`.
    pub token_count: Option<i32>,
    pub cost: Option<f64>,
    /// Server-reported prompt token count for the request that produced
    /// this assistant message (`usage.prompt_tokens` / `input_tokens`).
    /// Populated only on assistant rows; always `None` for user rows.
    /// Used as the authoritative "last known context size" on session load
    /// — no more tokenizing raw message content to estimate.
    pub input_tokens: Option<i32>,
    /// Reasoning/thinking content for non-CLI providers (dialagram, custom
    /// OpenAI-compatible). Persisted separately from `content` so it
    /// survives restart and can be reconstructed as Ctrl+O expandable
    /// thinking blocks in the TUI. CLI providers store reasoning inline
    /// inside `content` as `<!-- reasoning -->` markers instead.
    pub thinking: Option<String>,
}

impl Message {
    pub fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(Message {
            id: uuid_col(row, "id")?,
            session_id: uuid_col(row, "session_id")?,
            role: row.get("role")?,
            content: row.get("content")?,
            sequence: row.get("sequence")?,
            created_at: timestamp_col(row, "created_at")?,
            token_count: row.get("token_count")?,
            cost: row.get("cost")?,
            input_tokens: row.get("input_tokens").ok(),
            thinking: row.get("thinking").ok().flatten(),
        })
    }

    /// Create a new message
    pub fn new(session_id: Uuid, role: String, content: String, sequence: i32) -> Self {
        Self {
            id: Uuid::new_v4(),
            session_id,
            role,
            content,
            sequence,
            created_at: Utc::now(),
            token_count: None,
            cost: None,
            input_tokens: None,
            thinking: None,
        }
    }
}

// ─── File ────────────────────────────────────────────────────────────────────

/// File model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct File {
    pub id: Uuid,
    pub session_id: Uuid,
    pub path: std::path::PathBuf,
    pub content: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl File {
    pub fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(File {
            id: uuid_col(row, "id")?,
            session_id: uuid_col(row, "session_id")?,
            path: std::path::PathBuf::from(row.get::<_, String>("path")?),
            content: row.get("content")?,
            created_at: timestamp_col(row, "created_at")?,
            updated_at: timestamp_col(row, "updated_at")?,
        })
    }

    /// Create a new file record
    pub fn new(session_id: Uuid, path: std::path::PathBuf, content: Option<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            session_id,
            path,
            content,
            created_at: now,
            updated_at: now,
        }
    }
}

// ─── Attachment ──────────────────────────────────────────────────────────────

/// Attachment model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub id: Uuid,
    pub message_id: Uuid,
    #[serde(rename = "type")]
    pub attachment_type: String,
    pub mime_type: Option<String>,
    pub path: Option<std::path::PathBuf>,
    pub size_bytes: Option<i64>,
    pub created_at: DateTime<Utc>,
}

impl Attachment {
    pub fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(Attachment {
            id: uuid_col(row, "id")?,
            message_id: uuid_col(row, "message_id")?,
            attachment_type: row.get("attachment_type")?,
            mime_type: row.get("mime_type")?,
            path: row
                .get::<_, Option<String>>("path")?
                .map(std::path::PathBuf::from),
            size_bytes: row.get("size_bytes")?,
            created_at: timestamp_col(row, "created_at")?,
        })
    }
}

// ─── ToolExecution ───────────────────────────────────────────────────────────

/// Tool execution model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolExecution {
    pub id: Uuid,
    pub message_id: Uuid,
    pub tool_name: String,
    /// JSON
    pub arguments: String,
    /// JSON
    pub result: Option<String>,
    pub status: String,
    pub approved_at: Option<DateTime<Utc>>,
    pub executed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl ToolExecution {
    pub fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(ToolExecution {
            id: uuid_col(row, "id")?,
            message_id: uuid_col(row, "message_id")?,
            tool_name: row.get("tool_name")?,
            arguments: row.get("arguments")?,
            result: row.get("result")?,
            status: row.get("status")?,
            approved_at: opt_timestamp_col(row, "approved_at")?,
            executed_at: opt_timestamp_col(row, "executed_at")?,
            created_at: timestamp_col(row, "created_at")?,
        })
    }
}

// ─── Plan ────────────────────────────────────────────────────────────────────

/// Plan model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub id: Uuid,
    pub session_id: Uuid,
    pub title: String,
    pub description: String,
    pub context: String,
    /// JSON array of strings
    pub risks: String,
    /// Testing strategy and approach
    pub test_strategy: String,
    /// JSON array of strings (technologies, frameworks, tools)
    pub technical_stack: String,
    /// Draft, PendingApproval, Approved, Rejected, InProgress, Completed, Cancelled
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub approved_at: Option<DateTime<Utc>>,
}

impl Plan {
    pub fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(Plan {
            id: uuid_col(row, "id")?,
            session_id: uuid_col(row, "session_id")?,
            title: row.get("title")?,
            description: row.get("description")?,
            context: row.get("context")?,
            risks: row.get("risks")?,
            test_strategy: row.get("test_strategy")?,
            technical_stack: row.get("technical_stack")?,
            status: row.get("status")?,
            created_at: timestamp_col(row, "created_at")?,
            updated_at: timestamp_col(row, "updated_at")?,
            approved_at: opt_timestamp_col(row, "approved_at")?,
        })
    }
}

// ─── PlanTask ────────────────────────────────────────────────────────────────

/// Plan task model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanTask {
    pub id: Uuid,
    pub plan_id: Uuid,
    pub task_order: i32,
    pub title: String,
    pub description: String,
    /// Research, Edit, Create, Delete, Test, Refactor, Documentation, Configuration, Build, Other
    pub task_type: String,
    /// JSON array of task IDs
    pub dependencies: String,
    /// 1-5 scale
    pub complexity: i32,
    /// JSON array of strings (task completion criteria)
    pub acceptance_criteria: String,
    /// Pending, InProgress, Completed, Skipped, Failed, Blocked
    pub status: String,
    pub notes: Option<String>,
    pub completed_at: Option<DateTime<Utc>>,
}

impl PlanTask {
    pub fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(PlanTask {
            id: uuid_col(row, "id")?,
            plan_id: uuid_col(row, "plan_id")?,
            task_order: row.get("task_order")?,
            title: row.get("title")?,
            description: row.get("description")?,
            task_type: row.get("task_type")?,
            dependencies: row.get("dependencies")?,
            complexity: row.get("complexity")?,
            acceptance_criteria: row.get("acceptance_criteria")?,
            status: row.get("status")?,
            notes: row.get("notes")?,
            completed_at: opt_timestamp_col(row, "completed_at")?,
        })
    }
}

// ─── ChannelMessage ──────────────────────────────────────────────────────────

/// Channel message model — passive capture of messages from channel platforms
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelMessage {
    pub id: Uuid,
    pub channel: String,
    pub channel_chat_id: String,
    pub channel_chat_name: Option<String>,
    pub sender_id: String,
    pub sender_name: String,
    pub content: String,
    pub message_type: String,
    pub platform_message_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl ChannelMessage {
    pub fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(ChannelMessage {
            id: uuid_col(row, "id")?,
            channel: row.get("channel")?,
            channel_chat_id: row.get("channel_chat_id")?,
            channel_chat_name: row.get("channel_chat_name")?,
            sender_id: row.get("sender_id")?,
            sender_name: row.get("sender_name")?,
            content: row.get("content")?,
            message_type: row.get("message_type")?,
            platform_message_id: row.get("platform_message_id")?,
            created_at: timestamp_col(row, "created_at")?,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        channel: String,
        channel_chat_id: String,
        channel_chat_name: Option<String>,
        sender_id: String,
        sender_name: String,
        content: String,
        message_type: String,
        platform_message_id: Option<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            channel,
            channel_chat_id,
            channel_chat_name,
            sender_id,
            sender_name,
            content,
            message_type,
            platform_message_id,
            created_at: Utc::now(),
        }
    }
}

// ─── CronJob ─────────────────────────────────────────────────────────────────

/// Cron job model — a scheduled isolated session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    pub id: Uuid,
    pub name: String,
    pub cron_expr: String,
    pub timezone: String,
    pub prompt: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub thinking: String,
    pub auto_approve: bool,
    pub deliver_to: Option<String>,
    pub enabled: bool,
    pub last_run_at: Option<DateTime<Utc>>,
    pub next_run_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl CronJob {
    pub fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(CronJob {
            id: uuid_col(row, "id")?,
            name: row.get("name")?,
            cron_expr: row.get("cron_expr")?,
            timezone: row.get("timezone")?,
            prompt: row.get("prompt")?,
            provider: row.get("provider")?,
            model: row.get("model")?,
            thinking: row.get("thinking")?,
            auto_approve: row.get::<_, i32>("auto_approve")? != 0,
            deliver_to: row.get("deliver_to")?,
            enabled: row.get::<_, i32>("enabled")? != 0,
            last_run_at: opt_rfc3339_col(row, "last_run_at")?,
            next_run_at: opt_rfc3339_col(row, "next_run_at")?,
            created_at: rfc3339_col(row, "created_at")?,
            updated_at: rfc3339_col(row, "updated_at")?,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: String,
        cron_expr: String,
        timezone: String,
        prompt: String,
        provider: Option<String>,
        model: Option<String>,
        thinking: String,
        auto_approve: bool,
        deliver_to: Option<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name,
            cron_expr,
            timezone,
            prompt,
            provider,
            model,
            thinking,
            auto_approve,
            deliver_to,
            enabled: true,
            last_run_at: None,
            next_run_at: None,
            created_at: now,
            updated_at: now,
        }
    }
}

// ─── CronJobRun ──────────────────────────────────────────────────────────────

/// A single execution record for a cron job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJobRun {
    pub id: Uuid,
    pub job_id: Uuid,
    pub job_name: String,
    pub status: String, // "running", "success", "error"
    pub content: Option<String>,
    pub error: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost: f64,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl CronJobRun {
    pub fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(CronJobRun {
            id: uuid_col(row, "id")?,
            job_id: uuid_col(row, "job_id")?,
            job_name: row.get("job_name")?,
            status: row.get("status")?,
            content: row.get("content")?,
            error: row.get("error")?,
            input_tokens: row.get("input_tokens")?,
            output_tokens: row.get("output_tokens")?,
            cost: row.get("cost")?,
            provider: row.get("provider")?,
            model: row.get("model")?,
            started_at: rfc3339_col(row, "started_at")?,
            completed_at: opt_rfc3339_col(row, "completed_at")?,
            created_at: rfc3339_col(row, "created_at")?,
        })
    }

    pub fn new_running(
        job_id: Uuid,
        job_name: String,
        provider: Option<String>,
        model: Option<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            job_id,
            job_name,
            status: "running".to_string(),
            content: None,
            error: None,
            input_tokens: 0,
            output_tokens: 0,
            cost: 0.0,
            provider,
            model,
            started_at: now,
            completed_at: None,
            created_at: now,
        }
    }
}

// ─── FeedbackEntry ──────────────────────────────────────────────────────────

/// Feedback ledger entry — append-only observations for recursive self-improvement.
///
/// Records tool outcomes, user corrections, provider errors, and performance
/// signals. Consumed by the feedback_analyze and self_improve tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackEntry {
    pub id: i64,
    pub session_id: String,
    pub event_type: String,
    pub dimension: String,
    pub value: f64,
    pub metadata: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl FeedbackEntry {
    pub fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(FeedbackEntry {
            id: row.get("id")?,
            session_id: row.get("session_id")?,
            event_type: row.get("event_type")?,
            dimension: row.get("dimension")?,
            value: row.get("value")?,
            metadata: row.get("metadata")?,
            created_at: rfc3339_col(row, "created_at")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_new() {
        let session = Session::new(
            Some("Test Session".to_string()),
            Some("claude-sonnet-4-5".to_string()),
            Some("anthropic".to_string()),
        );

        assert_eq!(session.title, Some("Test Session".to_string()));
        assert_eq!(session.model, Some("claude-sonnet-4-5".to_string()));
        assert_eq!(session.token_count, 0);
        assert!(!session.is_archived());
    }

    #[test]
    fn test_message_new() {
        let session_id = Uuid::new_v4();
        let message = Message::new(session_id, "user".to_string(), "Hello!".to_string(), 1);

        assert_eq!(message.session_id, session_id);
        assert_eq!(message.role, "user");
        assert_eq!(message.content, "Hello!");
        assert_eq!(message.sequence, 1);
        assert!(message.token_count.is_none());
    }

    #[test]
    fn test_file_new() {
        let session_id = Uuid::new_v4();
        let path = std::path::PathBuf::from("/path/to/file.rs");
        let file = File::new(session_id, path.clone(), None);

        assert_eq!(file.session_id, session_id);
        assert_eq!(file.path, path);
        assert!(file.content.is_none());
    }

    #[test]
    fn test_feedback_entry_from_row_fields() {
        // Covered by integration tests in feedback_ledger_test.rs;
        // this verifies struct construction.
        let entry = FeedbackEntry {
            id: 1,
            session_id: "abc".to_string(),
            event_type: "tool_success".to_string(),
            dimension: "bash".to_string(),
            value: 1.0,
            metadata: None,
            created_at: Utc::now(),
        };
        assert_eq!(entry.event_type, "tool_success");
        assert_eq!(entry.dimension, "bash");
        assert!((entry.value - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_session_archived() {
        let mut session = Session::new(Some("Test".to_string()), Some("model".to_string()), None);

        assert!(!session.is_archived());

        session.archived_at = Some(Utc::now());
        assert!(session.is_archived());
    }
}
