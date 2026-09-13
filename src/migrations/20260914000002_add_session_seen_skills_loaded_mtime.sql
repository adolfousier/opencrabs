-- Active skill update tracking (issue #210): track mtime of skill file when loaded.
ALTER TABLE session_seen_skills ADD COLUMN loaded_mtime INTEGER;
