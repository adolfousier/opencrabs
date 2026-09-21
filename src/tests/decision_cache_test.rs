//! Tests for the L1 exact-decision-reuse ring (#1648, PR1).
//!
//! The cache identity is (tier, policy_version, normalizer_version,
//! canonical input) under sha256. These pin the two failure directions the
//! ring must never allow: a volatile field forking the key so repeats miss
//! forever (the cache silently does nothing), and a semantic change sharing
//! a key so a stale answer gets reused (the cache silently lies). Plus the
//! write-gate rules: sub-floor margins refused, re-put never duplicating.

use crate::db::Database;
use crate::db::repository::decision_cache::{DecisionCacheRepository, DecisionPut};
use crate::decisions::normalize::{NORMALIZER_VERSION, canonical_json, decision_key};
use serde_json::json;

async fn setup() -> (DecisionCacheRepository, Database) {
    let db = Database::connect_in_memory()
        .await
        .expect("in-memory database");
    db.run_migrations().await.expect("migrations");
    (DecisionCacheRepository::new(db.pool().clone()), db)
}

fn put_input(key: &str, tier: &str, policy: &str, margin: Option<f64>) -> DecisionPut {
    DecisionPut {
        key: key.to_string(),
        tier_id: tier.to_string(),
        result_json: "{\"category\":\"bugfix\"}".to_string(),
        p: Some(0.91),
        margin,
        policy_version: policy.to_string(),
        normalizer_version: NORMALIZER_VERSION.to_string(),
    }
}

fn key_of(payload: &serde_json::Value) -> String {
    decision_key("triage", "p1", NORMALIZER_VERSION, &canonical_json(payload))
}

#[test]
fn decision_cache_volatile_variants_share_one_key() {
    let a = json!({
        "draft": "fix the flaky cron",
        "category": "bugfix",
        "ts": "2026-09-20T23:57:12Z",
        "request_id": "550e8400-e29b-41d4-a716-446657890700",
        "client_ip": "10.0.1.23:8080",
        "cwd": "/Users/adolfo/srv/x",
    });
    let b = json!({
        "draft": "fix the flaky cron",
        "category": "bugfix",
        "ts": "2026-09-21 00:02:03",
        "request_id": "9c5a1f2e-77aa-49bb-8c3d-0f1e2d3c4b5a",
        "client_ip": "192.168.4.19",
        "cwd": "/home/other/srv/x",
    });
    assert_eq!(
        key_of(&a),
        key_of(&b),
        "volatile fields must not fork the key"
    );

    // The same masks must not swallow the signal: a changed category forks.
    let c = json!({
        "draft": "fix the flaky cron",
        "category": "feature",
        "ts": "2026-09-20T23:57:12Z",
        "request_id": "550e8400-e29b-41d4-a716-446657890700",
        "client_ip": "10.0.1.23:8080",
        "cwd": "/Users/adolfo/srv/x",
    });
    assert_ne!(key_of(&a), key_of(&c), "semantic change must fork the key");
}

#[test]
fn decision_cache_key_order_irrelevant_array_order_matters() {
    let x = json!({"a": 1, "b": {"c": true, "d": "s"}});
    let y = json!({"b": {"d": "s", "c": true}, "a": 1});
    assert_eq!(canonical_json(&x), canonical_json(&y));

    let arr_a = json!({"rules": ["alpha", "beta"]});
    let arr_b = json!({"rules": ["beta", "alpha"]});
    assert_ne!(
        canonical_json(&arr_a),
        canonical_json(&arr_b),
        "arrays are semantically ordered; sorting them would merge distinct inputs"
    );
}

#[tokio::test]
async fn decision_cache_policy_version_bump_misses() {
    let (repo, _db) = setup().await;
    let payload = json!({"q": "route this request"});
    let canonical = canonical_json(&payload);
    let k1 = decision_key("triage", "p1", NORMALIZER_VERSION, &canonical);
    let k2 = decision_key("triage", "p2", NORMALIZER_VERSION, &canonical);
    assert_ne!(k1, k2, "a policy bump must derive a different key");

    repo.put(put_input(&k1, "triage", "p1", Some(0.9)), 0.5)
        .await
        .expect("put p1");
    assert!(repo.get(&k1).await.expect("get k1").is_some());
    assert!(
        repo.get(&k2).await.expect("get k2").is_none(),
        "policy v2 must not read v1's answer"
    );
}

