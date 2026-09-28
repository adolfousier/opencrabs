-- Add the one-shot retirement column to cron_jobs (#544)
ALTER TABLE cron_jobs ADD COLUMN run_once INTEGER NOT NULL DEFAULT 0;
