-- Session seen-skills persistence (issue #138): the in-memory seen_skills
-- registry dies on every daemon restart, so a session that consumed a skill
-- before a rebuild looks "skill-less" to the post-compaction inventory stamp
-- (#125/#131). A row here is written on every mark_seen; boot hydrates the
-- in-memory registry from this table so stamps survive restarts.
--
-- `epoch` is the per-session compaction epoch the skill was seen at (#150):
-- the skill glob gate compares it against the session's current epoch. NULL
-- (a row written before this column existed) reads as epoch 0 — always
-- current, so pre-feature rows stay valid.
CREATE TABLE IF NOT EXISTS session_seen_skills (
    session_id TEXT NOT NULL,
    slug       TEXT NOT NULL,
    seen_at    INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    epoch      INTEGER NULL,
    -- No separate index on session_id: the primary key's implicit index is
    -- already leftmost-prefixed on it, so every per-session lookup and the
    -- orphan prune both ride that one. A second index would serve no read
    -- and cost a write on every mark_seen.
    PRIMARY KEY (session_id, slug)
);
