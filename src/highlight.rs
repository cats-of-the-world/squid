//! Syntax highlighting for fenced code blocks.
//!
//! The markdown renderer emits `<pre><code class="language-X">escaped</code></pre>`
//! for fenced blocks; this module replaces those with syntect-highlighted HTML
//! using inline styles, so no extra CSS is required on the site.

use std::sync::LazyLock;
use syntect::highlighting::ThemeSet;
use syntect::html::highlighted_html_for_string;
use syntect::parsing::SyntaxSet;

pub const DEFAULT_THEME: &str = "InspiredGitHub";

static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEME_SET: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

static CODE_BLOCK_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r#"(?s)<pre><code class="language-([^"]+)">(.*?)</code></pre>"#).unwrap()
});

/// Undo the entity escaping the markdown renderer applied to code contents.
fn unescape(code: &str) -> String {
    code.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// Replace fenced code blocks in `html` with syntect-highlighted markup.
/// `theme` is a syntect theme name; "none" disables highlighting and unknown
/// themes leave the document untouched (with a warning), so a typo in the
/// config never breaks a build.
pub fn highlight_code_blocks(html: &str, theme: &str) -> String {
    if theme == "none" {
        return html.to_string();
    }

    let Some(theme) = THEME_SET.themes.get(theme) else {
        let available: Vec<&str> = THEME_SET.themes.keys().map(String::as_str).collect();
        eprintln!(
            "unknown code_theme '{theme}', skipping highlighting (available: {}, or 'none')",
            available.join(", ")
        );
        return html.to_string();
    };

    CODE_BLOCK_RE
        .replace_all(html, |caps: &regex::Captures| {
            let lang = &caps[1];
            let code = unescape(&caps[2]);
            let syntax = SYNTAX_SET
                .find_syntax_by_token(lang)
                .unwrap_or_else(|| SYNTAX_SET.find_syntax_plain_text());
            // on a highlighting error keep the original escaped block
            highlighted_html_for_string(&code, &SYNTAX_SET, syntax, theme)
                .unwrap_or_else(|_| caps[0].to_string())
        })
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUST_BLOCK: &str =
        "<pre><code class=\"language-rust\">let x = &quot;hi&quot;;\n</code></pre>";

    #[test]
    fn test_highlights_known_language() {
        let result = highlight_code_blocks(RUST_BLOCK, DEFAULT_THEME);
        assert!(result.contains("style="), "{result}");
        assert!(!result.contains("language-rust"), "{result}");
    }

    #[test]
    fn test_theme_none_disables_highlighting() {
        let result = highlight_code_blocks(RUST_BLOCK, "none");
        assert_eq!(result, RUST_BLOCK);
    }

    #[test]
    fn test_unknown_theme_keeps_html() {
        let result = highlight_code_blocks(RUST_BLOCK, "definitely-not-a-theme");
        assert_eq!(result, RUST_BLOCK);
    }

    #[test]
    fn test_unknown_language_falls_back_to_plain() {
        let html = "<pre><code class=\"language-nosuchlang\">plain text</code></pre>";
        let result = highlight_code_blocks(html, DEFAULT_THEME);
        assert!(result.contains("plain text"), "{result}");
    }

    #[test]
    fn test_blocks_without_language_untouched() {
        let html = "<pre><code>no language</code></pre>";
        let result = highlight_code_blocks(html, DEFAULT_THEME);
        assert_eq!(result, html);
    }

    #[test]
    fn test_unescape_restores_entities() {
        assert_eq!(
            unescape("&lt;T&gt; &amp;&amp; &quot;s&quot;"),
            "<T> && \"s\""
        );
    }
}
