-- Add provider_name to sessions for per-session provider tracking
-- Allows each session to remember which provider it was using
ALTER TABLE sessions ADD COLUMN provider_name TEXT;
