//! The `deliver_to` session grammar resolves a target session (fork #144).
//!
//! Cron jobs can now deliver results into a session's notify queue via
//! `session:<uuid|prefix>` — the target is resolved through the shared
//! session-id resolver (`crate::cli::session_resolve`). These tests pin the
//! pure resolution core (`resolve_session_target`) against in-memory session
//! sets: full UUIDs pass through, valid prefixes resolve, garbage and short
//! prefixes reject. The DB-loading wrapper (`parse_session_target`) is a thin
//! async shell over this core — prefix behavior against a live DB is the
//! resolver's own test-suite concern.

use crate::cron::scheduler::resolve_session_target;
use uuid::Uuid;

fn session_with_id(id: Uuid) -> crate::db::models::Session {
    crate::db::models::Session {
        id,
        title: None,
        model: None,
        provider_name: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        archived_at: None,
        token_count: 0,
        total_cost: 0.0,
        working_directory: None,
        auto_title_attempted: false,
        project_id: None,
    }
}

/// A full UUID passes through untouched — the resolver's fast path, no DB.
#[test]
fn full_uuid_resolves_without_db() {
    let id = Uuid::new_v4();
    let resolved = resolve_session_target(&[], &id.to_string()).unwrap();
    assert_eq!(resolved, id);
}

/// A full UUID resolves even when the session set is non-empty and doesn't
/// contain it (fast-path passthrough parity — presence is the caller's check).
#[test]
fn full_uuid_passthrough_ignores_set() {
    let id = Uuid::new_v4();
    let other = session_with_id(Uuid::new_v4());
    let resolved = resolve_session_target(&[other], &id.to_string()).unwrap();
    assert_eq!(resolved, id);
}

/// A valid prefix (8+ chars, exactly one match) resolves to the full id.
#[test]
fn valid_prefix_resolves() {
    let id = Uuid::new_v4();
    let sessions = vec![session_with_id(id)];
    let prefix = id.to_string()[..8].to_string();
    assert_eq!(resolve_session_target(&sessions, &prefix), Some(id));
}

/// A prefix matching no session is `None` (resolver: 0 matches -> Err).
#[test]
fn unknown_prefix_rejects() {
    let sessions = vec![session_with_id(Uuid::new_v4())];
    assert_eq!(resolve_session_target(&sessions, "zzzzzzzz"), None);
}

/// A prefix below the 8-char minimum `session list` displays behaves like any
/// other non-match in a limited set — `None`, not a panic.
#[test]
fn short_prefix_rejects() {
    let id = Uuid::new_v4();
    let sessions = vec![session_with_id(id)];
    let short = id.to_string()[..4].to_string();
    assert_eq!(resolve_session_target(&sessions, &short), None);
}

/// Garbage (not a UUID, not a prefix of anything) is `None`, not a panic.
#[test]
fn garbage_rejects() {
    assert_eq!(
        resolve_session_target(&[], "not-a-uuid-or-prefix-at-all"),
        None
    );
    assert_eq!(resolve_session_target(&[], ""), None);
}

/// The degenerate duplicate-id set still resolves to that id (the resolver's
/// ambiguity arm can only fire on distinct ids sharing a prefix, which 8 hex
/// chars of UUIDv4 never produce in practice — the dup shape pins that
/// same-id rows are not treated as an error).
#[test]
fn duplicate_id_set_resolves() {
    let id = Uuid::new_v4();
    let sessions = vec![session_with_id(id), session_with_id(id)];
    let prefix = id.to_string()[..8].to_string();
    assert_eq!(resolve_session_target(&sessions, &prefix), Some(id));
}
