//! Shared types for the Mission Control data layer.
//!
//! Each panel renders a uniform list of items. The type wrapping each
//! item carries enough metadata for the renderer to badge / colour
//! consistently across the three sources (inbox / activity / schedule)
//! without leaking the underlying storage shape.

use chrono::{DateTime, Utc};

/// Optional rich detail for an inbox item, used by the detail popup
/// to render type-specific content beyond the generic summary line.
#[derive(Debug, Clone)]
pub enum McInboxDetail {}

/// One actionable item in the inbox panel — typically an RSI proposal.
#[derive(Debug, Clone)]
pub struct McInboxItem {
    /// Stable id for action-by-id flows (apply/reject). For RSI proposals
    /// this is `prop_tool_<uuid>` or `prop_cmd_<uuid>` from the inbox file.
    pub id: String,
    /// Short human label — slug name, e.g. "deploy_staging" or "/release".
    pub label: String,
    /// One-line summary surfaced under the label. The agent's rationale
    /// for why it proposed this, or a tool's command preview.
    pub summary: String,
    /// What kind of artifact this represents — drives the badge colour.
    pub kind: McInboxKind,
    /// Origin of the proposal (the `proposed_by` field on the inbox row,
    /// e.g. "rsi-autonomous"). Used for the "proposed by …" caption.
    pub source: String,
    /// When this item entered the inbox.
    pub created_at: DateTime<Utc>,
    /// Optional rich detail for the popup — type-specific content
    /// beyond the one-line summary (e.g. dedup text, rationale).
    pub detail: Option<McInboxDetail>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McInboxKind {
    /// RSI-proposed dynamic tool (lands in `tools.toml` on apply).
    ProposedTool,
    /// RSI-proposed slash command (lands in `commands.toml` on apply).
    ProposedCommand,
    /// RSI-proposed skill (lands at `~/.opencrabs/skills/<name>/SKILL.md`
    /// on apply, with YAML frontmatter wrapping the proposed body).
    ProposedSkill,
}

impl McInboxKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::ProposedTool => "tool",
            Self::ProposedCommand => "command",
            Self::ProposedSkill => "skill",
        }
    }
}

/// One entry in the activity feed — RSI-emitted events worth surfacing.
#[derive(Debug, Clone)]
pub struct McActivity {
    /// `None` when the journal entry carried no parseable date. Optional
    /// rather than defaulted because the old `Utc::now()` fallback rendered
    /// weeks-old entries as "10s ago" (#841): an unknown time must read as
    /// unknown, never as this instant.
    pub timestamp: Option<DateTime<Utc>>,
    /// One-line summary, already truncated to a reasonable display length
    /// by the service layer.
    pub detail: String,
    /// Severity hint for colour selection in the renderer.
    pub level: McActivityLevel,
    /// Origin tag — "rsi", "compaction", "template-sync", etc. Stored as
    /// a free string so adding a new source doesn't require a migration.
    pub source: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McActivityLevel {
    Info,
    Success,
    Warn,
    Error,
}

/// One scheduled / pending-action row.
#[derive(Debug, Clone)]
pub struct McScheduleItem {
    pub id: String,
    pub label: String,
    /// Free-text describing when/how it triggers — "0 9 * * *", "pending
    /// approval", "next at 14:00", etc.
    pub schedule: String,
    pub kind: McScheduleKind,
    /// `true` when the item is actively waiting on the user (e.g. a
    /// pending tool approval or a paused cron). Renders highlighted.
    pub awaiting_user: bool,
    /// The agent prompt / description — what this job actually does.
    pub prompt: String,
    /// Where results are delivered (e.g. "telegram:7711740248").
    pub deliver_to: Option<String>,
    /// When this job last ran.
    pub last_run_at: Option<DateTime<Utc>>,
    /// When this job is scheduled to run next.
    pub next_run_at: Option<DateTime<Utc>>,
    /// When this job was created.
    pub created_at: DateTime<Utc>,
    /// Whether the job is enabled (not paused).
    pub enabled: bool,
    /// Profile this job belongs to.
    pub profile_name: Option<String>,
    /// Status of the last run ("success", "error", "running").
    pub last_run_status: Option<String>,
    /// Cost of the last run in USD.
    pub last_run_cost: Option<f64>,
    /// Duration of the last run in seconds.
    pub last_run_duration_secs: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McScheduleKind {
    /// Recurring cron job from `~/.opencrabs/cron/*.toml`.
    Cron,
    /// One-shot agent action waiting on a user approval prompt.
    PendingApproval,
}

impl McScheduleKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Cron => "cron",
            Self::PendingApproval => "approval",
        }
    }
}

