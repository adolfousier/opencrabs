-- #1721: stable chat identity on sessions.
--
-- Channel session titles embed a stable `[chat:<id>]` suffix (#121), but
-- identity lives only in the title text: nothing at the schema level stops
-- two live sessions from claiming the same chat. The in-process single-flight
-- gate (#1201/#1228) serializes resolution inside one process; it cannot span
-- processes. Two opencrabs instances sharing one profile database (daemon +
-- TUI, or multiple daemons) can both miss the lookup and both INSERT. This
-- migration adds the identity column loonix's issue asks for, backfills it
-- from existing titles, collapses historical duplicates, and enforces
-- uniqueness at the storage layer so the loser of any future race resolves
-- the winner instead of forking the chat.
--
-- Ordering matters and is not reorderable:
--   1. add the column (NULL = never chat-scoped; the unique index is partial
--      and skips NULLs, so TUI/local sessions are untouched),
--   2. backfill from titles,
--   3. archive duplicate losers (10 of 13 real keys had duplicates, so the
--      unique index would fail to build without this),
--   4. only then create the unique index, partial on LIVE rows: archiving
--      losers clears the live set while preserving chat provenance on the
--      archived rows, and "at most one live session per chat" is exactly
--      the guarantee resolution needs.
-- The survivor per key is the most-recently-updated row, which is exactly
-- the row the title-suffix resolver would have picked (ORDER BY updated_at
-- DESC LIMIT 1), so resolution behaviour is unchanged on migrated data.

ALTER TABLE sessions ADD COLUMN channel_chat_key TEXT;

UPDATE sessions
SET channel_chat_key = substr(title, instr(title, '[chat:'))
WHERE title LIKE '%[chat:%]';

UPDATE sessions
SET archived_at = CAST(strftime('%s', 'now') AS INTEGER)
WHERE channel_chat_key IS NOT NULL
  AND archived_at IS NULL
  AND id NOT IN (
    SELECT id FROM (
      SELECT id, ROW_NUMBER() OVER (
        PARTITION BY channel_chat_key
        ORDER BY updated_at DESC, id DESC
      ) AS rn
      FROM sessions
      WHERE channel_chat_key IS NOT NULL
    ) WHERE rn = 1
  );

CREATE UNIQUE INDEX IF NOT EXISTS idx_sessions_channel_chat_key
  ON sessions(channel_chat_key)
  WHERE channel_chat_key IS NOT NULL AND archived_at IS NULL;
