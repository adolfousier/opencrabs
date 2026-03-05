-- Cron jobs: scheduled isolated sessions
CREATE TABLE IF NOT EXISTS cron_jobs (
    id          TEXT PRIMARY KEY NOT NULL,
    name        TEXT NOT NULL,
    cron_expr   TEXT NOT NULL,           -- standard cron expression (e.g. "0 9 * * *")
    timezone    TEXT NOT NULL DEFAULT 'UTC',
    prompt      TEXT NOT NULL,           -- the message/instruction to execute
    provider    TEXT,                    -- override provider (NULL = use default)
    model       TEXT,                    -- override model (NULL = use default)
    thinking    TEXT NOT NULL DEFAULT 'off', -- 'off', 'on', 'budget'
    auto_approve INTEGER NOT NULL DEFAULT 1, -- auto-approve tool calls
    deliver_to  TEXT,                    -- channel to deliver results (e.g. "telegram:123456")
    enabled     INTEGER NOT NULL DEFAULT 1,
    last_run_at TEXT,                    -- ISO 8601 timestamp of last execution
    next_run_at TEXT,                    -- ISO 8601 timestamp of next scheduled run
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
