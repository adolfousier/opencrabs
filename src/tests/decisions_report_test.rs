//! Decision accounting tests (#1648, PR3).
//!
//! Two contracts under test. The silence rule: the /usage decisions block
//! renders iff counters exist, and stays silent otherwise (an unmeasured
//! feature must not fake a dashboard, and an empty block on release day is
//! itself the cut signal). The sweeper rule: TTL pruning bounds
//! `decision_cache` growth, recency is `last_used_at` when a hit refreshed
//! it else `created_at`, and rows for tiers no longer in `[decisions.tiers]`
//! are garbage that must go.

use crate::config::{Config, DecisionTierConfig};
use crate::db::Database;
use crate::db::repository::DecisionStatsRepository;
use crate::db::repository::decision_cache::{DecisionCacheRepository, DecisionPut};
use crate::decisions::normalize::NORMALIZER_VERSION;
use crate::decisions::report::render_usage_lines;
use std::collections::BTreeMap;

fn tiers_from(tiers_toml: &str) -> BTreeMap<String, DecisionTierConfig> {
    let cfg: Config =
        toml::from_str(&format!("[decisions]\n{tiers_toml}")).expect("minimal decisions config");
    cfg.decisions.tiers
}

fn triage_config() -> DecisionTierConfig {
    tiers_from("[decisions.tiers.triage]\npolicy_version = \"p1\"\n")
        .remove("triage")
        .expect("triage tier")
}

fn stat(
    tier: &str,
    calls: i64,
    would_hit: i64,
    live_hit: i64,
) -> crate::db::repository::DecisionStats {
    crate::db::repository::DecisionStats {
        tier_id: tier.to_string(),
        calls,
        would_hit,
        live_hit,
        last_seen_at: 0,
    }
}

// ── The silence rule ────────────────────────────────────────────────────────

#[test]
fn decisions_usage_block_renders_iff_counters_exist() {
    // No counters: silent. This is the release-day kill-rule shape.
    assert!(
        render_usage_lines(&[], &[], &BTreeMap::new()).is_none(),
        "no counters must render nothing"
    );

    // Counters present: block appears with the numbers the evaluation reads.
    let tiers = tiers_from(
        "[decisions.tiers.triage]\npolicy_version = \"p1\"\nmode = \"shadow\"\nttl_hours = 168\n",
    );
    let stats = vec![stat("triage", 40, 30, 4)];
    let rows = vec![("triage".to_string(), 7i64)];
    let lines = render_usage_lines(&stats, &rows, &tiers).expect("block renders");
    let text = lines.join("\n");
    assert!(text.contains("triage"), "tier named: {text}");
    assert!(
        text.contains("[shadow vp1]"),
        "mode + policy_version: {text}"
    );
    assert!(text.contains("40 calls"), "calls: {text}");
    assert!(
        text.contains("would-hit 30 (75%)"),
        "would-hit rate: {text}"
    );
    assert!(text.contains("live-hit 4 (10%)"), "live-hit rate: {text}");
    assert!(text.contains("7 rows cached"), "row count: {text}");
    assert!(
        text.contains("est. model calls avoided (live hits): 4"),
        "avoided-call estimate: {text}"
    );
    assert!(!text.contains('\u{2014}'), "no em dashes in usage output");
}

#[test]
fn counters_without_config_render_as_unconfigured() {
    // A tier whose [decisions.tiers] entry was removed keeps its counters
    // visible (the evidence must survive the kill switch until release day),
    // but flagged so the stale state is obvious.
    let stats = vec![stat("ghost", 10, 8, 0)];
    let lines = render_usage_lines(&stats, &[], &BTreeMap::new()).expect("renders");
    let text = lines.join("\n");
    assert!(
        text.contains("(unconfigured)"),
        "stale tier flagged: {text}"
    );
    assert!(
        text.contains("would-hit 8 (80%)"),
        "counters still shown: {text}"
    );
}

// ── The sweeper rule ────────────────────────────────────────────────────────

async fn setup() -> (DecisionCacheRepository, DecisionStatsRepository, Database) {
    let db = Database::connect_in_memory()
        .await
        .expect("in-memory database");
    db.run_migrations().await.expect("migrations");
    (
        DecisionCacheRepository::new(db.pool().clone()),
        DecisionStatsRepository::new(db.pool().clone()),
        db,
    )
}

fn put(key: &str, tier: &str) -> DecisionPut {
    DecisionPut {
        key: key.to_string(),
        tier_id: tier.to_string(),
        result_json: "{\"decision\":\"bugfix\"}".to_string(),
        p: Some(0.9),
        margin: Some(0.5),
        policy_version: "p1".to_string(),
        normalizer_version: NORMALIZER_VERSION.to_string(),
    }
}

