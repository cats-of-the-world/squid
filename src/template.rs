use crate::config::Configuration;
use crate::deps::{DependencyGraph, FileChangeEvent};
use crate::io;
use crate::io::{LazyFolderReader, TemplateFile};
use crate::rss::*;
use anyhow::Context;
use anyhow::Result;

use crate::md::{MarkdownCollection, MarkdownDocument};
use crate::tinylang::{paginate, render, reverse, sort_by_key};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tinylang::eval;
use tinylang::types::{State, TinyLangType};
use tokio::fs::{create_dir, create_dir_all, remove_dir_all};
use tokio::task::JoinSet;

fn paginated_state(
    base_state: &State,
    collections: &HashMap<String, MarkdownCollection>,
    base_name: &str,
    page: usize,
    total_pages: usize,
    per_page: usize,
) -> State {
    let mut state = base_state.clone();

    for (name, collection) in collections {
        let start = (page - 1) * per_page;
        let end = (start + per_page).min(collection.collection.len());
        let page_items: Vec<TinyLangType> = if start < collection.collection.len() {
            collection.collection[start..end]
                .iter()
                .map(|doc| TinyLangType::Object(doc.as_tinylang_state()))
                .collect()
        } else {
            Vec::new()
        };

        let mut coll_state = State::new();
        coll_state.insert(
            "size".into(),
            TinyLangType::Numeric(collection.collection.len() as f64),
        );
        coll_state.insert("items".into(), TinyLangType::Vec(page_items));
        state.insert(name.clone(), TinyLangType::Object(coll_state));
    }

    let next_file = if page < total_pages {
        TinyLangType::String(page_file_name(base_name, page + 1))
    } else {
        TinyLangType::Nil
    };
    let prev_file = if page > 1 {
        TinyLangType::String(page_file_name(base_name, page - 1))
    } else {
        TinyLangType::Nil
    };

    let mut pagination = State::new();
    pagination.insert("current_page".into(), TinyLangType::Numeric(page as f64));
    pagination.insert(
        "total_pages".into(),
        TinyLangType::Numeric(total_pages as f64),
    );
    pagination.insert("per_page".into(), TinyLangType::Numeric(per_page as f64));
    pagination.insert("has_next".into(), TinyLangType::Bool(page < total_pages));
    pagination.insert("has_prev".into(), TinyLangType::Bool(page > 1));
    pagination.insert("next_file".into(), next_file);
    pagination.insert("prev_file".into(), prev_file);
    state.insert("pagination".into(), TinyLangType::Object(pagination));

    state
}

/// Replace `from` with `to` only when it is the suffix of `name`, unlike
/// `str::replace` which would also rewrite occurrences in the middle of the
/// name (e.g. `amd.md` must become `amd.html`, not `ahtml.html`).
fn swap_suffix(name: &str, from: &str, to: &str) -> String {
    match name.strip_suffix(from) {
        Some(stem) => format!("{stem}{to}"),
        None => name.to_string(),
    }
}

fn page_file_name(base_name: &str, page: usize) -> String {
    if page == 1 {
        format!("{base_name}.html")
    } else {
        format!("{base_name}-page-{page}.html")
    }
}

fn total_pages_for(collections: &HashMap<String, MarkdownCollection>, per_page: usize) -> usize {
    let max_items = collections
        .values()
        .map(|c| c.collection.len())
        .max()
        .unwrap_or(0);
    max_items.div_ceil(per_page).max(1)
}

/// Rebuild plan entry for a markdown output: (partial template path, output path, item state).
type MarkdownRebuildItem = (PathBuf, PathBuf, State);
/// Rebuild plan entry for a standalone output: (template path, output path, page number).
type StandaloneRebuildItem = (PathBuf, PathBuf, Option<usize>);

struct Builder {
    tinylang_state: Arc<State>,
    output_folder: PathBuf,
    eval_tasks: Option<JoinSet<anyhow::Result<String>>>,
    posts_per_page: Option<usize>,
    collections: Arc<HashMap<String, MarkdownCollection>>,
}

