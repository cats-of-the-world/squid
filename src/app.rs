use crate::config::Configuration;
use crate::deps::{FileChangeEvent, FileChangeType};
use crate::http;
use crate::io::copy_dir;
use crate::template::Website;
use crate::watch::FolderWatcher;
use anyhow::Result;
use chrono::Local;
use clap::{Parser, Subcommand};
use std::fs;
use std::path::Path;
use std::process::exit;
use tokio::runtime::Handle;
use tokio::signal;
use tokio::sync::mpsc;
use tokio::task::JoinSet;

#[derive(Subcommand, Debug, Clone)]
enum Commands {
    /// Initialize a new website in the current directory
    Init,
    /// Create a new content file
    New {
        /// Target folder (e.g. posts)
        folder: String,
        /// File name without extension
        name: String,
    },
}

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
pub(crate) struct Args {
    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(short, long)]
    template_folder: Option<String>,

    #[arg(short, long)]
    markdown_folder: Option<String>,

    #[arg(short, long)]
    static_resources: Option<String>,

    #[arg(short = 'v', long)]
    template_variables: Option<String>,

    #[arg(short, long)]
    output_folder: Option<String>,

    #[arg(short, long)]
    watch: bool,

    #[arg(short = 'p', long)]
    serve: Option<u16>,
}

pub struct App {
    args: Args,
}

impl App {
    pub fn new() -> Self {
        Self {
            args: Args::parse(),
        }
    }

    pub async fn run(&mut self) {
        match &self.args.command {
            Some(Commands::Init) => {
                Self::init_website();
                return;
            }
            Some(Commands::New { folder, name }) => {
                Self::create_new_file(self.args.markdown_folder.as_deref(), folder, name);
                return;
            }
            None => {}
        }

        let template_folder = self.args.template_folder.as_deref().unwrap_or_else(|| {
            eprintln!("error: --template-folder is required when not using a subcommand");
            exit(1);
        });
        let output_folder_str = self.args.output_folder.as_deref().unwrap_or_else(|| {
            eprintln!("error: --output-folder is required when not using a subcommand");
            exit(1);
        });
        let output_folder = Path::new(output_folder_str);
        let website = self
            .build_website(template_folder, output_folder)
            .await
            .unwrap_or_else(|e| {
                eprintln!("Error building website: {e:#}");
                exit(1);
            });
        self.copy_static_files(output_folder);

        let mut async_server = None;

        if let Some(port) = self.args.serve.as_ref() {
            println!("Serving website at http://127.0.0.1:{port}");
            async_server = Some(http::serve(*port, output_folder_str));
        }

        if let Some(async_server) = async_server {
            // if server flag is on, we always will rebuild the website
            // on changes
            tokio::select! {
                _ = async_server => {},
                _ = self.watch_website_files(website) => {},
                _ = signal::ctrl_c() => { println!("Stopping..."); }
            };
        } else if self.args.watch {
            println!("going to watch for change on files");
            tokio::select! {
                _ = self.watch_website_files(website) => {},
                _ = signal::ctrl_c() => { println!("Stopping..."); },
            };
        }
    }

    async fn build_website(&self, template_folder: &str, output_folder: &Path) -> Result<Website> {
        let template_folder = Path::new(template_folder);

        let config = self
            .args
            .template_variables
            .as_ref()
            .map(|f| Configuration::from_toml(f))
            .transpose()?;
        let markdown_folder = self
            .args
            .markdown_folder
            .as_ref()
            .map(|f| Path::new(&f).to_path_buf());

        let mut website = Website::new(config, template_folder.to_path_buf(), markdown_folder);
        let mut files_processed = website.build_from_scratch(output_folder).await?;

        Self::process_website_files(&mut files_processed).await;

        Ok(website)
    }

    fn init_website() {
        // Directory structure: markdown/posts, templates, static, output
        let dirs = ["markdown/posts", "templates", "static", "output"];
        for dir in dirs {
            if let Err(e) = fs::create_dir_all(dir) {
                eprintln!("Failed to create directory '{dir}': {e}");
                exit(1);
            }
        }

        let files: &[(&str, &str)] = &[
            (
                "config.toml",
                r#"website_name = "My Website"
uri = "https://example.com"

[custom_keys]
description = "A website built with Squid"
language = "en-us"
"#,
            ),
            (
                "templates/index.template",
                r#"<html>
    <head>
        <title>{{ website_name }}</title>
    </head>
    <body>
        <h1>{{ website_name }}</h1>
        <p>{{ description }}</p>
        <h2>Posts</h2>
        <ul>
        {% for post in sort_by_key(posts.items, 'title') %}
            <li><a href="{{ post.partial_uri }}">{{ post.title }}</a></li>
        {% end %}
        </ul>
    </body>
</html>
"#,
            ),
            (
                "templates/_posts.template",
                r#"<html>
    <head>
        <title>{{ content.title }} - {{ website_name }}</title>
    </head>
    <body>
        <h1>{{ content.title }}</h1>
        {{ content.content }}
        <br />
        <a href="/index.html">Back to home</a>
    </body>
</html>
"#,
            ),
            (
                "markdown/posts/hello-world.md",
                &format!(
                    "---\ntitle: Hello World\ndate: {}\nauthor: \n---\n\nWelcome to your new Squid website!\n",
                    Local::now().format("%Y-%m-%d")
                ),
            ),
        ];

        for (path, content) in files {
            if Path::new(path).exists() {
                println!("skipped '{path}' (already exists)");
                continue;
            }
            if let Err(e) = fs::write(path, content) {
                eprintln!("Failed to write '{path}': {e}");
                exit(1);
            }
            println!("created '{path}'");
        }

        println!();
        println!("Website initialized. Build with:");
        println!();
        println!(
            "  squid --template-folder templates --markdown-folder markdown \
             --static-resources static --output-folder output --template-variables config.toml"
        );
        println!();
        println!("Or to watch and serve locally:");
        println!();
        println!(
            "  squid --template-folder templates --markdown-folder markdown \
             --static-resources static --output-folder output --template-variables config.toml \
             --watch --serve 8080"
        );
    }

