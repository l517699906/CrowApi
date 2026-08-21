ALTER TABLE secure_secrets
    ADD COLUMN key_version INTEGER NOT NULL DEFAULT 1 CHECK (key_version > 0);

CREATE INDEX IF NOT EXISTS idx_secure_secrets_key_version
    ON secure_secrets(key_version);

CREATE TABLE IF NOT EXISTS secret_store_metadata (
    singleton          INTEGER PRIMARY KEY CHECK (singleton = 1),
    active_key_version INTEGER NOT NULL CHECK (active_key_version > 0),
    updated_at         TEXT NOT NULL
);

INSERT OR IGNORE INTO secret_store_metadata (singleton, active_key_version, updated_at)
VALUES (1, 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