impl Builder {
    fn new(
        state: State,
        output_folder: PathBuf,
        posts_per_page: Option<usize>,
        collections: Arc<HashMap<String, MarkdownCollection>>,
    ) -> Self {
        Self {
            tinylang_state: Arc::new(state),
            output_folder,
            eval_tasks: None,
            posts_per_page,
            collections,
        }
    }

    fn total_pages(&self, per_page: usize) -> usize {
        total_pages_for(&self.collections, per_page)
    }

    fn build_paginated_state(
        &self,
        base_name: &str,
        page: usize,
        total_pages: usize,
        per_page: usize,
    ) -> State {
        paginated_state(
            &self.tinylang_state,
            &self.collections,
            base_name,
            page,
            total_pages,
            per_page,
        )
    }

    async fn process_folder(
        &mut self,
        eval_tasks: JoinSet<anyhow::Result<String>>,
        template_folder_reader: &mut LazyFolderReader,
        collections: &HashMap<String, MarkdownCollection>,
    ) -> Result<()> {
        self.eval_tasks = Some(eval_tasks);

        while let Some(file) = template_folder_reader.async_next().await {
            let file = file?;

            // should handle _name.template differently since they are partials
            // for the rest of the templates we should generate a single output with same name
            if file.name.starts_with('_') {
                // removing the first _
                // and the .template from the end
                // this is safe because we filtered based on the extension name ('.template')
                let collection_name = &file.name[1..file.name.len() - 9];
                if let Some(collection) = collections.get(collection_name) {
                    self.eval_markdown_collection_to_output_file(collection.clone(), file)
                        .await?;
                }
                continue;
            }

            self.eval_template_to_output_file(file);
        }
        Ok(())
    }

    /// build a template without any markdown
    fn eval_template_to_output_file(&mut self, file: TemplateFile) {
        let base_name = swap_suffix(&file.name, ".template", "");

        match self.posts_per_page {
            Some(per_page) if per_page > 0 => {
                let total_pages = self.total_pages(per_page);
                for page in 1..=total_pages {
                    let file_name = page_file_name(&base_name, page);
                    let state = self.build_paginated_state(&base_name, page, total_pages, per_page);
                    let output_folder = self.output_folder.to_path_buf();
                    let contents = file.contents.clone();
                    self.eval_tasks
                        .as_mut()
                        .expect("eval_tasks initialized by process_folder")
                        .spawn(async move {
                            let html = eval(&contents, state)
                                .map_err(|e| anyhow::anyhow!("template evaluation failed: {e}"))?;
                            io::write_to_disk(output_folder, &file_name, html).await?;
                            Ok(file_name)
                        });
                }
            }
            _ => {
                let output_folder = self.output_folder.to_path_buf();
                let state = self.tinylang_state.clone();
                self.eval_tasks
                    .as_mut()
                    .expect("eval_tasks initialized by process_folder")
                    .spawn(async move {
                        let file_name = format!("{base_name}.html");
                        let html = eval(&file.contents, (*state).clone())
                            .map_err(|e| anyhow::anyhow!("template evaluation failed: {e}"))?;
                        io::write_to_disk(output_folder, &file_name, html).await?;
                        Ok(file_name)
                    });
            }
        }
    }

    async fn mk_collection_dir(&mut self, collection: &MarkdownCollection) -> Result<PathBuf> {
        let collection_name = collection
            .relative_path
            .file_name()
            .context("collection path has no directory name")?
            .to_string_lossy()
            .to_string();

        let output_folder = self.output_folder.join(&collection_name);

        if !output_folder.exists() {
            create_dir(&output_folder).await.with_context(|| {
                format!(
                    "failed to create collection directory '{}'",
                    output_folder.display()
                )
            })?;
        }
        Ok(output_folder)
    }

