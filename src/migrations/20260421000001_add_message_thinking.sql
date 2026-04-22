-- Add `thinking` column to messages table for persisting reasoning/thinking
-- content separately from the main response content.
--
-- Non-CLI providers (dialagram, custom OpenAI-compatible) extract reasoning
-- via `reasoning_content` or inline `<think/>` tags during streaming and
-- display it in the TUI as expandable Ctrl+O thinking blocks. But the
-- reasoning was never persisted to DB — on restart it was lost entirely.
--
-- CLI providers already store reasoning wrapped in `<!-- reasoning -->` markers
-- inside the `content` column and extract it via `extract_reasoning()` on reload.
-- Non-CLI providers intentionally skip that to avoid teaching models to echo
-- reasoning markers. This column gives non-CLI providers a clean separate slot.

ALTER TABLE messages ADD COLUMN thinking TEXT;
