-- WhatsApp newsletter poll cursors (#1529): a per-channel watermark so the
-- 15-minute poller only surfaces posts newer than what the owner already
-- received. Rows are created lazily on the FIRST observation of an opted-in
-- channel, which baselines silently (see whatsapp::newsletter) — the owner
-- never gets a dump of historical posts for flipping the config on.
--
-- `last_server_id` is the monotonic server-assigned id of the newest digested
-- post (the cursor WhatsApp's own pagination uses); `last_ts` carries that
-- post's Unix seconds. Newness is decided on server_id, ts is kept for the
-- `before` param when a page is ever exhausted.
CREATE TABLE IF NOT EXISTS whatsapp_newsletter_cursors (
    wa_jid         TEXT PRIMARY KEY,
    last_server_id INTEGER NOT NULL DEFAULT 0,
    last_ts        INTEGER NOT NULL DEFAULT 0,
    updated_at     INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
);