    /// builds a collection of markdown files using the appropriate template
    async fn eval_markdown_collection_to_output_file(
        &mut self,
        collection: MarkdownCollection,
        template: TemplateFile,
    ) -> Result<()> {
        let output_folder = self.mk_collection_dir(&collection).await?;

        // we need for each item in the collection
        // to evaluate the template using its header and content
        for item in collection.collection {
            let output_folder = output_folder.clone();

            let state = self.tinylang_state.clone();

            let template = template.clone();

            self.eval_tasks
                .as_mut()
                .expect("eval_tasks initialized by process_folder")
                .spawn(async move {
                    let mut state = (*state).clone();
                    state.insert("content".into(), item.as_tinylang_state().into());
                    let html = eval(&template.contents, state)
                        .map_err(|e| anyhow::anyhow!("template evaluation failed: {e}"))?;

                    // we need to save our file following the markdown file and not the template
                    let file_name = swap_suffix(&item.name, ".md", ".html");

                    io::write_to_disk(output_folder, &file_name, html).await?;
                    Ok(file_name)
                });
        }
        Ok(())
    }
}

#[derive(Default)]
struct WebsiteCachedState {
    collections: Option<HashMap<String, MarkdownCollection>>,
    state: Option<State>,
    builder: Option<Builder>,
    deps: Option<DependencyGraph>,
}

pub struct Website {
    template_folder: PathBuf,
    posts_folder: Option<PathBuf>,
    configuration: Option<Configuration>,
    cache: WebsiteCachedState,
}

impl Website {
    pub(crate) fn new(
        configuration: Option<Configuration>,
        template_folder: PathBuf,
        posts_folder: Option<PathBuf>,
    ) -> Self {
        Self {
            template_folder,
            posts_folder,
            configuration,
            cache: WebsiteCachedState::default(),
        }
    }

    pub async fn build_from_scratch(
        &mut self,
        output: &Path,
    ) -> Result<JoinSet<anyhow::Result<String>>> {
        if output.exists() {
            remove_dir_all(output)
                .await
                .with_context(|| format!("failed to clean output folder '{}'", output.display()))?;
        }
        create_dir_all(output)
            .await
            .with_context(|| format!("failed to create output folder '{}'", output.display()))?;

        let collections = self.build_markdown_collections().await?;

        if let Some(feed_config) = self.feed_config() {
            self.generate_site_rss(&feed_config, &collections, output)
                .await?;
        }

        let posts_per_page = self.configuration.as_ref().and_then(|c| c.posts_per_page);
        self.cache.builder = Some(Builder::new(
            self.build_state(&collections),
            output.to_path_buf(),
            posts_per_page,
            Arc::new(collections.clone()),
        ));

        self.compile_templates().await
    }

    fn feed_config(&self) -> Option<FeedConfig> {
        let c = self.configuration.as_ref()?;
        Some(FeedConfig {
            title: c.website_name.clone(),
            description: c
                .custom_keys
                .get("description")
                .cloned()
                .unwrap_or_else(|| format!("Latest posts from {}", c.website_name)),
            website_url: c.uri.clone(),
            language: c
                .custom_keys
                .get("language")
                .cloned()
                .unwrap_or_else(|| "en-us".to_string()),
        })
    }

    async fn generate_site_rss(
        &self,
        config: &FeedConfig,
        collections: &HashMap<String, MarkdownCollection>,
        output: &std::path::Path,
    ) -> Result<()> {
        // Collect all posts from all collections
        let mut all_posts = Vec::new();

        for collection in collections.values() {
            let collection_posts = collection
                .collection
                .iter()
                .filter_map(|doc| doc.to_post_metadata().ok());
            all_posts.extend(collection_posts);
        }

        // Generate RSS feed
        generate_rss(config, &all_posts, output).context("Failed to generate RSS feed")?;

        Ok(())
    }

