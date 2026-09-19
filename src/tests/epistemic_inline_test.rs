//! Epistemic confidence tracking.
//!
//! Moved out of `src/brain/tools/epistemic.rs`: tests live under `src/tests/`,
//! never inline beside the logic they exercise (#1076).

use crate::brain::tools::epistemic::*;
use chrono::Utc;

#[test]
fn test_confidence_ordering() {
    assert!(Confidence::Verified > Confidence::Inferred);
    assert!(Confidence::Inferred > Confidence::Uncertain);
    assert!(Confidence::Uncertain > Confidence::Contradicted);
}

#[test]
fn test_confidence_decay() {
    assert_eq!(Confidence::Verified.decay(), Confidence::Verified);
    assert_eq!(Confidence::Inferred.decay(), Confidence::Uncertain);
    assert_eq!(Confidence::Uncertain.decay(), Confidence::Contradicted);
    assert_eq!(Confidence::Contradicted.decay(), Confidence::Contradicted);
}

#[test]
fn test_add_belief_no_contradiction() {
    let mut store = EpistemicStore::new();
    let result = store.add_belief("test:key", "value1", Confidence::Verified, "test");
    assert_eq!(result, ContradictionResult::NoContradiction);
    assert_eq!(store.get_belief("test:key").unwrap().value, "value1");
}

#[test]
fn test_add_belief_contradiction() {
    let mut store = EpistemicStore::new();
    store.add_belief("test:key", "value1", Confidence::Verified, "test");
    let result = store.add_belief("test:key", "value2", Confidence::Inferred, "test2");

    assert!(matches!(result, ContradictionResult::Contradicted { .. }));

    // Old belief should be marked as contradicted
    let contradicted: Vec<_> = store.list_contradictions();
    assert_eq!(contradicted.len(), 1);
    assert_eq!(contradicted[0].value, "value1");

    // New belief should be active
    assert_eq!(store.get_belief("test:key").unwrap().value, "value2");
}

#[test]
fn test_verify_belief() {
    let mut store = EpistemicStore::new();
    store.add_belief("test:key", "value", Confidence::Uncertain, "test");
    assert!(store.verify_belief("test:key"));
    assert_eq!(
        store.get_belief("test:key").unwrap().confidence,
        Confidence::Verified
    );
}

#[test]
fn test_decay_logic() {
    let mut store = EpistemicStore::new();

    // Add a belief with old last_verified
    let mut belief = Belief {
        key: "test:old".to_string(),
        value: "old_value".to_string(),
        confidence: Confidence::Inferred,
        source: Source {
            origin: "test".to_string(),
            recorded_at: Utc::now() - chrono::Duration::days(45),
            last_verified: Utc::now() - chrono::Duration::days(45),
        },
        notes: None,
        hits: 0,
        last_used: None,
    };
    store.beliefs.insert("test:old".to_string(), belief.clone());

    // Add a recent belief
    belief.key = "test:recent".to_string();
    belief.confidence = Confidence::Inferred;
    belief.source.last_verified = Utc::now();
    store.beliefs.insert("test:recent".to_string(), belief);

    // Apply decay with 30-day threshold
    let decayed = store.apply_decay(30);

    // Only the old belief should decay
    assert_eq!(decayed.len(), 1);
    assert!(decayed[0].contains("test:old"));
    assert_eq!(
        store.get_belief("test:old").unwrap().confidence,
        Confidence::Uncertain
    );
    assert_eq!(
        store.get_belief("test:recent").unwrap().confidence,
        Confidence::Inferred
    );
}

#[test]
fn test_verified_beliefs_dont_decay() {
    let mut store = EpistemicStore::new();

    let belief = Belief {
        key: "test:verified".to_string(),
        value: "verified_value".to_string(),
        confidence: Confidence::Verified,
        source: Source {
            origin: "test".to_string(),
            recorded_at: Utc::now() - chrono::Duration::days(100),
            last_verified: Utc::now() - chrono::Duration::days(100),
        },
        notes: None,
        hits: 0,
        last_used: None,
    };
    store.beliefs.insert("test:verified".to_string(), belief);

    let decayed = store.apply_decay(30);
    assert!(decayed.is_empty());
    assert_eq!(
        store.get_belief("test:verified").unwrap().confidence,
        Confidence::Verified
    );
}

