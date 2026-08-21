ALTER TABLE api_keys
    ADD COLUMN access_scopes TEXT NOT NULL DEFAULT '["gateway"]'
    CHECK (json_valid(access_scopes) AND json_type(access_scopes) = 'array');

CREATE TABLE IF NOT EXISTS auth_audit_events (
    id           TEXT PRIMARY KEY,
    api_key_id   TEXT,
    api_key_name TEXT,
    method       TEXT NOT NULL,
    path         TEXT NOT NULL,
    origin       TEXT,
    outcome      TEXT NOT NULL,
    error_code   TEXT NOT NULL,
    trace_id     TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    FOREIGN KEY (api_key_id) REFERENCES api_keys(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_auth_audit_created_at
    ON auth_audit_events(created_at DESC);

CREATE INDEX IF NOT EXISTS idx_auth_audit_api_key
    ON auth_audit_events(api_key_id, created_at DESC);
