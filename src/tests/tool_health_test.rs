//! #1706 regression tests: tool health annotations built from feedback
//! ledger stats, and the ledger round-trip that feeds them. The pure
//! builder (`build_tool_health_annotations`) is the contract the schema
//! path depends on: flag a tool only when it has at least
//! `TOOL_HEALTH_MIN_SAMPLES` attempts in the window AND a success rate
//! below `TOOL_HEALTH_MAX_RATE`; never flag sentinel dimensions.

use crate::brain::agent::service::feedback::{
    TOOL_HEALTH_MAX_RATE, TOOL_HEALTH_MIN_SAMPLES, build_tool_health_annotations,
};
use crate::db::database::Database;
use crate::db::repository::FeedbackLedgerRepository;
use crate::db::repository::feedback_ledger::DimensionStats;

fn stat(dimension: &str, total: i64, successes: i64) -> DimensionStats {
    DimensionStats {
        dimension: dimension.to_string(),
        total_events: total,
        successes,
        failures: total - successes,
        success_rate: if total > 0 {
            successes as f64 / total as f64
        } else {
            0.0
        },
        avg_value: 0.0,
    }
}

#[test]
fn zero_success_tool_gets_broken_warning() {
    let stats = vec![stat("codegraph_explore", 6, 0)];
    let map = build_tool_health_annotations(&stats);
    let warning = map.get("codegraph_explore").expect("dead tool flagged");
    assert!(
        warning.contains("0/6"),
        "warning states the numbers: {warning}"
    );
    assert!(
        warning.contains("appears broken"),
        "zero successes say broken: {warning}"
    );
    assert!(
        warning.contains("prefer an alternative"),
        "actionable: {warning}"
    );
}

#[test]
fn partial_success_tool_gets_verify_warning() {
    let stats = vec![stat("flaky_tool", 9, 2)];
    let map = build_tool_health_annotations(&stats);
    let warning = map.get("flaky_tool").expect("under-half tool flagged");
    assert!(
        warning.contains("only 2/9"),
        "warning states the numbers: {warning}"
    );
    assert!(
        warning.contains("verify"),
        "partial success says verify, not broken: {warning}"
    );
}

#[test]
fn healthy_tool_not_flagged() {
    let stats = vec![stat("bash", 20, 18)];
    let map = build_tool_health_annotations(&stats);
    assert!(map.is_empty(), "healthy tool must not be flagged: {map:?}");
}

#[test]
fn below_min_samples_not_flagged() {
    // 0/3 is a 0% rate but 3 samples is noise, not evidence. Isolates the
    // floor: the rate condition alone would let this through to flagging.
    let stats = vec![stat("once_used_tool", 3, 0)];
    let map = build_tool_health_annotations(&stats);
    assert!(
        map.is_empty(),
        "below {TOOL_HEALTH_MIN_SAMPLES} attempts must not flag: {map:?}"
    );
}

#[test]
fn exactly_at_max_rate_not_flagged() {
    // rate == TOOL_HEALTH_MAX_RATE is the boundary: only strictly-below flags.
    let stats = vec![stat("boundary_tool", 10, 5)];
    assert_eq!(stats[0].success_rate, TOOL_HEALTH_MAX_RATE);
    let map = build_tool_health_annotations(&stats);
    assert!(map.is_empty(), "boundary rate must not flag: {map:?}");
}

#[test]
fn sentinel_dimensions_never_flagged() {
    let stats = vec![stat("phantom_tool_call", 20, 0), stat("", 12, 0)];
    let map = build_tool_health_annotations(&stats);
    assert!(
        map.is_empty(),
        "diagnostic dimensions are not tools: {map:?}"
    );
}

#[test]
fn mixed_set_flags_only_dead_tools() {
    let stats = vec![
        stat("codegraph_explore", 6, 0),   // dead: flagged
        stat("camofox_create_tab", 14, 0), // dead: flagged
        stat("bash", 50, 48),              // healthy
        stat("exa_search", 3, 0),          // too few samples
        stat("phantom_tool_call", 9, 0),   // sentinel
    ];
    let map = build_tool_health_annotations(&stats);
    assert_eq!(map.len(), 2, "exactly the two dead tools: {map:?}");
    assert!(map.contains_key("codegraph_explore"));
    assert!(map.contains_key("camofox_create_tab"));
}

/// Full data-path round trip: failures recorded through the real ledger
/// come back as an annotation. Uses the same in-memory DB fixture as the
/// rsi tests.
#[tokio::test]
async fn ledger_round_trip_feeds_annotations() {
    let db = Database::connect_in_memory().await.expect("in-memory DB");
    db.run_migrations().await.expect("migrations");
    let repo = FeedbackLedgerRepository::new(db.pool().clone());

    // The #1706 evidence profile: a dead tool failing every attempt.
    for _ in 0..6 {
        repo.record("sess", "tool_failure", "codegraph_explore", 0.0, None)
            .await
            .expect("record failure");
    }
    // A healthy tool in the same window must stay unflagged.
    for _ in 0..8 {
        repo.record("sess", "tool_success", "bash", 1.0, None)
            .await
            .expect("record success");
    }

    let window_since = (chrono::Utc::now() - chrono::Duration::days(7))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    let stats = repo
        .stats_by_dimension_since("tool_", Some(&window_since))
        .await
        .expect("stats query");

    let map = build_tool_health_annotations(&stats);
    let warning = map.get("codegraph_explore").expect("dead tool flagged");
    assert!(warning.contains("0/6"));
    assert!(!map.contains_key("bash"), "healthy tool unflagged: {map:?}");
}
