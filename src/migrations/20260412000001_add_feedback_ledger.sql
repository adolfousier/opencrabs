-- Feedback Ledger: append-only observations for recursive self-improvement.
-- Records tool outcomes, user corrections, provider errors, and agent
-- performance signals. Never deleted — compacted by periodic analysis.

CREATE TABLE IF NOT EXISTS feedback_ledger (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id      TEXT    NOT NULL,
    event_type      TEXT    NOT NULL,   -- 'tool_success', 'tool_failure', 'user_correction', 'provider_error', 'context_compaction', 'improvement_applied'
    dimension       TEXT    NOT NULL,   -- what was observed: tool name, provider name, etc.
    value           REAL    NOT NULL DEFAULT 1.0,  -- numeric signal (1.0 = success, 0.0 = failure, duration_ms, etc.)
    metadata        TEXT,               -- JSON blob with extra context
    created_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_feedback_ledger_session    ON feedback_ledger(session_id);
CREATE INDEX IF NOT EXISTS idx_feedback_ledger_event_type ON feedback_ledger(event_type);
CREATE INDEX IF NOT EXISTS idx_feedback_ledger_dimension  ON feedback_ledger(dimension);
CREATE INDEX IF NOT EXISTS idx_feedback_ledger_created    ON feedback_ledger(created_at DESC);
