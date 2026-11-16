-- The numbers a settled turn's card prints, made durable (#2985).
--
-- A settled tool block prints four numbers: the clock, the prompt size, the
-- output rate and the tool count. Only the clock and the prompt size had a
-- column. The rate, the window and the count lived in `Metrics::by_anchor`, a
-- HashMap in the window's own process, so restarting the app dropped them and
-- a settled card degraded to a bare clock.
--
-- `ctx_used` needs no column here: it is the prompt size the provider reported
-- for the turn's final call, which is already stored as `input_tokens`. Only
-- the window that number is measured against, the turn's own output rate and
-- the count of calls it made need a home.
--
-- All three are NULL on rows written before this migration. A reader prints no
-- metrics for those turns, which is the honest answer: the numbers were never
-- recorded, and substituting the session's current ones would misdescribe what
-- the turn cost.
ALTER TABLE messages ADD COLUMN ctx_window INTEGER;
ALTER TABLE messages ADD COLUMN tokens_per_sec REAL;
ALTER TABLE messages ADD COLUMN tool_calls INTEGER;
