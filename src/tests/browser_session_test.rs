//! Test the per-session-tab key format used by the P5 fix.
//!
//! Each agent session gets its own tab so concurrent turns on
//! different sessions don't stomp on each other's DOM state. The
//! HashMap key is `format!("session-{}", session_id)` — stable so
//! reconnecting to the same session id returns the same tab, and
//! prefixed so it can never collide with the legacy "default" key.

use crate::brain::tools::browser::BrowserManager;
use uuid::Uuid;

#[test]
fn session_key_is_stable_per_session_id() {
    let id = Uuid::new_v4();
    let k1 = BrowserManager::page_name_for_session(id);
    let k2 = BrowserManager::page_name_for_session(id);
    assert_eq!(
        k1, k2,
        "same session id must hash to the same page key — tabs are reused, not re-created per turn"
    );
}

#[test]
fn different_sessions_get_different_keys() {
    let a = BrowserManager::page_name_for_session(Uuid::new_v4());
    let b = BrowserManager::page_name_for_session(Uuid::new_v4());
    assert_ne!(a, b, "two sessions must never share a tab");
}

#[test]
fn session_key_never_equals_legacy_default() {
    // The legacy tab key "default" still exists for non-session-aware
    // callers (e.g. the old `take_screenshot()` fallback). Session
    // keys must have the `session-` prefix so they never collide.
    let id = Uuid::new_v4();
    let key = BrowserManager::page_name_for_session(id);
    assert_ne!(key, "default");
    assert!(
        key.starts_with("session-"),
        "session key must start with the `session-` prefix for namespacing"
    );
}

#[test]
fn nil_session_is_still_prefixed() {
    // A session id of all zeros (Uuid::nil()) is unusual but legal.
    // The key must still be prefixed — otherwise we'd hand back an
    // ambiguous "session-00000000..." that downstream could mistake
    // for something else. Pins the prefix invariant even for edge
    // cases.
    let key = BrowserManager::page_name_for_session(Uuid::nil());
    assert!(key.starts_with("session-"));
    assert!(key.ends_with("00000000-0000-0000-0000-000000000000"));
}
