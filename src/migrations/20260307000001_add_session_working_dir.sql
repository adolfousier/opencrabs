-- Add working_directory column to sessions table
-- Persists the last /cd directory per session so it survives restarts and switches.
ALTER TABLE sessions ADD COLUMN working_directory TEXT;
