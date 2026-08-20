CREATE TABLE IF NOT EXISTS secure_secrets (
    owner_type TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    version INTEGER NOT NULL,
    nonce BLOB NOT NULL,
    ciphertext BLOB NOT NULL,
    last_four TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (owner_type, owner_id)
);

ALTER TABLE channels ADD COLUMN secret_ref TEXT;
ALTER TABLE channels ADD COLUMN api_key_last4 TEXT NOT NULL DEFAULT '';

ALTER TABLE api_keys ADD COLUMN key_lookup TEXT;
ALTER TABLE api_keys ADD COLUMN key_hash TEXT;
ALTER TABLE api_keys ADD COLUMN key_prefix TEXT NOT NULL DEFAULT '';
ALTER TABLE api_keys ADD COLUMN key_last4 TEXT NOT NULL DEFAULT '';

CREATE UNIQUE INDEX IF NOT EXISTS idx_api_keys_key_lookup
    ON api_keys(key_lookup)
    WHERE key_lookup IS NOT NULL;
