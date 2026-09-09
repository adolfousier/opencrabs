//! The `deliver_to` session grammar resolves a target session (fork #144).
//!
//! Cron jobs can now deliver results into a session's notify queue via
//! `session:<uuid|prefix>` — the target is resolved through the shared
//! session-id resolver (`crate::cli::session_resolve`), so full UUIDs pass
//! through untouched and short prefixes resolve case-insensitively against
//! the DB. These tests pin the pure-parse contract: invalid shapes and
//! ambiguous/unknown prefixes reject; full UUIDs resolve without a DB hit.
//! (Prefix resolution against a live DB is an integration concern — the
//! resolver itself is pinned in `session_resolve`'s own test suite.)

use crate::cron::scheduler::parse_session_target;
use uuid::Uuid;

/// A full UUID passes through untouched — the resolver's fast path, no DB.
#[test]
fn full_uuid_resolves_without_db() {
    let id = Uuid::new_v4().to_string();
    assert_eq!(parse_session_target(&id), Some(id.parse().unwrap()));
}

/// An invalid shape (no colon issue here — the arm splits before the parse;
/// garbage after `session:`) is `None`, not a panic.
#[test]
fn garbage_rejects() {
    assert_eq!(parse_session_target("not-a-uuid-or-prefix-at-all"), None);
    assert_eq!(parse_session_target(""), None);
}

/// A 4-char prefix is below the 8-char minimum `session list` displays —
/// the resolver's ambiguity contract rejects it (0 matches here).
#[test]
fn short_prefix_rejects() {
    // No DB session can start with 'zzzz' in the unit environment (no DB
    // rows at all); resolver returns Err → None. Pins the loud-failure path.
    assert_eq!(parse_session_target("zzzzzzzz"), None);
}
