-- Bind processed documents and HNSW snapshots to the knowledge-base settings
-- that produced them. This prevents an old embedding/chunking task from
-- becoming current after the settings have changed.
ALTER TABLE kb_knowledge_bases
    ADD COLUMN config_revision INTEGER NOT NULL DEFAULT 1;

ALTER TABLE kb_documents
    ADD COLUMN processed_config_revision INTEGER NOT NULL DEFAULT 0;

UPDATE kb_documents
SET processed_config_revision = COALESCE(
    (SELECT config_revision
     FROM kb_knowledge_bases
     WHERE kb_knowledge_bases.id = kb_documents.kb_id),
    1
)
WHERE status = 'ready';

ALTER TABLE kb_index_meta
    ADD COLUMN format_version INTEGER NOT NULL DEFAULT 0;

ALTER TABLE kb_index_meta
    ADD COLUMN config_revision INTEGER NOT NULL DEFAULT 0;

ALTER TABLE kb_index_meta
    ADD COLUMN content_fingerprint TEXT;

ALTER TABLE kb_index_meta
    ADD COLUMN index_checksum TEXT;

-- Existing bincode files have no format marker or checksum. Keep their
-- database chunks searchable through the linear fallback, but force the file
-- index to be rebuilt before it can become active again.
UPDATE kb_index_meta
SET status = CASE WHEN chunk_count > 0 THEN 'stale' ELSE 'none' END,
    format_version = 0,
    config_revision = 0,
    content_fingerprint = NULL,
    index_checksum = NULL;

UPDATE kb_knowledge_bases
SET index_status = CASE WHEN chunk_count > 0 THEN 'stale' ELSE 'none' END
WHERE index_status = 'ready';

CREATE INDEX IF NOT EXISTS idx_kb_documents_config_revision
    ON kb_documents(kb_id, processed_config_revision, status);
