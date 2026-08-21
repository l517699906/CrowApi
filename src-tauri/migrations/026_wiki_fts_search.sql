-- Wiki full-text search index.
-- Page bodies remain managed files; this table is a searchable projection that
-- is synchronised transactionally whenever a page is written or removed.
CREATE VIRTUAL TABLE IF NOT EXISTS wiki_page_search USING fts5(
    page_id UNINDEXED,
    project_id UNINDEXED,
    path,
    title,
    content,
    tags,
    tokenize = 'unicode61 remove_diacritics 2'
);
