-- Version knowledge-base content independently from the persisted HNSW snapshot.
ALTER TABLE kb_knowledge_bases
    ADD COLUMN content_revision INTEGER NOT NULL DEFAULT 0 CHECK (content_revision >= 0);

ALTER TABLE kb_index_meta
    ADD COLUMN indexed_revision INTEGER NOT NULL DEFAULT 0 CHECK (indexed_revision >= 0);

-- Existing index files use positional chunk identifiers and cannot prove that
-- they still describe the current database snapshot. Force a safe rebuild.
UPDATE kb_knowledge_bases
SET content_revision = CASE
        WHEN EXISTS (
            SELECT 1 FROM kb_chunks
            WHERE kb_chunks.kb_id = kb_knowledge_bases.id
        ) THEN 1
        ELSE 0
    END,
    index_status = CASE
        WHEN EXISTS (
            SELECT 1 FROM kb_chunks
            WHERE kb_chunks.kb_id = kb_knowledge_bases.id
        ) THEN 'stale'
        ELSE 'none'
    END;

UPDATE kb_index_meta
SET indexed_revision = 0,
    status = CASE
        WHEN EXISTS (
            SELECT 1 FROM kb_chunks
            WHERE kb_chunks.kb_id = kb_index_meta.kb_id
        ) THEN 'stale'
        ELSE 'none'
    END,
    index_path = NULL,
    built_at = NULL;