    /// Incrementally rebuild only the outputs affected by a markdown file change.
    ///
    /// Rebuilds the changed markdown's HTML output plus all standalone templates (since they
    /// may list collection items). Falls back by returning an error if a new or deleted
    /// markdown file is detected (dep graph won't know it).
    pub async fn build_incremental_markdown(
        &mut self,
        change: &FileChangeEvent,
        output: &Path,
    ) -> Result<JoinSet<anyhow::Result<String>>> {
        let collections = self.build_markdown_collections().await?;

        // Refresh RSS with updated collection data
        if let Some(feed_config) = self.feed_config() {
            self.generate_site_rss(&feed_config, &collections, output)
                .await?;
        }

        let state = self.build_state(&collections);

        // Collect the rebuild plan from the dep graph before releasing borrows.
        let (markdown_items, standalone_items): (
            Vec<MarkdownRebuildItem>,
            Vec<StandaloneRebuildItem>,
        ) = {
            let deps = self.cache.deps.as_ref().context("no dependency graph")?;
            let collections_ref = self
                .cache
                .collections
                .as_ref()
                .context("no collections cache")?;

            // Any path not in the dep graph means a file was added or deleted — full rebuild needed.
            for path in &change.paths {
                let canonical = path.canonicalize().unwrap_or(path.clone());
                if !deps.knows_markdown(&canonical) {
                    return Err(anyhow::anyhow!(
                        "markdown file not in dependency graph, full rebuild required"
                    ));
                }
            }

            let affected = deps.affected_outputs(change);
            let mut markdown_items = Vec::new();
            for out in &affected {
                if let Some((md_path, coll_name)) = deps.markdown_for_output(out) {
                    let partial_path = deps
                        .partial_for_collection(&coll_name)
                        .context("partial template not found for collection")?;
                    if let Some(collection) = collections_ref.get(&coll_name) {
                        let item_name = md_path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("")
                            .to_string();
                        if let Some(item) =
                            collection.collection.iter().find(|i| i.name == item_name)
                        {
                            markdown_items.push((
                                partial_path,
                                out.clone(),
                                item.as_tinylang_state(),
                            ));
                        }
                    }
                }
            }

            let standalone_items: Vec<StandaloneRebuildItem> = deps
                .standalones()
                .map(|(t, o)| (t.clone(), o.clone(), deps.page_for_output(o)))
                .collect();

            (markdown_items, standalone_items)
        };

        let mut eval_tasks = JoinSet::new();

        for (template_path, output_path, item_state) in markdown_items {
            let template = TemplateFile::new(&template_path)?;
            let output_dir = output_path
                .parent()
                .with_context(|| format!("output path has no parent: {}", output_path.display()))?
                .to_path_buf();
            let file_name = output_path
                .file_name()
                .with_context(|| {
                    format!("output path has no file name: {}", output_path.display())
                })?
                .to_string_lossy()
                .to_string();
            let mut s = state.clone();
            s.insert("content".into(), item_state.into());
            eval_tasks.spawn(async move {
                let html = eval(&template.contents, s)
                    .map_err(|e| anyhow::anyhow!("template evaluation failed: {e}"))?;
                io::write_to_disk(output_dir, &file_name, html).await?;
                Ok(file_name)
            });
        }

        let posts_per_page = self.configuration.as_ref().and_then(|c| c.posts_per_page);

        for (template_path, output_path, page_opt) in standalone_items {
            let template = TemplateFile::new(&template_path)?;
            let output_dir = output_path
                .parent()
                .with_context(|| format!("output path has no parent: {}", output_path.display()))?
                .to_path_buf();
            let file_name = output_path
                .file_name()
                .with_context(|| {
                    format!("output path has no file name: {}", output_path.display())
                })?
                .to_string_lossy()
                .to_string();
            let s = match (posts_per_page.filter(|&p| p > 0), page_opt) {
                (Some(per_page), Some(page)) => {
                    let total = total_pages_for(&collections, per_page);
                    let base_name = swap_suffix(
                        template_path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or(""),
                        ".template",
                        "",
                    );
                    paginated_state(&state, &collections, &base_name, page, total, per_page)
                }
                _ => state.clone(),
            };
            eval_tasks.spawn(async move {
                let html = eval(&template.contents, s)
                    .map_err(|e| anyhow::anyhow!("template evaluation failed: {e}"))?;
                io::write_to_disk(output_dir, &file_name, html).await?;
                Ok(file_name)
            });
        }

        Ok(eval_tasks)
    }

    pub async fn compile_templates(&mut self) -> Result<JoinSet<anyhow::Result<String>>> {
        let mut template_folder_reader =
            LazyFolderReader::new(&self.template_folder, "template")
                .context("could not create lazy folder reader for template folder")?;
        self.cache
            .builder
            .as_mut()
            .context("compile_templates called without caching builder")?
            .process_folder(
                JoinSet::new(),
                &mut template_folder_reader,
                self.cache
                    .collections
                    .as_ref()
                    .context("compile_templates called without caching collections")?,
            )
            .await?;

        let output_folder = self
            .cache
            .builder
            .as_ref()
            .context("compile_templates called without caching builder")?
            .output_folder
            .clone();
        let eval_tasks = self
            .cache
            .builder
            .as_mut()
            .context("compile_templates called without caching builder")?
            .eval_tasks
            .take()
            .context("eval_tasks not initialized in builder")?;
        self.build_dependency_graph(&output_folder).await?;
        Ok(eval_tasks)
    }

