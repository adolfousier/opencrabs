-- Channel messages: passive capture of messages from Telegram groups/channels
-- (and other platforms where history cannot be fetched via API)
CREATE TABLE IF NOT EXISTS channel_messages (
    id TEXT PRIMARY KEY,
    channel TEXT NOT NULL,
    channel_chat_id TEXT NOT NULL,
    channel_chat_name TEXT,
    sender_id TEXT NOT NULL,
    sender_name TEXT NOT NULL,
    content TEXT NOT NULL,
    message_type TEXT NOT NULL DEFAULT 'text',
    platform_message_id TEXT,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_channel_messages_chat ON channel_messages(channel, channel_chat_id);
CREATE INDEX IF NOT EXISTS idx_channel_messages_time ON channel_messages(created_at);
