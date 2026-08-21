ALTER TABLE background_tasks
    ADD COLUMN auto_resume INTEGER NOT NULL DEFAULT 0 CHECK (auto_resume IN (0, 1));

CREATE INDEX IF NOT EXISTS idx_background_tasks_auto_resume
    ON background_tasks(status, auto_resume, created_at)
    WHERE auto_resume = 1;
