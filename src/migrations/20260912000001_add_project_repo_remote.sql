-- #1510: give projects an identity stronger than their name.
--
-- `projects` carried only a name, so the session-to-project rule could only
-- compare directory basenames. Two unrelated directories sharing a basename
-- (a benchmark checkout under the canonical repo's name) collapsed onto one
-- project, and a checkout whose directory name differs from the project was
-- unreachable. `repo_remote` stores the normalized origin remote of the
-- repository a session proved it belongs to; NULL until then. Adoption only:
-- writes never overwrite a recorded remote.
ALTER TABLE projects ADD COLUMN repo_remote TEXT;

CREATE INDEX IF NOT EXISTS idx_projects_repo_remote ON projects(repo_remote);
