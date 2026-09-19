-- Add trigger-gated execution and goal dispatch columns to cron_jobs
ALTER TABLE cron_jobs ADD COLUMN trigger_cmd TEXT;
ALTER TABLE cron_jobs ADD COLUMN trigger_on TEXT DEFAULT 'non_empty';
ALTER TABLE cron_jobs ADD COLUMN set_goal INTEGER NOT NULL DEFAULT 0;
ALTER TABLE cron_jobs ADD COLUMN goal_template TEXT;