/// Rewind a row's clocks directly: the repositories only ever write "now",
/// and time is the thing under test.
async fn age(db: &Database, key: &str, created_ago: i64, used_ago: Option<i64>) {
    let pool = db.pool().clone();
    let key = key.to_string();
    pool.get()
        .await
        .expect("conn")
        .interact(move |conn| {
            conn.execute(
                "UPDATE decision_cache SET created_at = strftime('%s','now') - ?1 WHERE key = ?2",
                rusqlite::params![created_ago, key],
            )?;
            if let Some(used_ago) = used_ago {
                conn.execute(
                    "UPDATE decision_cache SET last_used_at = strftime('%s','now') - ?1 WHERE key = ?2",
                    rusqlite::params![used_ago, key],
                )?;
            }
            Ok::<_, rusqlite::Error>(())
        })
        .await
        .expect("interact")
        .expect("age row");
}

#[tokio::test]
async fn ttl_prune_goes_by_last_used_at_when_set() {
    let (cache, _, db) = setup().await;
    assert!(cache.put(put("k-old", "triage"), 0.0).await.unwrap());
    assert!(cache.put(put("k-refreshed", "triage"), 0.0).await.unwrap());
    assert!(cache.put(put("k-fresh", "triage"), 0.0).await.unwrap());
    // 400h old on both clocks: out under a 168h ttl.
    age(&db, "k-old", 400 * 3600, None).await;
    // Created 400h ago but hit 1h ago: last_used_at keeps it (a row is only
    // ever kept by being useful).
    age(&db, "k-refreshed", 400 * 3600, Some(3600)).await;

    let n = cache.prune_expired("triage", 168).await.expect("prune");
    assert_eq!(n, 1, "only the untouched-expired row goes");
    assert!(cache.get("k-old").await.unwrap().is_none());
    assert!(
        cache.get("k-refreshed").await.unwrap().is_some(),
        "recently-used row must survive"
    );
    assert!(cache.get("k-fresh").await.unwrap().is_some());
    // Other tiers are untouched by a per-tier prune.
    assert!(cache.put(put("k-other", "route"), 0.0).await.unwrap());
    age(&db, "k-other", 400 * 3600, None).await;
    assert_eq!(cache.prune_expired("triage", 168).await.unwrap(), 0);
    assert!(cache.get("k-other").await.unwrap().is_some());
}

#[tokio::test]
async fn orphan_tier_rows_are_swept_known_rows_stay() {
    let (cache, _, db) = setup().await;
    assert!(cache.put(put("k-known", "triage"), 0.0).await.unwrap());
    assert!(
        cache
            .put(put("k-orphan", "removed-tier"), 0.0)
            .await
            .unwrap()
    );
    age(&db, "k-orphan", 400 * 3600, None).await;

    let known = vec!["triage".to_string()];
    let n = cache
        .delete_unknown_tiers(&known)
        .await
        .expect("sweep orphans");
    assert_eq!(n, 1, "the orphan goes, the known tier stays");
    assert!(cache.get("k-known").await.unwrap().is_some());
    assert!(cache.get("k-orphan").await.unwrap().is_none());
}

#[tokio::test]
async fn empty_known_list_clears_the_table() {
    // Kill switch shape: [decisions] removed from config means every stored
    // row is unreadable garbage; one sweep drops the lot.
    let (cache, _, _db) = setup().await;
    assert!(cache.put(put("k-a", "triage"), 0.0).await.unwrap());
    assert!(cache.put(put("k-b", "route"), 0.0).await.unwrap());
    let n = cache.delete_unknown_tiers(&[]).await.expect("clear");
    assert_eq!(n, 2);
    assert!(cache.count_by_tier().await.unwrap().is_empty());
}

#[tokio::test]
async fn counters_seed_from_bumps_and_render_end_to_end() {
    // The release-day evidence path: bumps accumulate across calls (and
    // survive repeated bumps on the same tier), then the same rows the /usage
    // surfaces read produce the block.
    let (_cache, stats_repo, _db) = setup().await;
    for i in 0..3 {
        stats_repo
            .bump("triage", i == 2, false)
            .await
            .expect("bump");
    }
    stats_repo.bump("triage", false, true).await.expect("bump");

    let rows = stats_repo.all().await.expect("all");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].calls, 4);
    assert_eq!(rows[0].would_hit, 1);
    assert_eq!(rows[0].live_hit, 1);

    let tiers = tiers_from("[decisions.tiers.triage]\npolicy_version = \"p1\"\n");
    let cached = vec![("triage".to_string(), 2i64)];
    let text = render_usage_lines(&rows, &cached, &tiers)
        .expect("counters exist, block renders")
        .join("\n");
    assert!(text.contains("4 calls"), "{text}");
    assert!(text.contains("would-hit 1 (25%)"), "{text}");
    assert!(text.contains("live-hit 1 (25%)"), "{text}");
}

#[test]
fn triage_config_default_mode_is_shadow() {
    // Belt for the renderer: the example tier line renders as shadow, the
    // default mode, never as an absent mode.
    let mut tiers = BTreeMap::new();
    tiers.insert("triage".to_string(), triage_config());
    let text = render_usage_lines(&[stat("triage", 1, 0, 0)], &[], &tiers)
        .expect("renders")
        .join("\n");
    assert!(text.contains("[shadow vp1]"), "{text}");
}
