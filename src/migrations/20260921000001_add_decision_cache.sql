-- L1 exact-decision-reuse ring for the decision pyramid (#1648).
--
-- Classification-shaped decisions (triage, routing, voice gates, draft
-- scoring, self-audit) are re-paid for in full model price even when the
-- input is an exact repeat. OpenCrabs has L0 rules/code-graph, L2
-- embeddings/FTS, L3 sub-agents, L4 prompt-cached frontier — L1 exact
-- decision reuse is the missing ring (the Store rewrite dropped llm_cache,
-- src/memory/db.rs:9).
--
-- One row per (tier, policy, normalizer, canonicalized input) identity:
-- `key` is sha256 over those four parts joined by unit separators
-- (crate::decisions::normalize::decision_key). `p` is the model's reported
-- confidence when it gave one, `margin` the runner-up distance; a row whose
-- margin sits below the tier's floor is refused at write time so
-- borderline decisions are never frozen. `hits` / `last_used_at` feed the
-- /usage accounting and TTL pruning.
--
-- Removal (release-day kill switch, decided per Adolfo 2026-09-21): set
-- [decisions] mode=off (or drop the section) so no code path reads or
-- writes this table, then
--   DROP TABLE IF EXISTS decision_cache;
-- Nothing else references it.
CREATE TABLE IF NOT EXISTS decision_cache (
    key                TEXT PRIMARY KEY NOT NULL,
    tier_id            TEXT NOT NULL,
    result_json        TEXT NOT NULL,
    p                  REAL,
    margin             REAL,
    policy_version     TEXT NOT NULL,
    normalizer_version TEXT NOT NULL,
    created_at         INTEGER NOT NULL,
    hits               INTEGER NOT NULL DEFAULT 0,
    last_used_at       INTEGER
);

-- Tier-scoped deletes (policy_version bumps, kill switch) and per-tier
-- reporting both filter on this pair.
CREATE INDEX IF NOT EXISTS idx_decision_cache_tier
    ON decision_cache(tier_id, policy_version);
