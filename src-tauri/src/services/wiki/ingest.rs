mod generator;
mod graph;
mod orchestration;
mod parser;

pub use generator::extract_title_from_content;
pub use graph::{extract_frontmatter, extract_tags_from_frontmatter, rebuild_graph_edges};
pub use orchestration::{
    ingest_source, start_existing_ingest_task, start_ingest_source, INGEST_ALREADY_RUNNING,
};

#[derive(Debug, Clone)]
pub struct IngestResult {
    pub task_id: String,
    pub pages_created: usize,
    pub page_paths: Vec<String>,
}

struct GeneratedPage {
    path: String,
    title: String,
    page_type: String,
    content: String,
}

struct WrittenPage {
    path: String,
    wikilinks: Vec<String>,
}

#[derive(Debug, Clone)]
struct ContentSection {
    heading: String,
    content: String,
}

#[cfg(test)]
mod tests {
    use super::{extract_frontmatter, extract_tags_from_frontmatter};

    #[test]
    fn frontmatter_and_tags_are_preserved() {
        let content = "---\ntitle: Example\ntags: [rust, \"wiki\"]\nsource: notes.md\n---\n# Example";

        assert_eq!(
            extract_frontmatter(content),
            Some("title: Example\ntags: [rust, \"wiki\"]\nsource: notes.md")
        );
        assert_eq!(extract_tags_from_frontmatter(content), vec!["rust", "wiki"]);
    }

    #[test]
    fn text_without_frontmatter_is_not_misclassified() {
        let content = "# Example\n\nA horizontal rule follows.\n\n---\n";

        assert_eq!(extract_frontmatter(content), None);
        assert!(extract_tags_from_frontmatter(content).is_empty());
    }
}
