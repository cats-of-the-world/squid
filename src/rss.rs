use chrono::{DateTime, Utc};
use rss::{ChannelBuilder, GuidBuilder, Item, ItemBuilder};

pub struct FeedConfig {
    pub title: String,
    pub description: String,
    pub website_url: String,
    pub feed_url: String,
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
        .link(config.feed_url.clone())
        .items(items)
        .language(Some(config.language.clone()))
        .last_build_date(Some(Utc::now().to_rfc2822()))
        .generator(Some("Squid".to_string()))
        .build();

    let rss_content = channel.to_string();
    let output_path = output_dir.join("rss.xml");
    std::fs::write(output_path, rss_content)
}