    /// Build the dependency graph for incremental builds. Must be called after
    /// compile_templates when collections and builder are populated.
    async fn build_dependency_graph(&mut self, output: &Path) -> Result<()> {
        let collections = self
            .cache
            .collections
            .as_ref()
            .context("build_dependency_graph called without collections")?;
        let output_folder = output.to_path_buf();
        let base_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

        let mut deps = DependencyGraph::new(self.template_folder.clone(), output_folder.clone());

        let mut template_reader = LazyFolderReader::new(&self.template_folder, "template")
            .context("could not create template reader for dependency graph")?;

        let posts_per_page = self.configuration.as_ref().and_then(|c| c.posts_per_page);

        while let Some(file) = template_reader.async_next().await {
            let file = file?;
            deps.register_template(file.path.clone(), &file.contents, &base_dir);

            if file.name.starts_with('_') {
                let collection_name = &file.name[1..file.name.len() - 9];
                if collections.contains_key(collection_name) {
                    deps.register_collection_partial(collection_name, file.path.clone());
                }
            } else if let Some(per_page) = posts_per_page.filter(|&p| p > 0) {
                let base_name = swap_suffix(&file.name, ".template", "");
                let total = total_pages_for(collections, per_page);
                for page in 1..=total {
                    let output_name = page_file_name(&base_name, page);
                    let output_path = output_folder.join(&output_name);
                    deps.register_standalone(file.path.clone(), &output_name);
                    deps.register_page_number(output_path, page);
                }
            } else {
                let output_name = swap_suffix(&file.name, ".template", ".html");
                deps.register_standalone(file.path.clone(), &output_name);
            }
        }

        for (collection_name, collection) in collections {
            let output_dir = output_folder.join(
                collection
                    .relative_path
                    .file_name()
                    .context("collection relative path has no file name")?
                    .to_string_lossy()
                    .as_ref(),
            );
            for item in &collection.collection {
                let md_path = collection.relative_path.join(&item.name);
                let output_name = swap_suffix(&item.name, ".md", ".html");
                let output_path = output_dir.join(&output_name);
                deps.register_markdown_output(md_path, collection_name, output_path);
            }
        }

        self.cache.deps = Some(deps);
        Ok(())
    }

