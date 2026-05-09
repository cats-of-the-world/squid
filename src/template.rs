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
    pagination.insert("total_pages".into(), TinyLangType::Numeric(total_pages as f64));
    pagination.insert("per_page".into(), TinyLangType::Numeric(per_page as f64));
    pagination.insert("has_next".into(), TinyLangType::Bool(page < total_pages));
    pagination.insert("has_prev".into(), TinyLangType::Bool(page > 1));
    pagination.insert("next_file".into(), next_file);
    pagination.insert("prev_file".into(), prev_file);
    state.insert("pagination".into(), TinyLangType::Object(pagination));

    state
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
    ((max_items + per_page - 1) / per_page).max(1)
}

struct Builder {
    tinylang_state: Arc<State>,
    output_folder: PathBuf,
    eval_tasks: Option<JoinSet<String>>,
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

    fn build_paginated_state(&self, base_name: &str, page: usize, total_pages: usize, per_page: usize) -> State {
        paginated_state(&self.tinylang_state, &self.collections, base_name, page, total_pages, per_page)
    }

    async fn process_folder(
        &mut self,
        eval_tasks: JoinSet<String>,
        template_folder_reader: &mut LazyFolderReader,
        collections: &HashMap<String, MarkdownCollection>,
    ) {
        self.eval_tasks = Some(eval_tasks);

        while let Some(file) = template_folder_reader.async_next().await {
            let file = file.unwrap();

            // should handle _name.template differently since they are partials
            // for the rest of the templates we should generate a single output with same name
            if file.name.starts_with('_') {
                // removing the first _
                // and the .template from the end
                // this is safe because we filtered based on the extension name ('.template')
                let collection_name = &file.name[1..file.name.len() - 9];
                if let Some(collection) = collections.get(collection_name) {
                    self.eval_markdown_collection_to_output_file(collection.clone(), file)
                        .await;
                }
                continue;
            }

            self.eval_template_to_output_file(file);
        }
    }

    /// build a template without any markdown
    fn eval_template_to_output_file(&mut self, file: TemplateFile) {
        let base_name = file.name.replace(".template", "");

        match self.posts_per_page {
            Some(per_page) if per_page > 0 => {
                let total_pages = self.total_pages(per_page);
                for page in 1..=total_pages {
                    let file_name = page_file_name(&base_name, page);
                    let state = self.build_paginated_state(&base_name, page, total_pages, per_page);
                    let output_folder = self.output_folder.to_path_buf();
                    let contents = file.contents.clone();
                    self.eval_tasks.as_mut().unwrap().spawn(async move {
                        let html = eval(&contents, state).unwrap();
                        io::write_to_disk(output_folder, &file_name, html).await;
                        file_name
                    });
                }
            }
            _ => {
                let output_folder = self.output_folder.to_path_buf();
                let state = self.tinylang_state.clone();
                self.eval_tasks.as_mut().unwrap().spawn(async move {
                    let file_name = format!("{base_name}.html");
                    let html = {
                        let state = (*state).clone();
                        eval(&file.contents, state).unwrap()
                    };
                    io::write_to_disk(output_folder, &file_name, html).await;
                    file_name
                });
            }
        }
    }

    async fn mk_collection_dir(&mut self, collection: &MarkdownCollection) -> PathBuf {
        let collection_name = collection.relative_path.clone();
        let collection_name = collection_name
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();

        let output_folder = self.output_folder.to_path_buf();
        let output_folder = output_folder.join(&collection_name);

        if !output_folder.exists() {
            create_dir(&output_folder).await.unwrap();
        }
        output_folder
    }

