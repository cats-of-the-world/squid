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

    /// Sort the collection newest-first by frontmatter date (undated documents
    /// go last, ties broken by file name) and link each document to its
    /// neighbors so templates can render next/previous post navigation.
    pub fn sort_and_link(&mut self) {
        self.collection.sort_by(|a, b| match (a.date(), b.date()) {
            (Some(da), Some(db)) => db.cmp(&da).then_with(|| a.name.cmp(&b.name)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.name.cmp(&b.name),
        });

        let refs: Vec<Neighbor> = self
            .collection
            .iter()
            .map(|doc| Neighbor {
                title: doc.header.get("title").cloned().unwrap_or_default(),
                partial_uri: doc.partial_uri.clone(),
            })
            .collect();

        for (i, doc) in self.collection.iter_mut().enumerate() {
            // newest-first order: "next" is the newer post, "previous" the older one
            doc.next = if i > 0 {
                Some(refs[i - 1].clone())
            } else {
                None
            };
            doc.previous = refs.get(i + 1).cloned();
        }
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

/// A link to an adjacent document in a sorted collection.
#[derive(Debug, Clone)]
pub struct Neighbor {
    pub title: String,
    pub partial_uri: String,
}

impl Neighbor {
    fn as_tinylang_state(&self) -> State {
        let mut state = State::new();
        state.insert("title".into(), self.title.clone().into());
        state.insert("partial_uri".into(), self.partial_uri.clone().into());
        state
    }
}

#[derive(Debug, Clone)]
pub struct MarkdownDocument {
    pub header: HashMap<String, String>,
    pub html_content: String,
    pub name: String,
    pub partial_uri: String,
    /// the newer adjacent document, set by MarkdownCollection::sort_and_link
    pub next: Option<Neighbor>,
    /// the older adjacent document, set by MarkdownCollection::sort_and_link
    pub previous: Option<Neighbor>,
}

/// Parse a date in any of the formats accepted in frontmatter:
/// RFC 3339 ("2024-01-10T10:00:00Z"), RFC 2822 ("Wed, 10 Jan 2024 10:00:00 +0000")
/// or a simple date ("2024-01-10", taken as midnight UTC).
pub(crate) fn parse_date(d: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(d) {
        return Some(dt.with_timezone(&Utc));
    }

    if let Ok(dt) = DateTime::parse_from_rfc2822(d) {
        return Some(dt.with_timezone(&Utc));
    }

    if let Ok(naive_date) = chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d") {
        if let Some(naive_datetime) = naive_date.and_hms_opt(0, 0, 0) {
            return Some(DateTime::<Utc>::from_naive_utc_and_offset(
                naive_datetime,
                Utc,
            ));
        }
    }

    None
}

/// Render markdown as GitHub Flavored Markdown (tables, strikethrough,
/// footnotes, task lists). Raw HTML is passed through, matching what users
/// of other static site generators expect from their markdown.
fn to_html(content: &str) -> String {
    let options = markdown::Options {
        parse: markdown::ParseOptions::gfm(),
        compile: markdown::CompileOptions {
            allow_dangerous_html: true,
            ..markdown::CompileOptions::gfm()
        },
    };
    // to_html_with_options only fails for MDX inputs, which GFM parsing
    // never produces; fall back to the plain renderer just in case.
    markdown::to_html_with_options(content, &options).unwrap_or_else(|_| markdown::to_html(content))
}

impl MarkdownDocument {
    pub fn new(content: &str, name: String, partial_uri: String) -> Result<Self> {
        let matter = Matter::<YAML>::new();
        let header = matter.parse(content);
        let html_content = to_html(&header.content);

        // Deserialize into JSON values first: frontmatter like `draft: true`
        // or `weight: 3` is a YAML bool/number and would fail a direct
        // HashMap<String, String> deserialization, rejecting the whole file.
        let header: HashMap<String, String> = match header.data {
            Some(d) => {
                let raw: HashMap<String, serde_json::Value> = d.deserialize()?;
                raw.into_iter()
                    .map(|(k, v)| match v {
                        serde_json::Value::String(s) => (k, s),
                        other => (k, other.to_string()),
                    })
                    .collect()
            }
            None => HashMap::new(),
        };

        Ok(Self {
            header,
            html_content,
            name,
            partial_uri,
            next: None,
            previous: None,
        })
    }

    /// Highlight fenced code blocks in the rendered HTML with the given
    /// syntect theme. See crate::highlight for the accepted theme names.
    pub fn highlight_code(&mut self, theme: &str) {
        self.html_content = crate::highlight::highlight_code_blocks(&self.html_content, theme);
    }

    /// Parsed `date` frontmatter, if present and in a supported format
    /// (RFC 3339, RFC 2822 or YYYY-MM-DD).
    pub fn date(&self) -> Option<DateTime<Utc>> {
        self.header.get("date").and_then(|d| parse_date(d))
    }

    /// A document is a draft when its frontmatter says `draft: true` or its
    /// date is in the future. Drafts are excluded from builds unless the user
    /// opts in (e.g. for local preview).
    pub fn is_draft(&self) -> bool {
        let drafted = self
            .header
            .get("draft")
            .map(|d| d.trim().eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        drafted || self.is_scheduled()
    }

    /// Whether the document's date is in the future. Date-only values are
    /// compared as local calendar dates, so a post dated today is published
    /// immediately even when the local timezone is ahead of UTC.
    fn is_scheduled(&self) -> bool {
        let Some(raw) = self.header.get("date") else {
            return false;
        };
        if let Ok(d) = chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
            return d > chrono::Local::now().date_naive();
        }
        parse_date(raw).map(|d| d > Utc::now()).unwrap_or(false)
    }

    /// Tags from frontmatter. Accepts a comma-separated string
    /// (`tags: rust, web`) or a YAML list (`tags: [rust, web]`), which the
    /// header normalization stores as a JSON array string.
    pub fn tags(&self) -> Vec<String> {
        let Some(raw) = self.header.get("tags") else {
            return Vec::new();
        };

        if let Ok(serde_json::Value::Array(values)) = serde_json::from_str(raw) {
            return values
                .into_iter()
                .map(|v| match v {
                    serde_json::Value::String(s) => s,
                    other => other.to_string(),
                })
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }

        raw.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    pub fn to_post_metadata(&self) -> Result<crate::rss::PostMetadata> {
        let date = self.date().unwrap_or_else(Utc::now);

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

        let tags = self.tags();

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

        let neighbor_state = |n: &Option<Neighbor>| match n {
            Some(n) => TinyLangType::Object(n.as_tinylang_state()),
            None => TinyLangType::Nil,
        };
        item_state.insert("next".into(), neighbor_state(&self.next));
        item_state.insert("previous".into(), neighbor_state(&self.previous));
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

    fn make_doc(name: &str, date: Option<&str>) -> MarkdownDocument {
        let content = match date {
            Some(d) => format!("---\ntitle: {name}\ndate: {d}\n---\nBody"),
            None => format!("---\ntitle: {name}\n---\nBody"),
        };
        MarkdownDocument::new(
            &content,
            format!("{name}.md"),
            format!("/posts/{name}.html"),
        )
        .unwrap()
    }

    #[test]
    fn test_sort_and_link_orders_newest_first() {
        let mut coll = MarkdownCollection::new(PathBuf::from("posts"));
        coll.collection.push(make_doc("old", Some("2023-01-01")));
        coll.collection.push(make_doc("new", Some("2024-06-01")));
        coll.collection.push(make_doc("mid", Some("2024-01-01")));
        coll.sort_and_link();

        let names: Vec<&str> = coll.collection.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["new.md", "mid.md", "old.md"]);
    }

    #[test]
    fn test_sort_and_link_undated_goes_last() {
        let mut coll = MarkdownCollection::new(PathBuf::from("posts"));
        coll.collection.push(make_doc("undated", None));
        coll.collection.push(make_doc("dated", Some("2024-01-01")));
        coll.sort_and_link();

        assert_eq!(coll.collection[0].name, "dated.md");
        assert_eq!(coll.collection[1].name, "undated.md");
    }

    #[test]
    fn test_sort_and_link_sets_neighbors() {
        let mut coll = MarkdownCollection::new(PathBuf::from("posts"));
        coll.collection.push(make_doc("old", Some("2023-01-01")));
        coll.collection.push(make_doc("new", Some("2024-01-01")));
        coll.sort_and_link();

        // newest first: [new, old]
        let newest = &coll.collection[0];
        assert!(newest.next.is_none());
        assert_eq!(newest.previous.as_ref().unwrap().title, "old");

        let oldest = &coll.collection[1];
        assert_eq!(oldest.next.as_ref().unwrap().title, "new");
        assert!(oldest.previous.is_none());

        let state = oldest.as_tinylang_state();
        assert!(matches!(state.get("next"), Some(TinyLangType::Object(_))));
        assert!(matches!(state.get("previous"), Some(TinyLangType::Nil)));
    }

    #[test]
    fn test_boolean_frontmatter_does_not_break_parsing() {
        let content = "---\ntitle: Post\ndraft: true\nweight: 3\n---\nBody";
        let doc = MarkdownDocument::new(content, "d.md".into(), "/d".into()).unwrap();
        assert_eq!(doc.header.get("draft").unwrap(), "true");
        assert_eq!(doc.header.get("weight").unwrap(), "3");
    }

    #[test]
    fn test_is_draft_flag() {
        let content = "---\ntitle: Post\ndraft: true\n---\nBody";
        let doc = MarkdownDocument::new(content, "d.md".into(), "/d".into()).unwrap();
        assert!(doc.is_draft());
    }

    #[test]
    fn test_is_draft_future_date() {
        let content = "---\ntitle: Post\ndate: 2999-01-01\n---\nBody";
        let doc = MarkdownDocument::new(content, "f.md".into(), "/f".into()).unwrap();
        assert!(doc.is_draft());
    }

    #[test]
    fn test_post_dated_today_is_not_draft() {
        let today = chrono::Local::now().format("%Y-%m-%d");
        let content = format!("---\ntitle: Post\ndate: {today}\n---\nBody");
        let doc = MarkdownDocument::new(&content, "t.md".into(), "/t".into()).unwrap();
        assert!(!doc.is_draft());
    }

    #[test]
    fn test_is_not_draft_by_default() {
        let content = "---\ntitle: Post\ndate: 2024-01-01\n---\nBody";
        let doc = MarkdownDocument::new(content, "p.md".into(), "/p".into()).unwrap();
        assert!(!doc.is_draft());
    }

    #[test]
    fn test_gfm_table_renders() {
        let content = "| a | b |\n| - | - |\n| 1 | 2 |";
        let doc = MarkdownDocument::new(content, "t.md".into(), "/t".into()).unwrap();
        assert!(doc.html_content.contains("<table>"), "{}", doc.html_content);
    }

    #[test]
    fn test_gfm_strikethrough_renders() {
        let content = "~~gone~~";
        let doc = MarkdownDocument::new(content, "s.md".into(), "/s".into()).unwrap();
        assert!(doc.html_content.contains("<del>"), "{}", doc.html_content);
    }

    #[test]
    fn test_raw_html_passes_through() {
        let content = "<div class=\"custom\">hello</div>";
        let doc = MarkdownDocument::new(content, "h.md".into(), "/h".into()).unwrap();
        assert!(
            doc.html_content.contains("<div class=\"custom\">"),
            "{}",
            doc.html_content
        );
    }

    #[test]
    fn test_markdown_document_no_frontmatter() {
        let content = "# Hello\nNo front matter here.";
        let doc = MarkdownDocument::new(content, "file.md".into(), "/posts/file".into()).unwrap();
        assert!(doc.header.is_empty());
        assert!(doc.html_content.contains("Hello"));
    }

    #[test]
    fn test_to_post_metadata_rfc2822_date() {
        let content = r#"---
title: Post
date: Wed, 10 Jan 2024 10:00:00 +0000
---
Content"#;
        let doc = MarkdownDocument::new(content, "p.md".into(), "/p".into()).unwrap();
        let meta = doc.to_post_metadata().unwrap();
        assert_eq!(meta.date.format("%Y-%m-%d").to_string(), "2024-01-10");
    }

    #[test]
    fn test_to_post_metadata_simple_date() {
        let content = r#"---
title: Post
date: 2024-03-15
---
Content"#;
        let doc = MarkdownDocument::new(content, "p.md".into(), "/p".into()).unwrap();
        let meta = doc.to_post_metadata().unwrap();
        assert_eq!(meta.date.format("%Y-%m-%d").to_string(), "2024-03-15");
    }

    #[test]
    fn test_to_post_metadata_description_fallback() {
        let content = r#"---
title: Post
description: From description field
---
Content"#;
        let doc = MarkdownDocument::new(content, "p.md".into(), "/p".into()).unwrap();
        let meta = doc.to_post_metadata().unwrap();
        assert_eq!(meta.excerpt, "From description field");
    }

    #[test]
    fn test_to_post_metadata_content_derived_excerpt() {
        let content = r#"---
title: Post
---
First paragraph of content."#;
        let doc = MarkdownDocument::new(content, "p.md".into(), "/p".into()).unwrap();
        let meta = doc.to_post_metadata().unwrap();
        assert!(!meta.excerpt.is_empty());
    }

    #[test]
    fn test_as_tinylang_state() {
        let content = r#"---
title: My Title
custom_key: custom_value
---
# Body"#;
        let doc = MarkdownDocument::new(content, "f.md".into(), "/f".into()).unwrap();
        let state = doc.as_tinylang_state();
        assert!(state.contains_key("title"));
        assert!(state.contains_key("content"));
        assert!(state.contains_key("partial_uri"));
        assert!(state.contains_key("custom_key"));
    }
}
