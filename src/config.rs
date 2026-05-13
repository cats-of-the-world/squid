use anyhow::{Context, Result};
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use std::fs::read_to_string;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Configuration {
    pub website_name: String,
    pub uri: String,
    pub custom_keys: HashMap<String, String>,
    #[serde(default)]
    pub posts_per_page: Option<usize>,
}

impl Configuration {
    pub fn from_toml(path: &str) -> Result<Self> {
        let content = read_to_string(path).context("could not read config file")?;
        let configuration = toml::from_str(&content).context("invalid toml file")?;
        Ok(configuration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempdir::TempDir;

    #[test]
    fn test_from_toml() {
        let content = r#"
        website_name = "my website"
        uri = "https://my_website.com"
        [custom_keys]
        something = "nice"
        "#;
        let tempdir = TempDir::new("toml").unwrap();
        let file_path = tempdir.into_path().join("config.toml");
        let mut file = File::create(&file_path).unwrap();
        writeln!(file, "{}", content).unwrap();

        let config = Configuration::from_toml(file_path.to_str().unwrap()).unwrap();
        assert_eq!("my website".to_string(), config.website_name);
        assert_eq!("https://my_website.com".to_string(), config.uri);
        assert_eq!(
            "nice",
            config.custom_keys.get("something").unwrap().as_str()
        );
    }

    #[test]
    fn test_from_toml_file_not_found() {
        let result = Configuration::from_toml("/nonexistent/path/config.toml");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("could not read config file"));
    }

    #[test]
    fn test_from_toml_invalid_toml() {
        let tempdir = TempDir::new("toml").unwrap();
        let file_path = tempdir.into_path().join("bad.toml");
        let mut file = File::create(&file_path).unwrap();
        writeln!(file, "not valid toml [[[").unwrap();
        let result = Configuration::from_toml(file_path.to_str().unwrap());
        assert!(result.is_err());
    }

    #[test]
    fn test_from_toml_posts_per_page() {
        let content = r#"
        website_name = "site"
        uri = "https://example.com"
        posts_per_page = 10
        [custom_keys]
        "#;
        let tempdir = TempDir::new("toml").unwrap();
        let file_path = tempdir.into_path().join("config.toml");
        let mut file = File::create(&file_path).unwrap();
        writeln!(file, "{}", content).unwrap();
        let config = Configuration::from_toml(file_path.to_str().unwrap()).unwrap();
        assert_eq!(config.posts_per_page, Some(10));
    }

    #[test]
    fn test_from_toml_posts_per_page_defaults_to_none() {
        let content = r#"
        website_name = "site"
        uri = "https://example.com"
        [custom_keys]
        "#;
        let tempdir = TempDir::new("toml").unwrap();
        let file_path = tempdir.into_path().join("config.toml");
        let mut file = File::create(&file_path).unwrap();
        writeln!(file, "{}", content).unwrap();
        let config = Configuration::from_toml(file_path.to_str().unwrap()).unwrap();
        assert_eq!(config.posts_per_page, None);
    }
}
