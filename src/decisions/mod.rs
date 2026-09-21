//! Decision pyramid ring L1: exact decision reuse (#1648).
//!
//! PR1 ships the pure half: canonicalization and key derivation, with the
//! storage ring behind it ([`crate::db::repository::DecisionCacheRepository`]).
//! The `decide_cached` tool, `[decisions]` config and shadow-mode logging
//! land in PR2. This module stays DB-free and model-free on purpose so the
//! identity rules of the cache are unit-testable alone — everything that
//! computes or verifies a key must go through it, so reader and writer can
//! never drift.

pub mod normalize;
