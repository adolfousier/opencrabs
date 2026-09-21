//! Mission Control Report Tool
//!
//! Renders the full Mission Control snapshot (analytics, activity feed,
//! inbox proposals, schedule) as a Markdown report the agent can send
//! through whatever channel it is on. Same data as the TUI Mission
//! Control view, exposed as a shareable report. No secrets, no message
//! content, nothing leaves the machine unless the agent chooses to send
//! the report.

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use crate::brain::mission_control::types::{McActivity, McAnalytics, McInboxItem, McScheduleItem};
use crate::brain::mission_control::{
    TimeWindow, activity_service, analytics_service, inbox_service, schedule_service,
};
use crate::db::Pool;
use async_trait::async_trait;
use serde_json::Value;

/// Generates a shareable mission control report from local stats.
pub struct MissionControlReportTool {
    pool: Pool,
}

impl MissionControlReportTool {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl Tool for MissionControlReportTool {
    fn name(&self) -> &str {
        "mission_control_report"
    }

    fn description(&self) -> &str {
        "Generate a shareable mission control report of this OpenCrabs instance: analytics \
         (tool usage, failure rates, RSI improvements, brain files), activity feed, inbox \
         proposals, and scheduled cron jobs. Returns Markdown you can send straight to a chat. \
         Reads only aggregate stats from the local database, brain file sizes, RSI proposals, \
         and cron jobs. Never exposes message content or secrets."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::ReadFiles]
    }

    fn requires_approval(&self) -> bool {
        false
    }

    async fn execute(&self, _input: Value, _context: &ToolExecutionContext) -> Result<ToolResult> {
        let analytics = analytics_service::summary(self.pool.clone(), TimeWindow::All).await;
        let activity = activity_service::recent(5);
        let inbox = inbox_service::list();
        let schedule = schedule_service::list(self.pool.clone()).await;
        Ok(ToolResult::success(render_markdown(
            &analytics, &activity, &inbox, &schedule,
        )))
    }
}

/// Level icon for activity feed entries.
fn activity_icon(level: &crate::brain::mission_control::types::McActivityLevel) -> &'static str {
    use crate::brain::mission_control::types::McActivityLevel;
    match level {
        McActivityLevel::Success => "✅",
        McActivityLevel::Warn => "⚠️",
        McActivityLevel::Error => "❌",
        McActivityLevel::Info => "ℹ️",
    }
}

/// Icon for inbox item kinds.
fn inbox_icon(kind: &crate::brain::mission_control::types::McInboxKind) -> &'static str {
    use crate::brain::mission_control::types::McInboxKind;
    match kind {
        McInboxKind::ProposedTool => "🔧",
        McInboxKind::ProposedCommand => "📋",
        McInboxKind::ProposedSkill => "🧠",
    }
}

/// Icon for schedule item kinds.
fn schedule_icon(item: &McScheduleItem) -> &'static str {
    if item.awaiting_user { "⏸️" } else { "⏰" }
}

