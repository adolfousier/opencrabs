-- Per-tier decision-cache counters (#1648, PR2).
--
-- The evaluation surface for the L1 reuse ring: `calls` counts every
-- decide_cached ask, `would_hit` counts the shadow-mode observations where
-- an identical (tier, policy_version, normalizer_version, canonical input)
-- key was already cached, and `live_hit` counts real cache reuses once a
-- tier is promoted to mode=live. Release-day keep-or-cut reads these
-- numbers off this table; they must survive restarts, so they live in the
-- DB, not in a log line.
--
-- Idempotent CREATE: a heal pass on stamp drift is unnecessary (same
-- rationale as the decision_cache entry). Removal is one command:
-- DROP TABLE IF EXISTS decision_stats.
CREATE TABLE IF NOT EXISTS decision_stats (
    tier_id      TEXT PRIMARY KEY NOT NULL,
    calls        INTEGER NOT NULL DEFAULT 0,
    would_hit    INTEGER NOT NULL DEFAULT 0,
    live_hit     INTEGER NOT NULL DEFAULT 0,
    last_seen_at INTEGER NOT NULL
);
