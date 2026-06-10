use chrono::{DateTime, Utc};
use rss::{ChannelBuilder, GuidBuilder, Item, ItemBuilder};

pub struct FeedConfig {
    pub title: String,
    pub description: String,
    pub website_url: String,
    pub language: String,
}

#[derive(Debug, Clone)]
pub struct PostMetadata {
    pub title: String,
    pub file_name: String,
    pub date: DateTime<Utc>,
    pub excerpt: String,
    pub html_content: String,
    pub author: String,
    pub tags: Vec<String>,
}

pub fn generate_rss(
    config: &FeedConfig,
    posts: &[PostMetadata],
    output_dir: &std::path::Path,
) -> std::io::Result<()> {
    // Sort posts by date (newest first)
    let mut sorted_posts = posts.to_vec();
    sorted_posts.sort_by(|a, b| b.date.cmp(&a.date));

    let items: Vec<Item> = sorted_posts
        .iter()
        .map(|post| {
            let post_url = format!("{}{}", config.website_url, post.file_name);

            ItemBuilder::default()
                .title(Some(post.title.clone()))
                .link(Some(post_url.clone()))
                .description(Some(post.excerpt.clone()))
                .content(Some(post.html_content.clone()))
                .author(Some(post.author.clone()))
                .guid(Some(
                    GuidBuilder::default()
                        .value(post_url)
                        .permalink(true)
                        .build(),
                ))
                .pub_date(Some(post.date.to_rfc2822()))
                .build()
        })
        .collect();

    // Build the channel (feed)
    let channel = ChannelBuilder::default()
        .title(config.title.clone())
        .description(config.description.clone())
        .link(config.website_url.clone())
        .items(items)
        .language(Some(config.language.clone()))
        .last_build_date(Some(Utc::now().to_rfc2822()))
        .generator(Some("Squid".to_string()))
        .build();

    let rss_content = channel.to_string();
    let output_path = output_dir.join("rss.xml");
    std::fs::write(output_path, rss_content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempdir::TempDir;

    fn make_config() -> FeedConfig {
        FeedConfig {
            title: "My Blog".to_string(),
            description: "A test blog".to_string(),
            website_url: "https://example.com".to_string(),
            language: "en-us".to_string(),
        }
    }

    fn make_post(title: &str, year: i32, month: u32, day: u32) -> PostMetadata {
        let date = DateTime::<Utc>::from_naive_utc_and_offset(
            chrono::NaiveDate::from_ymd_opt(year, month, day)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap(),
            Utc,
        );
        PostMetadata {
            title: title.to_string(),
            file_name: format!("/posts/{}", title.to_lowercase().replace(' ', "-")),
            date,
            excerpt: "excerpt".to_string(),
            html_content: "<p>content</p>".to_string(),
            author: "Author".to_string(),
            tags: vec![],
        }
    }

    #[test]
    fn test_generate_rss_creates_file_with_content() {
        let tempdir = TempDir::new("rss").unwrap();
        let posts = vec![make_post("Test Post", 2024, 1, 10)];
        generate_rss(&make_config(), &posts, tempdir.path()).unwrap();

        let rss_path = tempdir.path().join("rss.xml");
        assert!(rss_path.exists());
        let content = std::fs::read_to_string(&rss_path).unwrap();
        assert!(content.contains("My Blog"));
        assert!(content.contains("Test Post"));
        assert!(content.contains("https://example.com"));
    }

    #[test]
    fn test_generate_rss_sorts_newest_first() {
        let tempdir = TempDir::new("rss").unwrap();
        let posts = vec![
            make_post("Older Post", 2024, 1, 1),
            make_post("Newer Post", 2024, 6, 1),
        ];
        generate_rss(&make_config(), &posts, tempdir.path()).unwrap();

        let content = std::fs::read_to_string(tempdir.path().join("rss.xml")).unwrap();
        let newer_pos = content.find("Newer Post").unwrap();
        let older_pos = content.find("Older Post").unwrap();
        assert!(
            newer_pos < older_pos,
            "newer post should appear before older post"
        );
    }

    #[test]
    fn test_generate_rss_empty_posts() {
        let tempdir = TempDir::new("rss").unwrap();
        generate_rss(&make_config(), &[], tempdir.path()).unwrap();
        assert!(tempdir.path().join("rss.xml").exists());
    }
}