/// Render a full Mission Control snapshot as channel-friendly Markdown.
/// Pure, so it is unit-testable without a database.
pub(crate) fn render_markdown(
    a: &McAnalytics,
    activity: &[McActivity],
    inbox: &[McInboxItem],
    schedule: &[McScheduleItem],
) -> String {
    use crate::utils::string::md_table;

    let fail_pct = if a.tool_total_calls > 0 {
        (a.tool_total_fails as f64 / a.tool_total_calls as f64) * 100.0
    } else {
        0.0
    };

    // Markdown blocks (heading / table) joined with blank lines. Authored as
    // ATX headings + GFM tables so Telegram's native rich renderer turns them
    // into real bordered tables (not `<pre>` ASCII grids); the HTML fallback
    // still renders them readably.
    let mut blocks: Vec<String> = vec!["# 🦀 Mission Control".to_string()];

    // ── Analytics ────────────────────────────────────────────────────
    blocks.push("## 📊 Analytics".to_string());
    blocks.push(
        md_table(
            &["Metric", "Value"],
            &[
                vec![
                    "Tools".to_string(),
                    format!(
                        "{} calls, {} fails ({:.1}%)",
                        a.tool_total_calls, a.tool_total_fails, fail_pct
                    ),
                ],
                vec!["RSI applied".to_string(), a.rsi_applied_total.to_string()],
                vec![
                    "RSI liveness".to_string(),
                    crate::brain::mission_control::staleness::rsi_staleness_line(
                        chrono::Utc::now().timestamp(),
                        a.rsi_last_call_ts,
                        a.tool_events_since_rsi,
                    ),
                ],
                vec![
                    "Brain".to_string(),
                    format!(
                        "{:.1} KB across {} files",
                        a.brain_total_kb,
                        a.brain_files.len()
                    ),
                ],
                vec![
                    "Phantoms".to_string(),
                    format!(
                        "{} detected, {} resolved ({:.1}%)",
                        a.phantom.total, a.phantom.resolved, a.phantom.resolved_pct
                    ),
                ],
                vec![
                    "Stream recoveries".to_string(),
                    format!("{} ({} tools)", a.streaming.total, a.streaming.total_tools),
                ],
                vec![
                    "Brain verify".to_string(),
                    format!(
                        "{} pass, {} rollback, {} fail-closed",
                        a.brain_verify.passes, a.brain_verify.rollbacks, a.brain_verify.fail_closed
                    ),
                ],
            ],
        )
        .trim_end()
        .to_string(),
    );

    let mut table_section = |title: &str, headers: &[&str], rows: Vec<Vec<String>>| {
        if !rows.is_empty() {
            blocks.push(format!("### {title}"));
            blocks.push(md_table(headers, &rows).trim_end().to_string());
        }
    };

    // Decision-reuse counters (#1648 PR3). Silent when nothing has been
    // measured: an unused feature must not clutter the report, and an
    // empty section on release day is itself the cut signal.
    table_section(
        "Decision cache",
        &["Tier", "Calls", "Would-hit", "Live-hit"],
        a.decisions
            .iter()
            .map(|d| {
                vec![
                    format!("`{}`", d.tier_id),
                    d.calls.to_string(),
                    d.would_hit.to_string(),
                    d.live_hit.to_string(),
                ]
            })
            .collect(),
    );

    table_section(
        "Top Tools",
        &["Tool", "Calls", "Fail"],
        a.top_tools
            .iter()
            .take(10)
            .map(|t| {
                vec![
                    t.name.clone(),
                    format!("{} calls", t.total),
                    format!("{:.1}% fail", t.fail_rate),
                ]
            })
            .collect(),
    );
    table_section(
        "Flakiest (>=5 calls)",
        &["Tool", "Fail", "Calls"],
        a.flakiest_tools
            .iter()
            .take(8)
            .map(|t| {
                vec![
                    t.name.clone(),
                    format!("{:.1}% fail", t.fail_rate),
                    format!("{} calls", t.total),
                ]
            })
            .collect(),
    );
    table_section(
        "RSI by Dimension",
        &["Dimension", "Count"],
        a.rsi_top_dimensions
            .iter()
            .take(8)
            .map(|(dim, n)| vec![dim.clone(), n.to_string()])
            .collect(),
    );
    table_section(
        "Brain Files",
        &["File", "Size"],
        a.brain_files
            .iter()
            .take(10)
            .map(|f| vec![f.name.clone(), format!("{:.1} KB", f.kb)])
            .collect(),
    );
    table_section(
        "Phantom by Model",
        &["Model", "Total", "Resolved"],
        a.phantom
            .by_model
            .iter()
            .take(8)
            .map(|(model, total, resolved)| {
                vec![model.clone(), total.to_string(), resolved.to_string()]
            })
            .collect(),
    );
    table_section(
        "Tool Reliability by Model",
        &["Model", "Calls", "Fail"],
        a.model_tools
            .iter()
            .take(8)
            .map(|m| {
                vec![
                    m.model.clone(),
                    format!("{} calls", m.total),
                    format!("{:.1}% fail", m.fail_rate),
                ]
            })
            .collect(),
    );

    // ── Inbox (RSI proposals) ────────────────────────────────────────
    if !inbox.is_empty() {
        let rows: Vec<Vec<String>> = inbox
            .iter()
            .take(5)
            .map(|item| {
                vec![
                    format!("{} {}", inbox_icon(&item.kind), item.label),
                    item.summary.clone(),
                ]
            })
            .collect();
        blocks.push("## 📥 Inbox (RSI Proposals)".to_string());
        blocks.push(
            md_table(&["Proposal", "Detail"], &rows)
                .trim_end()
                .to_string(),
        );
    }

    // ── Activity feed ────────────────────────────────────────────────
    if !activity.is_empty() {
        let rows: Vec<Vec<String>> = activity
            .iter()
            .take(5)
            .map(|entry| {
                vec![
                    format!(
                        "{} {}",
                        activity_icon(&entry.level),
                        entry.timestamp.map_or_else(
                            || "undated".to_string(),
                            |ts| ts.format("%Y-%m-%d").to_string()
                        )
                    ),
                    entry.detail.clone(),
                ]
            })
            .collect();
        blocks.push("## 📰 Activity Feed".to_string());
        blocks.push(md_table(&["When", "Detail"], &rows).trim_end().to_string());
    }

    // ── Schedule (cron jobs) ─────────────────────────────────────────
    if !schedule.is_empty() {
        let rows: Vec<Vec<String>> = schedule
            .iter()
            .map(|item| {
                vec![
                    format!("{} {}", schedule_icon(item), item.label),
                    item.schedule.clone(),
                ]
            })
            .collect();
        blocks.push("## ⏰ Schedule".to_string());
        blocks.push(md_table(&["Job", "Schedule"], &rows).trim_end().to_string());
    }

    blocks.join("\n\n")
}