#[tokio::test]
async fn decision_cache_margin_floor_refuses_write() {
    let (repo, _db) = setup().await;

    // Measurable but borderline: refused, nothing stored.
    let low = key_of(&json!({"q": "low margin", "n": 1}));
    let stored = repo
        .put(put_input(&low, "triage", "p1", Some(0.05)), 0.2)
        .await
        .expect("put low");
    assert!(!stored, "margin below floor must be refused");
    assert!(repo.get(&low).await.expect("get low").is_none());

    // Exactly at the floor is storable (the floor is "refuse below it").
    let at = key_of(&json!({"q": "at floor", "n": 2}));
    assert!(
        repo.put(put_input(&at, "triage", "p1", Some(0.2)), 0.2)
            .await
            .expect("put at"),
        "margin equal to the floor must be accepted"
    );

    // Unmeasurable (deterministic-shaped) passes the gate.
    let none = key_of(&json!({"q": "no margin", "n": 3}));
    assert!(
        repo.put(put_input(&none, "triage", "p1", None), 0.2)
            .await
            .expect("put none"),
        "margin None is not a borderline decision"
    );
}

#[tokio::test]
async fn decision_cache_reput_single_row_hits_accumulate() {
    let (repo, _db) = setup().await;
    let key = key_of(&json!({"q": "reuse me"}));

    repo.put(put_input(&key, "triage", "p1", Some(0.9)), 0.5)
        .await
        .expect("first put");
    repo.bump_hits(&key).await.expect("bump 1");
    repo.bump_hits(&key).await.expect("bump 2");

    // Re-derive and re-put the same key: must refresh in place, not fork a
    // second row and not reset the hit counter.
    repo.put(put_input(&key, "triage", "p1", Some(0.95)), 0.5)
        .await
        .expect("re-put");

    let row = repo.get(&key).await.expect("get").expect("row present");
    assert_eq!(row.hits, 2, "re-put must preserve the hit counter");
    assert!(row.last_used_at.is_some(), "bumps must stamp last_used_at");
    assert_eq!(
        row.margin,
        Some(0.95),
        "re-put must refresh the stored result"
    );

    // One row for this slice: the scoped delete returns exactly one.
    assert_eq!(
        repo.delete_by_tier_version("triage", "p1")
            .await
            .expect("delete"),
        1,
        "re-put must not duplicate the row"
    );
    assert!(repo.get(&key).await.expect("get after").is_none());
}

#[tokio::test]
async fn decision_cache_delete_scoped_to_tier_and_version() {
    let (repo, _db) = setup().await;
    let k_triage_p1 = key_of(&json!({"q": "triage p1"}));
    let k_triage_p2 = decision_key(
        "triage",
        "p2",
        NORMALIZER_VERSION,
        &canonical_json(&json!({"q": "triage p1"})),
    );
    let k_route_p1 = decision_key(
        "route",
        "p1",
        NORMALIZER_VERSION,
        &canonical_json(&json!({"q": "triage p1"})),
    );

    for (key, tier, policy) in [
        (k_triage_p1.as_str(), "triage", "p1"),
        (k_triage_p2.as_str(), "triage", "p2"),
        (k_route_p1.as_str(), "route", "p1"),
    ] {
        repo.put(put_input(key, tier, policy, Some(0.9)), 0.5)
            .await
            .expect("seed");
    }

    assert_eq!(
        repo.delete_by_tier_version("triage", "p1")
            .await
            .expect("delete slice"),
        1
    );
    assert!(repo.get(&k_triage_p1).await.expect("g1").is_none());
    assert!(
        repo.get(&k_triage_p2).await.expect("g2").is_some(),
        "other policy_version survives"
    );
    assert!(
        repo.get(&k_route_p1).await.expect("g3").is_some(),
        "other tier survives"
    );
}