    fn create_new_file(markdown_folder: Option<&str>, folder: &str, name: &str) {
        let dir = match markdown_folder {
            Some(base) => Path::new(base).join(folder),
            None => Path::new(folder).to_path_buf(),
        };
        if let Err(e) = fs::create_dir_all(&dir) {
            eprintln!("Failed to create directory '{folder}': {e}");
            exit(1);
        }

        let file_path = dir.join(format!("{name}.md"));
        if file_path.exists() {
            eprintln!("File '{}' already exists", file_path.display());
            exit(1);
        }

        let title = name.replace('-', " ");
        let date = Local::now().format("%Y-%m-%d");
        let content = format!("---\ntitle: {title}\ndate: {date}\n---\n");

        if let Err(e) = fs::write(&file_path, content) {
            eprintln!("Failed to write '{}': {e}", file_path.display());
            exit(1);
        }

        println!("Created '{}'", file_path.display());
    }

    async fn process_website_files(files_processed: &mut JoinSet<Result<String>>) {
        let mut failed = false;

        while let Some(res) = files_processed.join_next().await {
            match res {
                Ok(Ok(file)) => {
                    println!("successfully processed {file}");
                }
                Ok(Err(e)) => {
                    eprintln!("failed to process file: {e:#}");
                    failed = true;
                }
                Err(e) => {
                    eprintln!("task panicked: {e:?}");
                    failed = true;
                }
            };
        }

        if failed {
            exit(1);
        }
    }

    fn copy_static_files(&self, output_folder: &Path) {
        let static_resources = self
            .args
            .static_resources
            .as_ref()
            .map(|dir| copy_dir(Path::new(&dir), output_folder));

        match static_resources {
            Some(Err(e)) => {
                eprintln!(
                    "task failed, could not copy static resources {:?}",
                    e.to_string()
                );
                exit(1);
            }
            Some(_) => println!("Copied static resources"),
            _ => println!("No static resources to be copied over"),
        }
    }

    /// watches for change in the directories selected by the user
    /// in order to re-build the website
    async fn watch_website_files(&self, mut website: Website) {
        let (tx, mut rx) = mpsc::channel(1);
        let mut watcher = FolderWatcher::new(Handle::current(), tx);

        if let Some(template_folder) = self.args.template_folder.as_ref() {
            if let Err(e) = watcher.watch(template_folder, FileChangeType::Template) {
                eprintln!("Failed to watch template folder '{template_folder}': {e}");
                return;
            }
        }

        if let Some(markdown_folder) = self.args.markdown_folder.as_ref() {
            if let Err(e) = watcher.watch(markdown_folder, FileChangeType::Markdown) {
                eprintln!("Failed to watch markdown folder '{markdown_folder}': {e}");
                return;
            }
        }

        if let Some(template_var) = self.args.template_variables.as_ref() {
            if let Err(e) = watcher.watch(template_var, FileChangeType::Config) {
                eprintln!("Failed to watch config file '{template_var}': {e}");
                return;
            }
        }

        if let Some(static_resources) = self.args.static_resources.as_ref() {
            if let Err(e) = watcher.watch(static_resources, FileChangeType::Static) {
                eprintln!("Failed to watch static resources '{static_resources}': {e}");
                return;
            }
        }

        let output_folder_str = self.args.output_folder.as_deref().unwrap_or("");
        let output_folder = Path::new(output_folder_str);

        while let Some(change) = rx.recv().await {
            println!("Detected changes on files, rebuilding site");
            self.handle_file_change(&mut website, &change, output_folder)
                .await;
            println!("Site rebuilt");
        }
    }

    async fn handle_file_change(
        &self,
        website: &mut Website,
        change: &FileChangeEvent,
        output_folder: &Path,
    ) {
        match change.change_type {
            FileChangeType::Static => {
                self.copy_static_files(output_folder);
            }
            FileChangeType::Markdown => {
                match website
                    .build_incremental_markdown(change, output_folder)
                    .await
                {
                    Ok(mut files_processed) => {
                        Self::process_website_files(&mut files_processed).await;
                    }
                    Err(e) => {
                        eprintln!("Incremental markdown rebuild failed: {e}, falling back to full rebuild");
                        if let Ok(mut files_processed) =
                            website.build_from_scratch(output_folder).await
                        {
                            Self::process_website_files(&mut files_processed).await;
                            self.copy_static_files(output_folder);
                        }
                    }
                }
            }
            FileChangeType::Template | FileChangeType::Config => {
                match website.build_incremental(change, output_folder).await {
                    Ok(Some(mut files_processed)) => {
                        Self::process_website_files(&mut files_processed).await;
                    }
                    Ok(None) | Err(_) => match website.build_from_scratch(output_folder).await {
                        Ok(mut files_processed) => {
                            Self::process_website_files(&mut files_processed).await;
                            self.copy_static_files(output_folder);
                        }
                        Err(e) => eprintln!("Full rebuild failed: {e:#}"),
                    },
                }
            }
        }
    }
}