    /// Incrementally rebuild only the outputs affected by the given file change.
    /// Returns None if a full rebuild is required (e.g. config change).
    pub async fn build_incremental(
        &mut self,
        change: &FileChangeEvent,
        _output: &Path,
    ) -> Result<Option<JoinSet<anyhow::Result<String>>>> {
        let deps = self.cache.deps.as_ref().context("no dependency graph")?;

        if deps.requires_full_rebuild(change) {
            return Ok(None);
        }

        if deps.is_static_change(change) {
            return Ok(Some(JoinSet::new()));
        }

        let affected = deps.affected_outputs(change);

        if affected.is_empty() {
            return Ok(Some(JoinSet::new()));
        }

        let collections = self.cache.collections.as_ref().context("no collections")?;
        let state = self.cache.state.as_ref().context("no state")?;
        let posts_per_page = self
            .cache
            .builder
            .as_ref()
            .context("no builder")?
            .posts_per_page;

        let mut eval_tasks = JoinSet::new();

        for output_path in affected {
            if let Some(template_path) = deps.template_for_output(&output_path) {
                let template = TemplateFile::new(&template_path)?;
                let output_folder = output_path
                    .parent()
                    .with_context(|| {
                        format!("output path has no parent: {}", output_path.display())
                    })?
                    .to_path_buf();
                let file_name = output_path
                    .file_name()
                    .with_context(|| {
                        format!("output path has no file name: {}", output_path.display())
                    })?
                    .to_string_lossy()
                    .to_string();
                let eval_state = match (
                    posts_per_page.filter(|&p| p > 0),
                    deps.page_for_output(&output_path),
                ) {
                    (Some(per_page), Some(page)) => {
                        let total = total_pages_for(collections, per_page);
                        let base_name = swap_suffix(
                            template_path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or(""),
                            ".template",
                            "",
                        );
                        paginated_state(state, collections, &base_name, page, total, per_page)
                    }
                    _ => state.clone(),
                };
                eval_tasks.spawn(async move {
                    let html = eval(&template.contents, eval_state)
                        .map_err(|e| anyhow::anyhow!("template evaluation failed: {e}"))?;
                    io::write_to_disk(output_folder, &file_name, html).await?;
                    Ok(file_name)
                });
            } else if let Some((md_path, coll_name)) = deps.markdown_for_output(&output_path) {
                let collection = collections
                    .get(&coll_name)
                    .context("collection not found")?;
                let item_name = md_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                let item = collection
                    .collection
                    .iter()
                    .find(|i| i.name == item_name)
                    .context("markdown item not found")?;
                let partial_path = deps
                    .partial_for_collection(&coll_name)
                    .context("partial not found")?;
                let template = TemplateFile::new(&partial_path)?;
                let output_folder = output_path
                    .parent()
                    .with_context(|| {
                        format!("output path has no parent: {}", output_path.display())
                    })?
                    .to_path_buf();
                let file_name = output_path
                    .file_name()
                    .with_context(|| {
                        format!("output path has no file name: {}", output_path.display())
                    })?
                    .to_string_lossy()
                    .to_string();
                let mut state = state.clone();
                state.insert("content".into(), item.as_tinylang_state().into());
                eval_tasks.spawn(async move {
                    let html = eval(&template.contents, state)
                        .map_err(|e| anyhow::anyhow!("template evaluation failed: {e}"))?;
                    io::write_to_disk(output_folder, &file_name, html).await?;
                    Ok(file_name)
                });
            }
        }

        Ok(Some(eval_tasks))
    }

    pub async fn build_markdown_collections(
        &mut self,
    ) -> Result<HashMap<String, MarkdownCollection>> {
        let mut collections = HashMap::new();
        let posts_folder = match &self.posts_folder {
            Some(p) => p,
            None => return Ok(collections),
        };

        let mut markdown_folder_reader = io::LazyFolderReader::new(posts_folder, "md")
            .context("could not create lazy folder reader for markdown folder")?;

        while let Some(file) = markdown_folder_reader.async_next().await {
            let file = match file {
                Ok(f) => f,
                Err(e) => {
                    //todo log lib
                    eprintln!("{}", e);
                    continue;
                }
            };

            let markdown_content = match MarkdownDocument::new(
                &file.contents,
                file.name,
                self.partial_uri(&file.path),
            ) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("{}", e);
                    continue;
                }
            };
            let mut path = file.path;
            // remove the filename
            path.pop();

            let path_as_string = match path.file_name() {
                Some(name) => name.to_string_lossy().to_string(),
                None => {
                    eprintln!(
                        "Warning: could not determine collection name for '{}'",
                        path.display()
                    );
                    continue;
                }
            };

            let collection = collections
                .entry(path_as_string)
                .or_insert(MarkdownCollection::new(path));

