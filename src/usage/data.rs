//! Usage dashboard data layer
//!
//! All data structs, DB aggregation queries, activity classifier, and period enum.
//! Queries hit the DB pool directly — no dependency on repository structs.

use crate::db::{Pool, interact_err};
use anyhow::{Context, Result};
use rusqlite::params;

// ── Period filter ────────────────────────────────────────────────────────────

/// Time period filter for dashboard cards
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Today,
    Week,
    Month,
    AllTime,
}

impl Period {
    /// Cycle to the next period (wraps around)
    pub fn next(self) -> Self {
        match self {
            Self::Today => Self::Week,
            Self::Week => Self::Month,
            Self::Month => Self::AllTime,
            Self::AllTime => Self::Today,
        }
    }

    /// Returns the epoch timestamp for the start of this period, or None for AllTime
    pub fn since_epoch(self) -> Option<i64> {
        let now = chrono::Utc::now().timestamp();
        match self {
            Self::Today => Some(now - 86_400),
            Self::Week => Some(now - 7 * 86_400),
            Self::Month => Some(now - 30 * 86_400),
            Self::AllTime => None,
        }
    }

    /// Short label for the status bar
    pub fn label(self) -> &'static str {
        match self {
            Self::Today => "Today",
            Self::Week => "Week",
            Self::Month => "Month",
            Self::AllTime => "All Time",
        }
    }
}

// ── Data structs ─────────────────────────────────────────────────────────────

/// Summary bar: total tokens, total cost, session count
#[derive(Debug, Clone, Default)]
pub struct SummaryStats {
    pub total_tokens: i64,
    pub total_cost: f64,
    pub session_count: i64,
    pub call_count: i64,
}

/// Daily usage for the sparkline / bar chart
#[derive(Debug, Clone)]
pub struct DailyStats {
    pub date: String,
    pub tokens: i64,
    pub cost: f64,
    pub calls: i64,
}

/// Per-project (working_directory) usage
#[derive(Debug, Clone)]
pub struct ProjectStats {
    pub project: String,
    pub cost: f64,
    pub tokens: i64,
    pub sessions: i64,
}

/// Per-model usage
#[derive(Debug, Clone)]
pub struct ModelStats {
    pub model: String,
    pub tokens: i64,
    pub cost: f64,
    pub calls: i64,
    pub estimated: bool,
}

/// Tool usage count
#[derive(Debug, Clone)]
pub struct ToolStats {
    pub tool_name: String,
    pub call_count: i64,
}

/// Activity category usage
#[derive(Debug, Clone)]
pub struct ActivityStats {
    pub category: String,
    pub cost: f64,
    pub turns: i64,
    pub one_shot_pct: f64,
}

/// All dashboard data, fetched once per period change
#[derive(Debug, Clone, Default)]
pub struct DashboardData {
    pub summary: SummaryStats,
    pub daily: Vec<DailyStats>,
    pub projects: Vec<ProjectStats>,
    pub models: Vec<ModelStats>,
    pub tools: Vec<ToolStats>,
    pub activities: Vec<ActivityStats>,
}

// ── Activity classifier ──────────────────────────────────────────────────────

/// Classify a session title into an activity category using keyword heuristics.
pub fn classify_activity(title: &str) -> &'static str {
    let t = title.to_lowercase();

    if t.contains("ci") || t.contains("deploy") || t.contains("release") || t.contains("workflow") {
        return "CI/Deploy";
    }
    if t.contains("bug") || t.contains("fix") || t.contains("error") || t.contains("crash") {
        return "Bug Fixes";
    }
    if t.contains("refactor") || t.contains("cleanup") || t.contains("clean up") {
        return "Refactoring";
    }
    if t.contains("test") || t.contains("spec") || t.contains("coverage") {
        return "Testing";
    }
    if t.contains("doc") || t.contains("readme") || t.contains("changelog") {
        return "Documentation";
    }
    if t.contains("feat") || t.contains("add") || t.contains("new") || t.contains("implement") {
        return "Features";
    }
    if t.contains("config") || t.contains("setup") || t.contains("setting") {
        return "Config";
    }
    "Development"
}

// ── Data fetching ────────────────────────────────────────────────────────────

impl DashboardData {
    /// Fetch all dashboard data for a given period.
    pub async fn fetch(pool: &Pool, period: Period) -> Result<Self> {
        let since = period.since_epoch();

        // Run all queries concurrently
        let (summary, daily, projects, models, tools, activities) = tokio::try_join!(
            fetch_summary(pool, since),
            fetch_daily(pool, since),
            fetch_projects(pool, since),
            fetch_models(pool, since),
            fetch_tools(pool, since),
            fetch_activities(pool, since),
        )?;

        Ok(Self {
            summary,
            daily,
            projects,
            models,
            tools,
            activities,
        })
    }
}

