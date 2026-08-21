use super::{ContentSection, GeneratedPage};
use crate::core::proxy;
use crate::db::repository::Repository;
use std::collections::HashSet;
use std::sync::Arc;
use tauri::AppHandle;

/// Generate wiki pages from content sections via LLM.
pub(super) async fn generate_wiki_pages(
    app: &AppHandle,
    db_repo: &Arc<Repository>,
    model: &str,
    channel_id: &str,
    project_id: &str,
    source_filename: &str,
    sections: &[ContentSection],
    schema: &str,
) -> Result<Vec<GeneratedPage>, String> {
    // Build a combined context from all sections
    let combined: String = sections.iter()
        .map(|s| format!("## {}\n{}", s.heading, s.content))
        .collect::<Vec<_>>()
        .join("\n\n");

    // Truncate to fit context window (rough estimate)
    let max_chars = 24000;
    let truncated = if combined.len() > max_chars {
        let mut t = combined[..max_chars].to_string();
        t.push_str("\n\n[... content truncated ...]");
        t
    } else {
        combined
    };

    let system_prompt = format!(
        r#"You are a Wiki maintainer. Read the source document and generate structured wiki pages in Markdown.

## Wiki Schema
{}

## Instructions
1. Analyze the source document and identify key entities, concepts, and topics.
2. For each key item, generate a wiki page in Markdown format.
3. Use `[[wikilinks]]` to connect related pages.
4. Each page should have YAML frontmatter with: title, type (entity/concept/summary), tags, source.
5. Separate pages with a delimiter: ---PAGE---
6. The first line of each page should be the file path (e.g., `entities/my-item.md`).

## Output Format
```
entities/page-name.md
---
title: Page Name
type: entity
tags: [tag1, tag2]
source: {}
---
# Page Name

Content here with [[wikilinks]] to other pages.

## Details
...
---PAGE---
concepts/another-concept.md
---
title: Another Concept
type: concept
tags: [tag1]
source: {}
---
# Another Concept
...
```

Generate 3-8 pages depending on document complexity. Focus on the most important entities and concepts."#,
        schema, source_filename, source_filename
    );

    let user_prompt = format!("Source document: {}\n\nContent:\n{}", source_filename, truncated);

    let chat_request = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": user_prompt}
        ],
        "stream": false,
        "temperature": 0.3
    });

    let chat_request_str: String = serde_json::to_string(&chat_request).unwrap_or_default();

    let proxy_result = proxy::handle_request(
        db_repo,
        app,
        channel_id,
        "Wiki Ingest",
        chat_request,
        false,
        "chat",
        Some(chat_request_str),
        Some(format!("wiki-ingest_{}", project_id)),
        None,
    ).await;

    let response_body = match proxy_result {
        Ok(result) => result.body,
        Err((code, msg)) => {
            return Err(format!("LLM request failed ({}): {}", code, msg));
        }
    };

    // Extract text from response
    let raw_text = response_body
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|t| t.as_str())
        .unwrap_or("");

    if raw_text.is_empty() {
        // Fallback: create a single summary page from the source
        return Ok(vec![GeneratedPage {
            path: format!("summaries/{}.md", sanitize_filename(source_filename)),
            title: source_filename.to_string(),
            page_type: "summary".to_string(),
            content: format!(
                "---\ntitle: {}\ntype: summary\ntags: []\nsource: {}\n---\n\n# {}\n\n{}",
                source_filename, source_filename, source_filename, truncated
            ),
        }]);
    }

    // Parse the LLM output into pages
    Ok(parse_generated_pages(raw_text, source_filename))
}

