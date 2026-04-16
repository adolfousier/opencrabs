-- Add `input_tokens` to the messages table so we can persist the
-- server-reported prompt token count on each assistant response.
-- This replaces the in-memory `session_context_cache` HashMap that used
-- to mirror this value and died on every process restart.

ALTER TABLE messages ADD COLUMN input_tokens INTEGER;
