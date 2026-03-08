-- Pending requests — track in-flight agent requests for restart recovery.
-- Rows exist only while a request is being processed. On completion they
-- are deleted. Any rows left on startup indicate a crash/restart mid-request
-- and are replayed automatically.
CREATE TABLE IF NOT EXISTS pending_requests (
    id           TEXT PRIMARY KEY,
    session_id   TEXT NOT NULL,
    user_message TEXT NOT NULL,
    channel      TEXT NOT NULL DEFAULT 'tui',
    status       TEXT NOT NULL DEFAULT 'PROCESSING',
    created_at   INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at   INTEGER NOT NULL DEFAULT (unixepoch())
);
