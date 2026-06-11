//! Generates a sitemap.xml from the set of pages the build produced.

use std::path::Path;

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\'', "&apos;")
        .replace('"', "&quot;")
}

/// Write a sitemap.xml in `output_dir` listing every page under `base_url`.
/// `pages` are site-relative paths like "index.html" or "posts/hello.html".
pub fn generate_sitemap(
    base_url: &str,
    mut pages: Vec<String>,
    output_dir: &Path,
) -> std::io::Result<()> {
    // deterministic output regardless of build order
    pages.sort();

    let base = base_url.trim_end_matches('/');
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    for page in &pages {
        let url = format!("{}/{}", base, page.trim_start_matches('/'));
        xml.push_str("  <url><loc>");
        xml.push_str(&xml_escape(&url));
        xml.push_str("</loc></url>\n");
    }
    xml.push_str("</urlset>\n");

    std::fs::write(output_dir.join("sitemap.xml"), xml)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempdir::TempDir;

    #[test]
    fn test_generate_sitemap_lists_pages() {
        let tempdir = TempDir::new("sitemap").unwrap();
        generate_sitemap(
            "https://example.com",
            vec!["index.html".into(), "posts/hello.html".into()],
            tempdir.path(),
        )
        .unwrap();

        let content = std::fs::read_to_string(tempdir.path().join("sitemap.xml")).unwrap();
        assert!(content.contains("<loc>https://example.com/index.html</loc>"));
        assert!(content.contains("<loc>https://example.com/posts/hello.html</loc>"));
    }

    #[test]
    fn test_generate_sitemap_sorted_and_escaped() {
        let tempdir = TempDir::new("sitemap").unwrap();
        generate_sitemap(
            "https://example.com/",
            vec!["b.html".into(), "a&b.html".into()],
            tempdir.path(),
        )
        .unwrap();

        let content = std::fs::read_to_string(tempdir.path().join("sitemap.xml")).unwrap();
        let a = content.find("a&amp;b.html").unwrap();
        let b = content.find("/b.html").unwrap();
        assert!(a < b, "pages should be sorted");
        // no double slash after the base url
        assert!(content.contains("https://example.com/b.html"));
    }

    #[test]
    fn test_generate_sitemap_empty() {
        let tempdir = TempDir::new("sitemap").unwrap();
        generate_sitemap("https://example.com", vec![], tempdir.path()).unwrap();
        let content = std::fs::read_to_string(tempdir.path().join("sitemap.xml")).unwrap();
        assert!(content.contains("<urlset"));
    }
}
