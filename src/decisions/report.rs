//! Decision accounting renderers (#1648, PR3).
//!
//! The release-day kill rule ("evaluate til next release if we keep or
//! not") only works if the numbers are visible without opening SQLite, so
//! `/usage` grows a decisions block and Mission Control gains the same
//! counters. This module is pure formatting: counters in, markdown out.
//!
//! Silence rule: no counters, no block. The feature being unused must not
//! add noise to a dashboard nobody asked a question about; the block
//! appears exactly when there is evidence to show.

use crate::config::{DecisionMode, DecisionTierConfig};
use crate::db::repository::DecisionStats;
use std::collections::BTreeMap;

fn mode_label(mode: DecisionMode) -> &'static str {
    match mode {
        DecisionMode::Shadow => "shadow",
        DecisionMode::Live => "live",
        DecisionMode::Off => "off",
    }
}

fn pct(part: i64, whole: i64) -> String {
    if whole > 0 {
        format!("{:.0}%", (part as f64 / whole as f64) * 100.0)
    } else {
        "0%".to_string()
    }
}

/// Per-tier lines for the usage surfaces. `stats` are the persisted call
/// counters, `cached_rows` the per-tier `decision_cache` row counts, and
/// `tiers` the live `[decisions.tiers]` config (a tier with counters but no
/// config line renders "(unconfigured)", which is precisely the stale state
/// the promotion review needs to catch).
///
/// Returns `None` when there is nothing measured yet (kill-rule silence).
pub fn render_usage_lines(
    stats: &[DecisionStats],
    cached_rows: &[(String, i64)],
    tiers: &BTreeMap<String, DecisionTierConfig>,
) -> Option<Vec<String>> {
    if stats.is_empty() {
        return None;
    }
    let mut lines = Vec::with_capacity(stats.len() + 2);
    lines.push("Decision cache (#1648), per tier:".to_string());
    for s in stats {
        let tier_cfg = tiers.get(&s.tier_id);
        let mode = tier_cfg.map_or("(unconfigured)", |c| mode_label(c.mode));
        let policy = tier_cfg.map_or("?", |c| c.policy_version.as_str());
        let rows = cached_rows
            .iter()
            .find(|(t, _)| t == &s.tier_id)
            .map_or(0, |(_, n)| *n);
        lines.push(format!(
            "- `{tier}` [{mode} v{policy}]: {calls} calls, would-hit {would} ({would_pct}), live-hit {live} ({live_pct}), {rows} rows cached",
            tier = s.tier_id,
            calls = s.calls,
            would = s.would_hit,
            would_pct = pct(s.would_hit, s.calls),
            live = s.live_hit,
            live_pct = pct(s.live_hit, s.calls),
        ));
    }
    // The avoided-call estimate: each live hit skipped a model round-trip.
    // Token math would need the decision calls in the usage ledger, which
    // shadow-mode deliberately does not add; call count is the honest unit.
    let total_live: i64 = stats.iter().map(|s| s.live_hit).sum();
    lines.push(format!(
        "- est. model calls avoided (live hits): {total_live}"
    ));
    Some(lines)
}

/// Mission Control /channel-report variant: the same data as one markdown
/// block for the rich renderers. Silent under the same rule.
pub fn render_usage_block(
    stats: &[DecisionStats],
    cached_rows: &[(String, i64)],
    tiers: &BTreeMap<String, DecisionTierConfig>,
) -> Option<String> {
    render_usage_lines(stats, cached_rows, tiers).map(|lines| lines.join("\n"))
}
