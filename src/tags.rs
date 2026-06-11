//! Tag taxonomy support.
//!
//! Tags are declared in frontmatter (`tags: rust, blogging` or a YAML list)
//! and collected across every markdown collection. A reserved `_tag.template`
//! is rendered once per tag into `tags/<slug>.html`.

use crate::md::{MarkdownCollection, MarkdownDocument};
use std::collections::{BTreeMap, HashMap};
use tinylang::types::{State, TinyLangType};

/// A tag and every document carrying it.
pub struct Tag {
    pub name: String,
    pub slug: String,
    /// site-absolute link to the tag page, e.g. /tags/rust.html or /tags/rust/
    pub uri: String,
    /// output path relative to the tags directory, e.g. rust.html or rust/index.html
    pub output_name: String,
    pub documents: Vec<MarkdownDocument>,
}

impl Tag {
    pub fn as_tinylang_state(&self) -> State {
        let mut state = State::new();
        state.insert("name".into(), self.name.clone().into());
        state.insert("slug".into(), self.slug.clone().into());
        state.insert("uri".into(), self.uri.clone().into());
        state.insert(
            "size".into(),
            TinyLangType::Numeric(self.documents.len() as f64),
        );
        state.insert(
            "items".into(),
            TinyLangType::Vec(
                self.documents
                    .iter()
                    .map(|doc| TinyLangType::Object(doc.as_tinylang_state()))
                    .collect(),
            ),
        );
        state
    }
}

/// Gather all tags across collections, sorted by name. Documents keep their
/// collection ordering (newest first within a collection).
pub fn collect_tags(
    collections: &HashMap<String, MarkdownCollection>,
    pretty_urls: bool,
) -> Vec<Tag> {
    let mut by_name: BTreeMap<String, Vec<MarkdownDocument>> = BTreeMap::new();

    for collection in collections.values() {
        for doc in &collection.collection {
            for tag in doc.tags() {
                by_name.entry(tag).or_default().push(doc.clone());
            }
        }
    }

    by_name
        .into_iter()
        .map(|(name, documents)| {
            let slug = slugify(&name);
            let (uri, output_name) = if pretty_urls {
                (format!("/tags/{slug}/"), format!("{slug}/index.html"))
            } else {
                (format!("/tags/{slug}.html"), format!("{slug}.html"))
            };
            Tag {
                slug,
                uri,
                output_name,
                name,
                documents,
            }
        })
        .collect()
}

/// The global `tags` value exposed to every template.
pub fn tags_state(tags: &[Tag]) -> TinyLangType {
    TinyLangType::Vec(
        tags.iter()
            .map(|t| TinyLangType::Object(t.as_tinylang_state()))
            .collect(),
    )
}

/// Lowercase, alphanumerics kept, everything else collapsed into single
/// dashes: "Rust & WebAssembly!" -> "rust-webassembly".
pub fn slugify(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut pending_dash = false;

    for c in value.chars() {
        if c.is_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.extend(c.to_lowercase());
        } else {
            pending_dash = true;
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_doc(name: &str, tags: &str) -> MarkdownDocument {
        let content = format!("---\ntitle: {name}\ntags: {tags}\n---\nBody");
        MarkdownDocument::new(
            &content,
            format!("{name}.md"),
            format!("/posts/{name}.html"),
        )
        .unwrap()
    }

    fn make_collections(docs: Vec<MarkdownDocument>) -> HashMap<String, MarkdownCollection> {
        let mut coll = MarkdownCollection::new(PathBuf::from("posts"));
        coll.collection = docs;
        let mut collections = HashMap::new();
        collections.insert("posts".to_string(), coll);
        collections
    }

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("Rust & WebAssembly!"), "rust-webassembly");
        assert_eq!(slugify("simple"), "simple");
        assert_eq!(slugify("  spaced  out  "), "spaced-out");
        assert_eq!(slugify("CamelCase"), "camelcase");
    }

    #[test]
    fn test_collect_tags_groups_documents() {
        let collections = make_collections(vec![make_doc("a", "rust, web"), make_doc("b", "rust")]);
        let tags = collect_tags(&collections, false);

        assert_eq!(tags.len(), 2);
        // BTreeMap ordering: rust before web
        assert_eq!(tags[0].name, "rust");
        assert_eq!(tags[0].documents.len(), 2);
        assert_eq!(tags[1].name, "web");
        assert_eq!(tags[1].documents.len(), 1);
    }

    #[test]
    fn test_collect_tags_empty_when_no_tags() {
        let content = "---\ntitle: untagged\n---\nBody";
        let doc = MarkdownDocument::new(content, "u.md".into(), "/posts/u.html".into()).unwrap();
        let tags = collect_tags(&make_collections(vec![doc]), false);
        assert!(tags.is_empty());
    }

    #[test]
    fn test_tag_state_shape() {
        let collections = make_collections(vec![make_doc("a", "rust")]);
        let tags = collect_tags(&collections, false);
        let state = tags[0].as_tinylang_state();

        assert!(matches!(state.get("name"), Some(TinyLangType::String(s)) if s == "rust"));
        assert!(
            matches!(state.get("uri"), Some(TinyLangType::String(s)) if s == "/tags/rust.html")
        );
        assert!(matches!(state.get("size"), Some(TinyLangType::Numeric(n)) if *n == 1.0));
        assert!(matches!(state.get("items"), Some(TinyLangType::Vec(v)) if v.len() == 1));
    }

    #[test]
    fn test_tag_pretty_urls() {
        let collections = make_collections(vec![make_doc("a", "rust")]);
        let tags = collect_tags(&collections, true);
        assert_eq!(tags[0].uri, "/tags/rust/");
        assert_eq!(tags[0].output_name, "rust/index.html");
    }
}
