-- Unified background task ledger for knowledge, Wiki and future services.
CREATE TABLE IF NOT EXISTS background_tasks (
    id                 TEXT PRIMARY KEY,
    domain             TEXT NOT NULL,
    task_type          TEXT NOT NULL,
    resource_type      TEXT NOT NULL,
    resource_id        TEXT NOT NULL,
    subject_id         TEXT,
    parent_task_id     TEXT,
    idempotency_key    TEXT,
    retry_of           TEXT,
    status             TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'running', 'succeeded', 'failed', 'cancelled', 'interrupted')),
    stage              TEXT NOT NULL DEFAULT 'queued',
    progress           INTEGER NOT NULL DEFAULT 0 CHECK (progress BETWEEN 0 AND 100),
    total_items        INTEGER NOT NULL DEFAULT 0 CHECK (total_items >= 0),
    done_items         INTEGER NOT NULL DEFAULT 0 CHECK (done_items >= 0),
    payload_json       TEXT NOT NULL DEFAULT '{}',
    result_json        TEXT,
    error_message      TEXT,
    retryable          INTEGER NOT NULL DEFAULT 1 CHECK (retryable IN (0, 1)),
    cancel_requested   INTEGER NOT NULL DEFAULT 0 CHECK (cancel_requested IN (0, 1)),
    attempt            INTEGER NOT NULL DEFAULT 1 CHECK (attempt >= 1),
    created_at         TEXT NOT NULL,
    started_at         TEXT,
    updated_at         TEXT NOT NULL,
    completed_at       TEXT,
    FOREIGN KEY (parent_task_id) REFERENCES background_tasks(id) ON DELETE SET NULL,
    FOREIGN KEY (retry_of) REFERENCES background_tasks(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_background_tasks_resource
    ON background_tasks(domain, resource_type, resource_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_background_tasks_status
    ON background_tasks(status, created_at);
CREATE INDEX IF NOT EXISTS idx_background_tasks_retry_of
    ON background_tasks(retry_of);
CREATE INDEX IF NOT EXISTS idx_background_tasks_parent_task
    ON background_tasks(parent_task_id, created_at);
CREATE INDEX IF NOT EXISTS idx_background_tasks_subject
    ON background_tasks(domain, subject_id, created_at DESC);

INSERT INTO background_tasks (
    id, domain, task_type, resource_type, resource_id, subject_id,
    idempotency_key, status, stage, progress, total_items, done_items,
    payload_json, error_message, retryable, cancel_requested, attempt,
    created_at, started_at, updated_at, completed_at
)
SELECT
    id,
    'knowledge',
    task_type,
    'knowledge_base',
    kb_id,
    doc_id,
    CASE
        WHEN doc_id IS NOT NULL THEN 'knowledge:document:' || doc_id
        ELSE 'knowledge:' || task_type || ':' || kb_id
    END,
    CASE status
        WHEN 'completed' THEN 'succeeded'
        WHEN 'done' THEN 'succeeded'
        WHEN 'failed' THEN 'failed'
        WHEN 'pending' THEN 'pending'
        ELSE 'interrupted'
    END,
    CASE
        WHEN status IN ('completed', 'done') THEN 'completed'
        WHEN status = 'failed' THEN 'failed'
        WHEN status = 'pending' THEN 'queued'
        ELSE 'interrupted'
    END,
    progress,
    total_items,
    done_items,
    '{}',
    error_message,
    1,
    0,
    1,
    created_at,
    CASE WHEN status = 'pending' THEN NULL ELSE created_at END,
    COALESCE(completed_at, created_at),
    completed_at
FROM kb_tasks
WHERE 1 = 1
ON CONFLICT(id) DO NOTHING;

INSERT INTO background_tasks (
    id, domain, task_type, resource_type, resource_id, subject_id,
    idempotency_key, status, stage, progress, total_items, done_items,
    payload_json, result_json, error_message, retryable, cancel_requested,
    attempt, created_at, started_at, updated_at, completed_at
)
SELECT
    CASE
        WHEN EXISTS (SELECT 1 FROM background_tasks existing WHERE existing.id = queue.id)
            THEN 'wiki:' || queue.id
        ELSE queue.id
    END,
    'wiki',
    task_type,
    'wiki_project',
    project_id,
    source_id,
    'wiki:' || task_type || ':' || project_id || ':' || COALESCE(source_id, ''),
    CASE status
        WHEN 'completed' THEN 'succeeded'
        WHEN 'done' THEN 'succeeded'
        WHEN 'failed' THEN 'failed'
        WHEN 'pending' THEN 'pending'
        ELSE 'interrupted'
    END,
    CASE
        WHEN status IN ('completed', 'done') THEN 'completed'
        WHEN status = 'failed' THEN 'failed'
        WHEN status = 'pending' THEN 'queued'
        ELSE 'interrupted'
    END,
    progress,
    total_steps,
    done_steps,
    '{}',
    result_json,
    error_message,
    1,
    0,
    1,
    created_at,
    started_at,
    COALESCE(completed_at, started_at, created_at),
    completed_at
FROM wiki_ingest_queue AS queue;

CREATE UNIQUE INDEX IF NOT EXISTS idx_background_tasks_active_key
    ON background_tasks(idempotency_key)
    WHERE idempotency_key IS NOT NULL AND status IN ('pending', 'running');