            collection.collection.push(markdown_content);
        }

        self.cache.collections = Some(collections.clone());
        Ok(collections)
    }

    fn partial_uri(&self, path: &Path) -> String {
        // we leave only the relative path after the `posts_folder` to avoid
        // creating a url with a local path (e.g. $HOME/my_site/posts)
        let relative = self
            .posts_folder
            .as_ref()
            .and_then(|folder| path.strip_prefix(folder).ok())
            .unwrap_or(path);
        let uri = swap_suffix(&relative.to_string_lossy(), ".md", ".html");
        format!("/{}", uri.trim_start_matches('/'))
    }

    /// We need to transform all the information we build about the collections to the
    /// template State, so that users can use them. For example, listing all the markdown
    /// posts and linking to them.
    pub fn build_collection_state(
        &self,
        collections: &HashMap<String, MarkdownCollection>,
    ) -> State {
        let mut state = State::new();

        for (key, collection) in collections {
            state.insert(key.clone(), collection.as_tinylang_state().into());
        }
        state
    }

    /// Build the generic State that will be passed to all partials and templates
    /// this allow users to define special variables that they may want to use on their
    /// template.
    fn build_default_state(&self) -> State {
        let mut state = HashMap::default();

        if let Some(c) = self.configuration.as_ref() {
            state.insert("website_name".into(), c.website_name.clone().into());
            state.insert("uri".into(), c.uri.clone().into());
            for (key, value) in &c.custom_keys {
                state.insert(key.clone(), value.clone().into());
            }
        }

        state.insert("render".into(), TinyLangType::Function(render));
        state.insert("sort_by_key".into(), TinyLangType::Function(sort_by_key));
        state.insert("reverse".into(), TinyLangType::Function(reverse));
        state.insert("paginate".into(), TinyLangType::Function(paginate));
        state
    }

    fn build_state(&mut self, collections: &HashMap<String, MarkdownCollection>) -> State {
        let mut state = self.build_default_state();
        // passes all the collections state as well so users can use it for
        // things like pagination
        state.extend(self.build_collection_state(collections));
        self.cache.state = Some(state.clone());
        state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_page_file_name_page_one() {
        assert_eq!(page_file_name("index", 1), "index.html");
    }

    #[test]
    fn test_page_file_name_subsequent_pages() {
        assert_eq!(page_file_name("index", 2), "index-page-2.html");
        assert_eq!(page_file_name("index", 10), "index-page-10.html");
    }

    #[test]
    fn test_total_pages_for_empty_collections() {
        let collections: HashMap<String, MarkdownCollection> = HashMap::new();
        assert_eq!(total_pages_for(&collections, 5), 1);
    }

    #[test]
    fn test_total_pages_for_exact_multiple() {
        let mut collections = HashMap::new();
        let mut coll = MarkdownCollection::new(PathBuf::from("posts"));
        for i in 0..6 {
            coll.collection.push(
                crate::md::MarkdownDocument::new(
                    &format!("---\ntitle: Post {i}\n---\ncontent"),
                    format!("post{i}.md"),
                    format!("/posts/post{i}"),
                )
                .unwrap(),
            );
        }
        collections.insert("posts".to_string(), coll);
        assert_eq!(total_pages_for(&collections, 3), 2);
    }

    #[test]
    fn test_total_pages_for_partial_last_page() {
        let mut collections = HashMap::new();
        let mut coll = MarkdownCollection::new(PathBuf::from("posts"));
        for i in 0..7 {
            coll.collection.push(
                crate::md::MarkdownDocument::new(
                    &format!("---\ntitle: Post {i}\n---\ncontent"),
                    format!("post{i}.md"),
                    format!("/posts/post{i}"),
                )
                .unwrap(),
            );
        }
        collections.insert("posts".to_string(), coll);
        assert_eq!(total_pages_for(&collections, 3), 3);
    }

    #[test]
    fn test_swap_suffix_only_replaces_suffix() {
        assert_eq!("amd.html", swap_suffix("amd.md", ".md", ".html"));
        assert_eq!(
            "my-md-notes.html",
            swap_suffix("my-md-notes.md", ".md", ".html")
        );
        assert_eq!("index", swap_suffix("index.template", ".template", ""));
        assert_eq!("no-extension", swap_suffix("no-extension", ".md", ".html"));
    }

    #[test]
    fn test_partial_uri_strips_only_folder_prefix_and_extension() {
        let website = Website::new(
            None,
            PathBuf::from("templates"),
            Some(PathBuf::from("markdown")),
        );

        assert_eq!(
            "/posts/hello.html",
            website.partial_uri(Path::new("markdown/posts/hello.md"))
        );
        // file names containing the folder name or "md" must not be mangled
        assert_eq!(
            "/posts/markdown-tips.html",
            website.partial_uri(Path::new("markdown/posts/markdown-tips.md"))
        );
        assert_eq!(
            "/posts/amd.html",
            website.partial_uri(Path::new("markdown/posts/amd.md"))
        );
    }
}
