-- Usage ledger: cumulative record of all token/cost usage.
-- Entries are NEVER deleted — even when sessions are removed.
-- This is the source of truth for "total spent" across all time.

CREATE TABLE IF NOT EXISTS usage_ledger (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,        -- which session incurred this usage (informational, not FK)
    model TEXT NOT NULL DEFAULT '',   -- model used
    token_count INTEGER NOT NULL DEFAULT 0,
    cost REAL NOT NULL DEFAULT 0.0,
    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
);

-- Backfill from existing sessions so historical usage isn't lost
INSERT INTO usage_ledger (session_id, model, token_count, cost, created_at)
SELECT id, COALESCE(model, ''), token_count, total_cost, created_at
FROM sessions
WHERE token_count > 0 OR total_cost > 0.0;
