-- Add a process-local lease and bounded retry metadata to the durable task
-- ledger. A lease makes abandoned workers detectable without changing the
-- public task status contract; dead-letter tasks remain `failed` but cannot be
-- retried automatically after the configured attempt budget is exhausted.
ALTER TABLE background_tasks
    ADD COLUMN lease_owner TEXT;

ALTER TABLE background_tasks
    ADD COLUMN lease_until TEXT;

ALTER TABLE background_tasks
    ADD COLUMN heartbeat_at TEXT;

ALTER TABLE background_tasks
    ADD COLUMN next_retry_at TEXT;

ALTER TABLE background_tasks
    ADD COLUMN max_attempts INTEGER NOT NULL DEFAULT 5 CHECK (max_attempts >= 1);

ALTER TABLE background_tasks
    ADD COLUMN dead_letter INTEGER NOT NULL DEFAULT 0 CHECK (dead_letter IN (0, 1));

CREATE INDEX IF NOT EXISTS idx_background_tasks_lease
    ON background_tasks(status, lease_until)
    WHERE status = 'running';

CREATE INDEX IF NOT EXISTS idx_background_tasks_retry_schedule
    ON background_tasks(status, next_retry_at, auto_resume)
    WHERE status = 'failed' AND dead_letter = 0;
