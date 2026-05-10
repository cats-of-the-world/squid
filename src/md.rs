use anyhow::Result;
use chrono::DateTime;
use chrono::Utc;
use gray_matter::engine::YAML;
use gray_matter::Matter;
use std::collections::HashMap;
use std::path::PathBuf;
use tinylang::types::{State, TinyLangType};

#[derive(Debug, Clone)]
pub struct MarkdownCollection {
    pub name: String,
    pub relative_path: PathBuf,
    pub collection: Vec<MarkdownDocument>,
}

impl MarkdownCollection {
    pub fn new(path: PathBuf) -> Self {
        Self {
            name: path.to_string_lossy().to_string(),
            relative_path: path,
            collection: Vec::new(),
        }
    }

    pub fn to_post_metadata(&self) -> Vec<crate::rss::PostMetadata> {
        self.collection
            .iter()
            .filter_map(|doc| doc.to_post_metadata().ok())
            .collect()
    }

    /// collects metadata about the collection and exposes it as TinyLang::State so
    /// it can be used on the templates we are building
    pub fn as_tinylang_state(&self) -> State {
        let mut collection_state = State::new();
        collection_state.insert(
            "size".to_string(),
            TinyLangType::Numeric(self.collection.len() as f64),
        );

        let mut items_state = Vec::new();

        for item in &self.collection {
            items_state.push(TinyLangType::Object(item.as_tinylang_state()));
        }

        collection_state.insert("items".into(), TinyLangType::Vec(items_state));
        collection_state
    }
}

#[derive(Debug, Clone)]
pub struct MarkdownDocument {
    pub header: HashMap<String, String>,
    pub html_content: String,
    pub name: String,
    pub partial_uri: String,
}

impl MarkdownDocument {
    pub fn new(content: &str, name: String, partial_uri: String) -> Result<Self> {
        let matter = Matter::<YAML>::new();
        let header = matter.parse(content);
        let html_content = markdown::to_html(&header.content);

        let header: HashMap<String, String> = match header.data {
            Some(d) => d.deserialize()?,
            None => HashMap::new(),
        };

        Ok(Self {
            header,
            html_content,
            name,
            partial_uri,
        })
    }

    pub fn to_post_metadata(&self) -> Result<crate::rss::PostMetadata> {
        // Parse date from header
        let date = self
            .header
            .get("date")
            .and_then(|d| {
                // Try multiple date formats
                // First try RFC 3339 (e.g., "2024-01-10T10:00:00Z")
                if let Ok(dt) = DateTime::parse_from_rfc3339(d) {
                    return Some(dt.with_timezone(&Utc));
                }

                // Try RFC 2822 (e.g., "Wed, 10 Jan 2024 10:00:00 +0000")
                if let Ok(dt) = DateTime::parse_from_rfc2822(d) {
                    return Some(dt.with_timezone(&Utc));
                }

                // Try simple date format (e.g., "2024-01-10")
                if let Ok(naive_date) = chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d") {
                    if let Some(naive_datetime) = naive_date.and_hms_opt(0, 0, 0) {
                        return Some(DateTime::<Utc>::from_naive_utc_and_offset(
                            naive_datetime,
                            Utc,
                        ));
                    }
                }

                None
            })
            .unwrap_or_else(Utc::now);

        // Get excerpt from header or generate from content
        let excerpt = self
            .header
            .get("excerpt")
            .cloned()
            .or_else(|| self.header.get("description").cloned())
            .unwrap_or_else(|| {
                // Extract first paragraph from HTML content
                let plain_text =
                    html2text::from_read(self.html_content.as_bytes(), 80).unwrap_or_default();
                plain_text
                    .split("\n\n")
                    .next()
                    .unwrap_or("")
                    .chars()
                    .take(150)
                    .collect()
            });

        // Extract tags from header
        let tags = self
            .header
            .get("tags")
            .map(|tags_str| {
                tags_str
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_else(Vec::new);

        Ok(crate::rss::PostMetadata {
            title: self.header.get("title").cloned().unwrap_or_default(),
            file_name: self.partial_uri.clone(),
            date,
            excerpt,
            html_content: self.html_content.clone(),
            author: self.header.get("author").cloned().unwrap_or_default(),
            tags,
        })
    }

    /// collects metadata about the markdown document and exposes it as TinyLang::State so
    /// it can be used on the templates we are building
    pub fn as_tinylang_state(&self) -> State {
        let mut item_state = State::new();
        for (header_key, header_value) in &self.header {
            item_state.insert(header_key.clone(), header_value.clone().into());
        }

        item_state.insert("content".into(), self.html_content.clone().into());

        item_state.insert("partial_uri".to_string(), self.partial_uri.clone().into());
        item_state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_markdown_document() {
        let content = r#"---
title: This is pretty cool
---
# This is an amazing content!"#;

        let markdown =
            MarkdownDocument::new(content, "my_file.md".into(), "/posts/".to_string()).unwrap();

        assert_eq!(
            "<h1>This is an amazing content!</h1>",
            markdown.html_content
        );

        assert_eq!(
            "This is pretty cool",
            markdown.header.get("title").unwrap().as_str()
        );
    }
    #[test]
    fn test_to_post_metadata() {
        let content = r#"---
title: Test Post
date: 2024-01-10T10:00:00Z
author: John Doe
tags: rust, blogging
excerpt: Test excerpt
---
# Test Content"#;

        let markdown = MarkdownDocument::new(
            content,
            "test-post.md".into(),
            "/posts/test-post".to_string(),
        )
        .unwrap();

        let metadata = markdown.to_post_metadata().unwrap();

        assert_eq!(metadata.title, "Test Post");
        assert_eq!(metadata.author, "John Doe");
        assert_eq!(metadata.excerpt, "Test excerpt");
        assert_eq!(metadata.tags, vec!["rust", "blogging"]);
        assert_eq!(metadata.file_name, "/posts/test-post");
    }
}