    /// builds a collection of markdown files using the appropriate template
    async fn eval_markdown_collection_to_output_file(
        &mut self,
        collection: MarkdownCollection,
        template: TemplateFile,
    ) {
        let output_folder = self.mk_collection_dir(&collection).await;

        // we need for each item in the collection
        // to evaluate the template using its header and content
        for item in collection.collection {
            let output_folder = output_folder.clone();

            let state = self.tinylang_state.clone();

            let template = template.clone();

            self.eval_tasks.as_mut().unwrap().spawn(async move {
                let html = {
                    let mut state = (*state).clone();

                    state.insert("content".into(), item.as_tinylang_state().into());

                    eval(&template.contents, state).unwrap()
                };

                // we need to save our file following the markdown file and not the template
                let file_name = item.name.replace(".md", ".html");

                io::write_to_disk(output_folder, &file_name, html).await;

                file_name
            });
        }
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

    pub async fn build_from_scratch(&mut self, output: &Path) -> Result<JoinSet<String>> {
        if output.exists() {
            remove_dir_all(output)
                .await
                .with_context(|| format!("failed to clean output folder '{}'", output.display()))?;
        }
        create_dir_all(output)
            .await
            .with_context(|| format!("failed to create output folder '{}'", output.display()))?;

        let collections = self.build_markdown_collections().await?;

        if let Some(c) = self.configuration.clone() {
            let feed_config = FeedConfig {
                title: c.website_name.clone(),
                description: c
                    .custom_keys
                    .get("description")
                    .cloned()
                    .unwrap_or_else(|| format!("Latest posts from {}", c.website_name)),
                website_url: c.uri.clone(),
                feed_url: format!("{}/rss.xml", c.uri),
                language: c
                    .custom_keys
                    .get("language")
                    .cloned()
                    .unwrap_or_else(|| "en-us".to_string()),
            };
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
    ) -> Result<JoinSet<String>> {
        let collections = self.build_markdown_collections().await?;

        // Refresh RSS with updated collection data
        if let Some(c) = self.configuration.clone() {
            let feed_config = FeedConfig {
                title: c.website_name.clone(),
                description: c
                    .custom_keys
                    .get("description")
                    .cloned()
                    .unwrap_or_else(|| format!("Latest posts from {}", c.website_name)),
                website_url: c.uri.clone(),
                feed_url: format!("{}/rss.xml", c.uri),
                language: c
                    .custom_keys
                    .get("language")
                    .cloned()
                    .unwrap_or_else(|| "en-us".to_string()),
            };
            let all_posts: Vec<_> = collections
                .values()
                .flat_map(|col| {
                    col.collection
                        .iter()
                        .filter_map(|d| d.to_post_metadata().ok())
                })
                .collect();
            generate_rss(&feed_config, &all_posts, output).context("Failed to generate RSS")?;
        }

        let state = self.build_state(&collections);

        // Collect the rebuild plan from the dep graph before releasing borrows.
        // Each markdown entry: (partial_template_path, output_path, item_tinylang_state)
        // Each standalone entry: (template_path, output_path, page_number)
        let (markdown_items, standalone_items): (
            Vec<(PathBuf, PathBuf, tinylang::types::State)>,
            Vec<(PathBuf, PathBuf, Option<usize>)>,
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

            let standalone_items: Vec<(PathBuf, PathBuf, Option<usize>)> = deps
                .standalones()
                .map(|(t, o)| (t.clone(), o.clone(), deps.page_for_output(o)))
                .collect();

            (markdown_items, standalone_items)
        };

        let mut eval_tasks = JoinSet::new();

        for (template_path, output_path, item_state) in markdown_items {
            let template = TemplateFile::new(&template_path)?;
            let output_dir = output_path.parent().unwrap().to_path_buf();
            let file_name = output_path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string();
            let mut s = state.clone();
            s.insert("content".into(), item_state.into());
            eval_tasks.spawn(async move {
                let html = eval(&template.contents, s).unwrap();
                io::write_to_disk(output_dir, &file_name, html).await;
                file_name
            });
        }

        let posts_per_page = self.configuration.as_ref().and_then(|c| c.posts_per_page);

        for (template_path, output_path, page_opt) in standalone_items {
            let template = TemplateFile::new(&template_path)?;
            let output_dir = output_path.parent().unwrap().to_path_buf();
            let file_name = output_path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string();
            let s = match (posts_per_page.filter(|&p| p > 0), page_opt) {
                (Some(per_page), Some(page)) => {
                    let total = total_pages_for(&collections, per_page);
                    let base_name = template_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .replace(".template", "");
                    paginated_state(&state, &collections, &base_name, page, total, per_page)
                }
                _ => state.clone(),
            };
            eval_tasks.spawn(async move {
                let html = eval(&template.contents, s).unwrap();
                io::write_to_disk(output_dir, &file_name, html).await;
                file_name
            });
        }

        Ok(eval_tasks)
    }

    pub async fn compile_templates(&mut self) -> Result<JoinSet<String>> {
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
            .await;

        let output_folder = self
            .cache
            .builder
            .as_ref()
            .context("compile_templates called without caching builder")?
            .output_folder
            .clone();
        let result = Ok(self
            .cache
            .builder
            .as_mut()
            .context("compile_templates called without caching builder")?
            .eval_tasks
            .take()
            .unwrap());
        self.build_dependency_graph(&output_folder).await?;
        result
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
                let base_name = file.name.replace(".template", "");
                let total = total_pages_for(collections, per_page);
                for page in 1..=total {
                    let output_name = page_file_name(&base_name, page);
                    let output_path = output_folder.join(&output_name);
                    deps.register_standalone(file.path.clone(), &output_name);
                    deps.register_page_number(output_path, page);
                }
            } else {
                let output_name = file.name.replace(".template", ".html");
                deps.register_standalone(file.path.clone(), &output_name);
            }
        }

        for (collection_name, collection) in collections {
            let output_dir = output_folder.join(
                collection
                    .relative_path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .as_ref(),
            );
            for item in &collection.collection {
                let md_path = collection.relative_path.join(&item.name);
                let output_name = item.name.replace(".md", ".html");
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
    ) -> Result<Option<JoinSet<String>>> {
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
                let output_folder = output_path.parent().unwrap().to_path_buf();
                let file_name = output_path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_string();
                let eval_state = match (
                    posts_per_page.filter(|&p| p > 0),
                    deps.page_for_output(&output_path),
                ) {
                    (Some(per_page), Some(page)) => {
                        let total = total_pages_for(collections, per_page);
                        let base_name = template_path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("")
                            .replace(".template", "");
                        paginated_state(state, collections, &base_name, page, total, per_page)
                    }
                    _ => state.clone(),
                };
                eval_tasks.spawn(async move {
                    let html = eval(&template.contents, eval_state).unwrap();
                    io::write_to_disk(output_folder, &file_name, html).await;
                    file_name
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
                let output_folder = output_path.parent().unwrap().to_path_buf();
                let file_name = output_path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_string();
                let mut state = state.clone();
                state.insert("content".into(), item.as_tinylang_state().into());
                eval_tasks.spawn(async move {
                    let html = eval(&template.contents, state).unwrap();
                    io::write_to_disk(output_folder, &file_name, html).await;
                    file_name
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

            //TODO avoid unwrap
            let path_as_string = path.file_name().unwrap().to_string_lossy();

            let collection = collections
                .entry(path_as_string.to_string())
                .or_insert(MarkdownCollection::new(path));

            collection.collection.push(markdown_content);
        }

        self.cache.collections = Some(collections.clone());
        Ok(collections)
    }

    fn partial_uri(&self, path: &Path) -> String {
        path.to_string_lossy()
            .to_string()
            // we leave only the relative path after the `posts_folder` to avoid
            // creating a url with a local path (e.g. $HOME/my_site/posts)
            .replace(
                self.posts_folder
                    .as_ref()
                    .unwrap()
                    .to_string_lossy()
                    .as_ref(),
                "",
            )
            .replace("md", "html")
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
        state.extend(self.build_collection_state(collections).into_iter());
        self.cache.state = Some(state.clone());
        state
    }
}