/// Parse LLM-generated pages from the response text.
fn parse_generated_pages(text: &str, source_filename: &str) -> Vec<GeneratedPage> {
    let mut pages = Vec::new();
    let mut current_path = String::new();
    let mut current_content = String::new();
    let mut in_content = false;

    let mut lines = text.lines().peekable();

    // Skip any preamble before first path
    while let Some(line) = lines.peek() {
        let trimmed = line.trim();
        if trimmed.ends_with(".md") || (trimmed.contains('/') && trimmed.ends_with(".md")) {
            break;
        }
        // Also check for path-like patterns
        if trimmed.contains(".md") && !trimmed.starts_with("#") {
            break;
        }
        lines.next();
    }

    for line in text.lines() {
        let trimmed = line.trim();

        // Check for page delimiter
        if trimmed == "---PAGE---" {
            if !current_content.is_empty() {
                if let Some(path) = extract_path_from_content(&current_content) {
                    pages.push(build_page(&path, &current_content, source_filename));
                }
            }
            current_content.clear();
            current_path.clear();
            in_content = false;
            continue;
        }

        // Check if line looks like a file path (ends with .md and has no spaces in path part)
        if !in_content && (trimmed.ends_with(".md") && trimmed.len() < 200) {
            current_path = trimmed.to_string();
            in_content = true;
            continue;
        }

        if in_content {
            current_content.push_str(line);
            current_content.push('\n');
        }
    }

    // Handle last page
    if !current_content.is_empty() {
        if current_path.is_empty() {
            // Try to extract path from content
            if let Some(path) = extract_path_from_content(&current_content) {
                current_path = path;
            }
        }
        if !current_path.is_empty() {
            pages.push(build_page(&current_path, &current_content, source_filename));
        } else if let Some(page) = build_page_from_content(&current_content) {
            pages.push(page);
        }
    }

    // Deduplicate by path
    let mut seen = HashSet::new();
    pages.retain(|p| {
        if seen.contains(&p.path) {
            false
        } else {
            seen.insert(p.path.clone());
            true
        }
    });

    if pages.is_empty() {
        // Fallback: create a summary page
        pages.push(GeneratedPage {
            path: format!("summaries/{}.md", sanitize_filename(source_filename)),
            title: source_filename.to_string(),
            page_type: "summary".to_string(),
            content: format!(
                "---\ntitle: {}\ntype: summary\ntags: []\nsource: {}\n---\n\n# {}\n\n{}",
                source_filename, source_filename, source_filename,
                text.chars().take(8000).collect::<String>()
            ),
        });
    }

    pages
}

fn extract_path_from_content(content: &str) -> Option<String> {
    // Look for first line that looks like a path
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.ends_with(".md") && trimmed.len() < 200 {
            // Clean up markdown code fences
            let clean = trimmed.trim_start_matches("```").trim_end_matches("```").trim();
            if clean.ends_with(".md") {
                return Some(clean.to_string());
            }
        }
    }
    None
}

fn build_page(path: &str, raw_content: &str, source_filename: &str) -> GeneratedPage {
    // Remove path line from content if present
    let content = raw_content.lines()
        .filter(|l| {
            let t = l.trim();
            !(t.ends_with(".md") && t.len() < 200)
        })
        .collect::<Vec<_>>()
        .join("\n");

    let content = content.trim();

    // Extract title from frontmatter or first heading
    let title = extract_title_from_content(content, path);

    // Determine page type from frontmatter or path
    let page_type = if path.starts_with("entities/") {
        "entity"
    } else if path.starts_with("concepts/") {
        "concept"
    } else if path.starts_with("summaries/") {
        "summary"
    } else if path.ends_with("index.md") {
        "index"
    } else if path.ends_with("log.md") {
        "log"
    } else {
        "entity"
    };

    // Ensure content has frontmatter
    let final_content = if content.starts_with("---") {
        content.to_string()
    } else {
        format!(
            "---\ntitle: {}\ntype: {}\ntags: []\nsource: {}\n---\n\n{}",
            title, page_type, source_filename, content
        )
    };

    GeneratedPage {
        path: path.to_string(),
        title,
        page_type: page_type.to_string(),
        content: final_content,
    }
}

fn build_page_from_content(content: &str) -> Option<GeneratedPage> {
    let title = extract_title_from_content(content, "");
    if title.is_empty() {
        return None;
    }
    let path = format!("entities/{}.md", sanitize_filename(&title));
    Some(GeneratedPage {
        path,
        title,
        page_type: "entity".to_string(),
        content: content.to_string(),
    })
}

pub fn extract_title_from_content(content: &str, fallback_path: &str) -> String {
    // Try frontmatter title
    if content.starts_with("---") {
        let end = content[3..].find("---");
        if let Some(e) = end {
            let frontmatter = &content[3..3 + e];
            for line in frontmatter.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("title:") {
                    let title = trimmed[6..].trim().trim_matches('"').trim_matches('\'');
                    if !title.is_empty() {
                        return title.to_string();
                    }
                }
            }
        }
    }
    // Try first heading
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("# ") {
            return trimmed[2..].trim().to_string();
        }
        if trimmed.starts_with("## ") {
            return trimmed[3..].trim().to_string();
        }
    }
    // Fallback to filename from path
    if !fallback_path.is_empty() {
        return fallback_path
            .split('/')
            .last()
            .unwrap_or(fallback_path)
            .trim_end_matches(".md")
            .to_string();
    }
    "Untitled".to_string()
}

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}