#[test]
fn test_serialization_roundtrip() {
    let mut store = EpistemicStore::new();
    store.add_belief("test:key", "value", Confidence::Inferred, "test:origin");

    let toml_str = toml::to_string_pretty(&store).unwrap();
    let loaded: EpistemicStore = toml::from_str(&toml_str).unwrap();

    assert_eq!(loaded.get_belief("test:key").unwrap().value, "value");
    assert_eq!(
        loaded.get_belief("test:key").unwrap().confidence,
        Confidence::Inferred
    );
}

#[test]
fn test_list_by_key_prefix() {
    let mut store = EpistemicStore::new();
    store.add_belief(
        "plan:task:1:abc",
        "failed",
        Confidence::Contradicted,
        "test",
    );
    store.add_belief("plan:task:2:def", "done", Confidence::Verified, "test");
    store.add_belief(
        "memory:truelens:ip",
        "1.2.3.4",
        Confidence::Inferred,
        "test",
    );

    let plan_beliefs = store.list_by_key_prefix("plan:task:");
    assert_eq!(plan_beliefs.len(), 2);

    let memory_beliefs = store.list_by_key_prefix("memory:");
    assert_eq!(memory_beliefs.len(), 1);
    assert_eq!(memory_beliefs[0].key, "memory:truelens:ip");

    let empty = store.list_by_key_prefix("nonexistent:");
    assert!(empty.is_empty());
}

/// #1641: touch_belief increments hits and sets last_used.
#[test]
fn test_touch_belief() {
    let mut store = EpistemicStore::new();
    store.add_belief("test:key", "value", Confidence::Inferred, "test");

    let belief = store.get_belief("test:key").unwrap();
    assert_eq!(belief.hits, 0);
    assert!(belief.last_used.is_none());

    // First touch
    assert!(store.touch_belief("test:key"));
    let belief = store.get_belief("test:key").unwrap();
    assert_eq!(belief.hits, 1);
    assert!(belief.last_used.is_some());

    // Second touch
    assert!(store.touch_belief("test:key"));
    let belief = store.get_belief("test:key").unwrap();
    assert_eq!(belief.hits, 2);

    // Non-existent key
    assert!(!store.touch_belief("nonexistent"));
}

/// #1641: recently-used beliefs don't decay even if never re-verified.
/// The decay clock uses last_used (falling back to recorded_at), not
/// last_verified.
#[test]
fn test_decay_uses_last_used_not_last_verified() {
    let mut store = EpistemicStore::new();

    // Old belief that was recently touched — should NOT decay
    let mut belief = Belief {
        key: "test:old_but_used".to_string(),
        value: "still_valid".to_string(),
        confidence: Confidence::Inferred,
        source: Source {
            origin: "test".to_string(),
            recorded_at: Utc::now() - chrono::Duration::days(60),
            last_verified: Utc::now() - chrono::Duration::days(60),
        },
        notes: None,
        hits: 5,
        last_used: Some(Utc::now() - chrono::Duration::days(2)),
    };
    store
        .beliefs
        .insert("test:old_but_used".to_string(), belief.clone());

    // Old belief never touched — SHOULD decay
    belief.key = "test:old_unused".to_string();
    belief.value = "stale".to_string();
    belief.hits = 0;
    belief.last_used = None;
    store
        .beliefs
        .insert("test:old_unused".to_string(), belief);

    let decayed = store.apply_decay(30);

    // Only the unused one decayed
    assert_eq!(decayed.len(), 1);
    assert!(decayed[0].contains("test:old_unused"));

    // The recently-used one is still Inferred
    assert_eq!(
        store.get_belief("test:old_but_used").unwrap().confidence,
        Confidence::Inferred
    );

    // The unused one dropped to Uncertain
    assert_eq!(
        store.get_belief("test:old_unused").unwrap().confidence,
        Confidence::Uncertain
    );
}
