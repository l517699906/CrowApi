use super::ContentSection;

/// Parse content into sections based on file type.
pub(super) fn parse_content(content: &str, file_ext: &str) -> Vec<ContentSection> {
    match file_ext.as_ref() {
        "md" | "markdown" => parse_markdown(content),
        "txt" => parse_plain_text(content),
        "json" => parse_json(content),
        _ => parse_plain_text(content),
    }
}

fn parse_markdown(content: &str) -> Vec<ContentSection> {
    let mut sections = Vec::new();
    let mut current_heading = String::from("Overview");
    let mut current_content = String::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("# ") {
            if !current_content.is_empty() {
                sections.push(ContentSection {
                    heading: current_heading.clone(),
                    content: current_content.clone(),
                });
                current_content.clear();
            }
            current_heading = trimmed[2..].to_string();
        } else if trimmed.starts_with("## ") {
            if !current_content.is_empty() {
                sections.push(ContentSection {
                    heading: current_heading.clone(),
                    content: current_content.clone(),
                });
                current_content.clear();
            }
            current_heading = trimmed[3..].to_string();
        } else {
            current_content.push_str(line);
            current_content.push('\n');
        }
    }

    if !current_content.is_empty() {
        sections.push(ContentSection {
            heading: current_heading,
            content: current_content,
        });
    }

    // If only one small section, split by paragraphs
    if sections.len() <= 1 && content.len() > 2000 {
        return parse_plain_text(content);
    }

    if sections.is_empty() {
        sections.push(ContentSection {
            heading: "Document".to_string(),
            content: content.to_string(),
        });
    }

    sections
}

fn parse_plain_text(content: &str) -> Vec<ContentSection> {
    let mut sections = Vec::new();
    let mut chunk = String::new();
    let mut chunk_idx = 1;
    let max_chunk = 3000;

    for line in content.lines() {
        if chunk.len() + line.len() > max_chunk {
            if !chunk.is_empty() {
                sections.push(ContentSection {
                    heading: format!("Section {}", chunk_idx),
                    content: chunk.clone(),
                });
                chunk_idx += 1;
                chunk.clear();
            }
        }
        chunk.push_str(line);
        chunk.push('\n');
    }

    if !chunk.is_empty() {
        sections.push(ContentSection {
            heading: format!("Section {}", chunk_idx),
            content: chunk,
        });
    }

    if sections.is_empty() {
        sections.push(ContentSection {
            heading: "Document".to_string(),
            content: content.to_string(),
        });
    }

    sections
}

fn parse_json(content: &str) -> Vec<ContentSection> {
    // Try to parse and flatten JSON into readable sections
    match serde_json::from_str::<serde_json::Value>(content) {
        Ok(json) => {
            let pretty = serde_json::to_string_pretty(&json).unwrap_or_else(|_| content.to_string());
            parse_plain_text(&pretty)
        }
        Err(_) => parse_plain_text(content),
    }
}
