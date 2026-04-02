-- Cron job execution history — stores every run's result in the DB
CREATE TABLE IF NOT EXISTS cron_job_runs (
    id              TEXT PRIMARY KEY NOT NULL,
    job_id          TEXT NOT NULL REFERENCES cron_jobs(id) ON DELETE CASCADE,
    job_name        TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'running',  -- running, success, error
    content         TEXT,
    error           TEXT,
    input_tokens    INTEGER NOT NULL DEFAULT 0,
    output_tokens   INTEGER NOT NULL DEFAULT 0,
    cost            REAL NOT NULL DEFAULT 0.0,
    provider        TEXT,
    model           TEXT,
    started_at      TEXT NOT NULL,
    completed_at    TEXT,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_cron_job_runs_job_id ON cron_job_runs(job_id);
CREATE INDEX IF NOT EXISTS idx_cron_job_runs_started_at ON cron_job_runs(started_at);