/// One tool's usage and reliability, for the analytics panel.
#[derive(Debug, Clone)]
pub struct McToolStat {
    pub name: String,
    pub total: i64,
    pub failures: i64,
    /// Failures as a percentage of total, rounded to one decimal.
    pub fail_rate: f64,
}

/// One brain file's on-disk size, for the analytics panel.
#[derive(Debug, Clone)]
pub struct McBrainFile {
    pub name: String,
    pub kb: f64,
}

/// Time window for analytics queries. The TUI's D/W/M filter tabs and the
/// report tool convert this to an epoch-seconds bound for the DB queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeWindow {
    Day,
    Week,
    Month,
    /// All-time (no lower bound).
    #[default]
    All,
}

impl TimeWindow {
    /// Epoch seconds bounding the window's start; `None` = all-time.
    pub fn since_epoch(self) -> Option<i64> {
        let now = chrono::Utc::now().timestamp();
        match self {
            TimeWindow::Day => Some(now - 86_400),
            TimeWindow::Week => Some(now - 7 * 86_400),
            TimeWindow::Month => Some(now - 30 * 86_400),
            TimeWindow::All => None,
        }
    }
}

/// Phantom tool-call detection stats for the analytics panel.
#[derive(Debug, Clone, Default)]
pub struct McPhantomStats {
    pub total: i64,
    pub resolved: i64,
    /// Resolved as a percentage of total, rounded to one decimal.
    pub resolved_pct: f64,
    /// (model, total, resolved), most phantoms first.
    pub by_model: Vec<(String, i64, i64)>,
}

/// Streaming-recovery stats for the analytics panel.
#[derive(Debug, Clone, Default)]
pub struct McStreamingStats {
    pub total: i64,
    pub total_tools: i64,
    /// (model, recovery count), most recoveries first.
    pub by_model: Vec<(String, i64)>,
}

/// Brain-file verification gate stats for the analytics panel.
#[derive(Debug, Clone, Default)]
pub struct McBrainVerifyStats {
    pub passes: i64,
    pub rollbacks: i64,
    pub fail_closed: i64,
}

/// One model's tool-execution reliability, for the per-model breakdown.
#[derive(Debug, Clone, Default)]
pub struct McModelToolStat {
    pub model: String,
    pub total: i64,
    pub failures: i64,
    /// Failures as a percentage of total, rounded to one decimal.
    pub fail_rate: f64,
}

/// Snapshot for the Mission Control analytics panel. Built from data
/// OpenCrabs already owns: the `tool_executions` and `feedback_ledger`
/// tables and the active profile's brain `.md` files. No secrets, no message
/// content, nothing leaves the machine.
#[derive(Debug, Clone, Default)]
pub struct McAnalytics {
    pub tool_total_calls: i64,
    pub tool_total_fails: i64,
    /// Most-used tools first.
    pub top_tools: Vec<McToolStat>,
    /// Highest failure rate first (only tools with enough calls to matter).
    pub flakiest_tools: Vec<McToolStat>,
    pub rsi_applied_total: i64,
    /// Unix ts of the last RSI activity (self_improve / feedback_analyze /
    /// rsi_propose execution); None = never ran (#469).
    pub rsi_last_call_ts: Option<i64>,
    /// Tool events recorded AFTER the last RSI activity (#469).
    pub tool_events_since_rsi: i64,
    /// RSI `improvement_applied` counts per dimension, largest first.
    pub rsi_top_dimensions: Vec<(String, i64)>,
    /// Brain files, largest first.
    pub brain_files: Vec<McBrainFile>,
    pub brain_total_kb: f64,
    /// Phantom tool-call detection stats (all-time).
    pub phantom: McPhantomStats,
    /// Streaming-recovery stats (all-time).
    pub streaming: McStreamingStats,
    /// Brain-file verification gate stats (all-time).
    pub brain_verify: McBrainVerifyStats,
    /// Per-model tool reliability, most calls first.
    pub model_tools: Vec<McModelToolStat>,
    /// Decision-cache tier counters (#1648 PR3), most calls first. Empty
    /// when the feature has never been observed; empty means "no block".
    pub decisions: Vec<McDecisionStat>,
}

/// One decision-reuse tier's counters (#1648), read straight from
/// `decision_stats`. `would_hit` is what shadow mode measured but did not
/// act on; `live_hit` is reuse actually taken. Release-day keep-or-cut is
/// judged from these.
#[derive(Debug, Clone, Default)]
pub struct McDecisionStat {
    pub tier_id: String,
    pub calls: i64,
    pub would_hit: i64,
    pub live_hit: i64,
}