async fn fetch_summary(pool: &Pool, since: Option<i64>) -> Result<SummaryStats> {
    let conn = pool.get().await.context("pool")?;
    conn.interact(move |conn| {
        let (query, param): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(s) = since {
            (
                "SELECT COALESCE(SUM(token_count), 0), COALESCE(SUM(cost), 0.0), \
                 COUNT(DISTINCT session_id), COUNT(*) \
                 FROM usage_ledger WHERE created_at >= ?1",
                vec![Box::new(s)],
            )
        } else {
            (
                "SELECT COALESCE(SUM(token_count), 0), COALESCE(SUM(cost), 0.0), \
                 COUNT(DISTINCT session_id), COUNT(*) \
                 FROM usage_ledger",
                vec![],
            )
        };
        let refs: Vec<&dyn rusqlite::types::ToSql> = param.iter().map(|p| p.as_ref()).collect();
        conn.query_row(query, refs.as_slice(), |row| {
            Ok(SummaryStats {
                total_tokens: row.get(0)?,
                total_cost: row.get(1)?,
                session_count: row.get(2)?,
                call_count: row.get(3)?,
            })
        })
    })
    .await
    .map_err(interact_err)?
    .context("Failed to fetch summary stats")
}

async fn fetch_daily(pool: &Pool, since: Option<i64>) -> Result<Vec<DailyStats>> {
    let conn = pool.get().await.context("pool")?;
    conn.interact(move |conn| {
        let (query, param): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(s) = since {
            (
                "SELECT date(created_at, 'unixepoch') AS day, \
                 COALESCE(SUM(token_count), 0), COALESCE(SUM(cost), 0.0), COUNT(*) \
                 FROM usage_ledger WHERE created_at >= ?1 \
                 GROUP BY day ORDER BY day ASC",
                vec![Box::new(s)],
            )
        } else {
            (
                "SELECT date(created_at, 'unixepoch') AS day, \
                 COALESCE(SUM(token_count), 0), COALESCE(SUM(cost), 0.0), COUNT(*) \
                 FROM usage_ledger GROUP BY day ORDER BY day ASC",
                vec![],
            )
        };
        let refs: Vec<&dyn rusqlite::types::ToSql> = param.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(query)?;
        let rows = stmt.query_map(refs.as_slice(), |row| {
            Ok(DailyStats {
                date: row.get(0)?,
                tokens: row.get(1)?,
                cost: row.get(2)?,
                calls: row.get(3)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
    })
    .await
    .map_err(interact_err)?
    .context("Failed to fetch daily stats")
}

/// Map raw working_directory to a display-friendly project name.
/// User home directory (no project) typically means brain-file editing.
fn normalize_project_name(raw: &str) -> String {
    if raw == "unknown" || raw.is_empty() {
        return "unknown".to_string();
    }
    let home = std::env::var("HOME").unwrap_or_default();
    if !home.is_empty() && raw.trim_end_matches('/') == home.trim_end_matches('/') {
        return "brain-files".to_string();
    }
    raw.rsplit('/').next().unwrap_or(raw).to_string()
}

/// Merge rows that map to the same display name by summing stats.
fn merge_project_stats(stats: &mut Vec<ProjectStats>) {
    let mut map = std::collections::HashMap::<String, ProjectStats>::new();
    for s in stats.drain(..) {
        map.entry(s.project.clone())
            .and_modify(|e| {
                e.cost += s.cost;
                e.tokens += s.tokens;
                e.sessions += s.sessions;
            })
            .or_insert(s);
    }
    *stats = map.into_values().collect();
    stats.sort_by(|a, b| {
        b.cost
            .partial_cmp(&a.cost)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

async fn fetch_projects(pool: &Pool, since: Option<i64>) -> Result<Vec<ProjectStats>> {
    let conn = pool.get().await.context("pool")?;
    conn.interact(move |conn| -> rusqlite::Result<Vec<ProjectStats>> {
        let (query, param): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(s) = since {
            (
                "SELECT COALESCE(s.working_directory, 'unknown'), \
                 COALESCE(SUM(u.cost), 0.0), COALESCE(SUM(u.token_count), 0), \
                 COUNT(DISTINCT u.session_id) \
                 FROM usage_ledger u \
                 LEFT JOIN sessions s ON u.session_id = s.id \
                 WHERE u.created_at >= ?1 \
                 GROUP BY s.working_directory \
                 ORDER BY SUM(u.cost) DESC",
                vec![Box::new(s)],
            )
        } else {
            (
                "SELECT COALESCE(s.working_directory, 'unknown'), \
                 COALESCE(SUM(u.cost), 0.0), COALESCE(SUM(u.token_count), 0), \
                 COUNT(DISTINCT u.session_id) \
                 FROM usage_ledger u \
                 LEFT JOIN sessions s ON u.session_id = s.id \
                 GROUP BY s.working_directory \
                 ORDER BY SUM(u.cost) DESC",
                vec![],
            )
        };
        let refs: Vec<&dyn rusqlite::types::ToSql> = param.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(query)?;
        let rows = stmt.query_map(refs.as_slice(), |row| {
            let raw: String = row.get(0)?;
            let project = normalize_project_name(&raw);
            Ok(ProjectStats {
                project,
                cost: row.get(1)?,
                tokens: row.get(2)?,
                sessions: row.get(3)?,
            })
        })?;
        let mut stats: Vec<ProjectStats> = rows.collect::<std::result::Result<Vec<_>, _>>()?;
        merge_project_stats(&mut stats);
        Ok(stats)
    })
    .await
    .map_err(interact_err)?
    .context("Failed to fetch project stats")
}

async fn fetch_models(pool: &Pool, since: Option<i64>) -> Result<Vec<ModelStats>> {
    // Load current pricing to recalculate costs at display time.
    // Stored costs may be stale (old pricing or $0.00 for unknown models at record time).
    let pricing = crate::usage::pricing::PricingConfig::load().ok();

    let conn = pool.get().await.context("pool")?;
    let models = conn.interact(move |conn| {
        // Reuse the same SQL normalization as usage_ledger.rs stats_by_model
        let base_where = if since.is_some() {
            "WHERE model != '' AND created_at >= ?1"
        } else {
            "WHERE model != ''"
        };
        let query = format!(
            "WITH stripped AS ( \
               SELECT *, \
                 LOWER(CASE WHEN model LIKE '%/%' \
                   THEN SUBSTR(model, INSTR(model, '/') + 1) \
                   ELSE model \
                 END) AS m1 \
               FROM usage_ledger {base_where} \
             ), \
             cleaned AS ( \
               SELECT *, \
                 CASE \
                   WHEN m1 LIKE '%:free' THEN SUBSTR(m1, 1, LENGTH(m1) - 5) \
                   WHEN m1 LIKE '%-free' THEN SUBSTR(m1, 1, LENGTH(m1) - 5) \
                   WHEN m1 LIKE '%-thinking' THEN SUBSTR(m1, 1, LENGTH(m1) - 9) \
                   ELSE m1 \
                 END AS m2 \
               FROM stripped \
             ), \
             prefixed AS ( \
               SELECT *, \
                 CASE WHEN m2 LIKE 'claude-%' THEN SUBSTR(m2, 8) ELSE m2 END AS m3 \
               FROM cleaned \
             ) \
             SELECT \
               CASE \
                 WHEN m3 IN ('opus', 'opus-4-6') THEN 'opus-4-6' \
                 WHEN m3 IN ('sonnet', 'sonnet-4-6') THEN 'sonnet-4-6' \
                 WHEN m3 IN ('haiku', 'haiku-4-5', 'haiku-4-5-20251001') THEN 'haiku-4-5' \
                 WHEN m3 IN ('qwen-3.6-max-preview', 'qwen3.6-max-preview', 'qwen-3-6-max-preview', 'qwen3-6-max-preview', 'qwen-max-preview') THEN 'qwen3.6-max-preview' \
                 WHEN m3 IN ('coder-model', 'qwen3.6-plus', 'qwen-3.6-plus') THEN 'qwen3.6-plus' \
                 WHEN m3 IN ('qwen3.5-plus', 'qwen-3.5-plus') THEN 'qwen3.5-plus' \
                 WHEN m3 IN ('minimax-m2.5') THEN 'minimax-m2.5' \
                 WHEN m3 IN ('minimax-m2.7') THEN 'minimax-m2.7' \
                 WHEN m3 IN ('mimo-v2-omni', 'mimo-v2-omni-free') THEN 'mimo-v2-omni' \
                 WHEN m3 IN ('mimo-v2-pro', 'mimo-v2-pro-free') THEN 'mimo-v2-pro' \
                 WHEN m3 IN ('kimi-k2.5', 'kimi-k2-5', 'kimi-k2.6', 'kimi-k2-6', 'kimik2.6') THEN 'kimi-k2.6' \
                 WHEN m3 IN ('glm-5-turbo', 'zhipu') THEN 'glm-5-turbo' \
                 ELSE m3 \
               END AS normalized_model, \
               COALESCE(SUM(token_count), 0), \
               COUNT(*) \
             FROM prefixed \
             GROUP BY normalized_model \
             ORDER BY SUM(token_count) DESC"
        );
        let mut stmt = conn.prepare(&query)?;
        let map_row = |row: &rusqlite::Row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        };
        let rows: Vec<(String, i64, i64)> = if let Some(s) = since {
            stmt.query_map(params![s], map_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?
        } else {
            stmt.query_map([], map_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        Ok::<_, rusqlite::Error>(rows)
    })
    .await
    .map_err(interact_err)?
    .context("Failed to fetch model stats")?;

    // Recalculate costs using current TOML pricing (80/20 input/output split)
    Ok(models
        .into_iter()
        .map(|(model, tokens, calls)| {
            let cost = pricing
                .as_ref()
                .and_then(|p| p.estimate_cost(&model, tokens))
                .unwrap_or(0.0);
            ModelStats {
                model,
                tokens,
                cost,
                calls,
                estimated: cost == 0.0 && tokens > 0,
            }
        })
        .collect())
}

async fn fetch_tools(pool: &Pool, since: Option<i64>) -> Result<Vec<ToolStats>> {
    let conn = pool.get().await.context("pool")?;
    conn.interact(move |conn| {
        let (query, param): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(s) = since {
            (
                "SELECT tool_name, COUNT(*) as cnt \
                 FROM tool_executions WHERE created_at >= ?1 \
                 GROUP BY tool_name ORDER BY cnt DESC",
                vec![Box::new(s)],
            )
        } else {
            (
                "SELECT tool_name, COUNT(*) as cnt \
                 FROM tool_executions \
                 GROUP BY tool_name ORDER BY cnt DESC",
                vec![],
            )
        };
        let refs: Vec<&dyn rusqlite::types::ToSql> = param.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(query)?;
        let rows = stmt.query_map(refs.as_slice(), |row| {
            Ok(ToolStats {
                tool_name: row.get(0)?,
                call_count: row.get(1)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
    })
    .await
    .map_err(interact_err)?
    .context("Failed to fetch tool stats")
}

async fn fetch_activities(pool: &Pool, since: Option<i64>) -> Result<Vec<ActivityStats>> {
    let conn = pool.get().await.context("pool")?;
    conn.interact(move |conn| -> rusqlite::Result<Vec<ActivityStats>> {
        // Fetch per-session stats with titles for classification
        let (query, param): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(s) = since {
            (
                "SELECT COALESCE(s.title, ''), \
                 COALESCE(SUM(u.cost), 0.0), COUNT(*), \
                 COUNT(DISTINCT u.session_id), s.category \
                 FROM usage_ledger u \
                 LEFT JOIN sessions s ON u.session_id = s.id \
                 WHERE u.created_at >= ?1 \
                 GROUP BY u.session_id",
                vec![Box::new(s)],
            )
        } else {
            (
                "SELECT COALESCE(s.title, ''), \
                 COALESCE(SUM(u.cost), 0.0), COUNT(*), \
                 COUNT(DISTINCT u.session_id), s.category \
                 FROM usage_ledger u \
                 LEFT JOIN sessions s ON u.session_id = s.id \
                 GROUP BY u.session_id",
                vec![],
            )
        };
        let refs: Vec<&dyn rusqlite::types::ToSql> = param.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(query)?;

        // Aggregate by activity category
        // (cost, turns, total_sessions, one_shot_sessions)
        let mut categories: std::collections::HashMap<String, (f64, i64, i64, i64)> =
            std::collections::HashMap::new();
        let rows = stmt.query_map(refs.as_slice(), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, f64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })?;
        for row in rows {
            let (title, cost, turns, explicit_cat) = row?;
            let category: String = match explicit_cat {
                Some(ref c) if !c.is_empty() => c.clone(),
                _ => classify_activity(&title).to_string(),
            };
            let entry = categories.entry(category).or_insert((0.0, 0, 0, 0));
            entry.0 += cost;
            entry.1 += turns;
            entry.2 += 1; // session count
            if turns <= 1 {
                entry.3 += 1; // one-shot session
            }
        }

        let mut result: Vec<ActivityStats> = categories
            .into_iter()
            .map(|(cat, (cost, turns, sessions, one_shot_sessions))| {
                let one_shot = if sessions > 0 {
                    (one_shot_sessions as f64 / sessions as f64) * 100.0
                } else {
                    0.0
                };
                ActivityStats {
                    category: cat.to_string(),
                    cost,
                    turns,
                    one_shot_pct: one_shot,
                }
            })
            .collect();
        result.sort_by(|a, b| {
            b.cost
                .partial_cmp(&a.cost)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(result)
    })
    .await
    .map_err(interact_err)?
    .context("Failed to fetch activity stats")
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Format token count for display (e.g., 1292500000 → "1292.5M")
pub fn fmt_tokens(t: i64) -> String {
    if t >= 1_000_000 {
        format!("{:.1}M", t as f64 / 1_000_000.0)
    } else if t >= 1_000 {
        format!("{:.0}K", t as f64 / 1_000.0)
    } else {
        format!("{}", t)
    }
}

/// Format cost for display
pub fn fmt_cost(c: f64) -> String {
    if c >= 1.0 {
        format!("${:.2}", c)
    } else if c >= 0.01 {
        format!("${:.3}", c)
    } else {
        format!("${:.4}", c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_period_cycle() {
        assert_eq!(Period::Today.next(), Period::Week);
        assert_eq!(Period::Week.next(), Period::Month);
        assert_eq!(Period::Month.next(), Period::AllTime);
        assert_eq!(Period::AllTime.next(), Period::Today);
    }

    #[test]
    fn test_period_since_epoch() {
        assert!(Period::Today.since_epoch().is_some());
        assert!(Period::Week.since_epoch().is_some());
        assert!(Period::Month.since_epoch().is_some());
        assert!(Period::AllTime.since_epoch().is_none());
    }

    #[test]
    fn test_period_labels() {
        assert_eq!(Period::Today.label(), "Today");
        assert_eq!(Period::Week.label(), "Week");
        assert_eq!(Period::Month.label(), "Month");
        assert_eq!(Period::AllTime.label(), "All Time");
    }

    #[test]
    fn test_classify_activity() {
        assert_eq!(classify_activity("fix login bug"), "Bug Fixes");
        assert_eq!(classify_activity("Fix crash on startup"), "Bug Fixes");
        assert_eq!(
            classify_activity("error handling improvements"),
            "Bug Fixes"
        );
        assert_eq!(classify_activity("refactor auth module"), "Refactoring");
        assert_eq!(classify_activity("cleanup old code"), "Refactoring");
        assert_eq!(classify_activity("add unit tests"), "Testing");
        assert_eq!(classify_activity("test coverage for parser"), "Testing");
        assert_eq!(classify_activity("update README"), "Documentation");
        assert_eq!(classify_activity("changelog updates"), "Documentation");
        assert_eq!(classify_activity("ci pipeline fix"), "CI/Deploy");
        assert_eq!(classify_activity("release v1.0"), "CI/Deploy");
        assert_eq!(classify_activity("deploy to prod"), "CI/Deploy");
        assert_eq!(classify_activity("add new feature"), "Features");
        assert_eq!(classify_activity("implement search"), "Features");
        assert_eq!(classify_activity("config file parsing"), "Config");
        assert_eq!(classify_activity("setup dev environment"), "Config");
        assert_eq!(classify_activity("random chat session"), "Development");
        assert_eq!(classify_activity(""), "Development");
    }

    #[test]
    fn test_fmt_tokens() {
        assert_eq!(fmt_tokens(0), "0");
        assert_eq!(fmt_tokens(500), "500");
        assert_eq!(fmt_tokens(1_500), "2K");
        assert_eq!(fmt_tokens(1_500_000), "1.5M");
        assert_eq!(fmt_tokens(1_292_500_000), "1292.5M");
    }

    #[test]
    fn test_fmt_cost() {
        assert_eq!(fmt_cost(0.0), "$0.0000");
        assert_eq!(fmt_cost(0.005), "$0.0050");
        assert_eq!(fmt_cost(0.05), "$0.050");
        assert_eq!(fmt_cost(1.50), "$1.50");
        assert_eq!(fmt_cost(507.20), "$507.20");
    }

    #[test]
    fn test_dashboard_data_default() {
        let d = DashboardData::default();
        assert_eq!(d.summary.total_tokens, 0);
        assert_eq!(d.summary.total_cost, 0.0);
        assert!(d.daily.is_empty());
        assert!(d.projects.is_empty());
        assert!(d.models.is_empty());
        assert!(d.tools.is_empty());
        assert!(d.activities.is_empty());
    }
}
