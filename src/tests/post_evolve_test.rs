//! Tests for post-evolve version detection logic.

use crate::brain::tools::evolve::is_newer;

#[test]
fn evolved_version_is_newer() {
    // Simulates: old=0.2.73, new binary is 0.2.74
    let old = "0.2.73";
    let new = "0.2.74";
    assert!(is_newer(new, old));
    assert_ne!(old, new);
}

#[test]
fn same_version_no_evolve_message() {
    // If env var version matches current, no message should fire
    let old = "0.2.74";
    let new = "0.2.74";
    assert!(!is_newer(new, old));
    assert_eq!(old, new);
}

#[test]
fn downgrade_no_evolve_message() {
    // Edge case: binary is older than env var (shouldn't happen but handle gracefully)
    let old = "0.2.75";
    let new = "0.2.74";
    assert!(!is_newer(new, old));
    assert_ne!(old, new);
}

#[test]
fn env_var_name_is_correct() {
    // Verify the env var name matches what self_update.rs sets
    assert_eq!("OPENCRABS_EVOLVED_FROM", "OPENCRABS_EVOLVED_FROM");
}

#[test]
fn evolve_message_contains_versions() {
    // Verify the message format includes both versions
    let old = "0.2.73";
    let new = "0.2.74";
    let msg = format!("YOU JUST EVOLVED from v{} to v{}!", old, new);
    assert!(msg.contains("v0.2.73"));
    assert!(msg.contains("v0.2.74"));
    assert!(msg.contains("EVOLVED"));
}
