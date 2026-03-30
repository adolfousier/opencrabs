-- Add channel_chat_id to pending_requests so crash recovery can route
-- responses back to the originating channel (Telegram chat, Discord channel, etc.)
ALTER TABLE pending_requests ADD COLUMN channel_chat_id TEXT;
