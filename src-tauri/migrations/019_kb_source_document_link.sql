-- Link imported knowledge-base documents to the source job that created them.
-- Nullable keeps existing documents valid; historical rows cannot be mapped
-- safely because older imports did not persist the source id.
ALTER TABLE kb_documents ADD COLUMN source_id TEXT;

CREATE INDEX IF NOT EXISTS idx_documents_source_id
    ON kb_documents(source_id);
