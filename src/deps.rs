//! Dependency tracking for incremental builds.
//!
//! Tracks which outputs depend on which source files, so we can rebuild only
//! what's affected when a file changes.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

static RENDER_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r#"render\s*\(\s*['"]([^'"]+)['"]\s*\)"#).unwrap());

/// Type of source file that can change.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FileChangeType {
    Template,
    Markdown,
    Config,
    Static,
}

/// A file change event with the specific path(s) that changed.
#[derive(Debug, Clone)]
pub struct FileChangeEvent {
    pub change_type: FileChangeType,
    pub paths: Vec<PathBuf>,
}

/// Dependency graph for incremental builds.
///
/// Tracks:
/// - Template A includes template B via render() → when B changes, A's output must rebuild
/// - Partial _X.template renders collection X → when partial changes, all X outputs rebuild
/// - Markdown in collection X → when md changes, only that md's output rebuilds
/// - Config affects global state → full rebuild
pub struct DependencyGraph {
    template_folder: PathBuf,
    output_folder: PathBuf,
    /// Templates that include this template (reverse deps). When key changes, values must rebuild.
    reverse_template_deps: HashMap<PathBuf, HashSet<PathBuf>>,
    /// Collection name -> partial template path
    collection_partials: HashMap<String, PathBuf>,
    /// Partial template path -> collection name (reverse lookup)
    partial_to_collection: HashMap<PathBuf, String>,
    /// Markdown path -> (collection_name, output_path)
    markdown_outputs: HashMap<PathBuf, (String, PathBuf)>,
    /// Standalone template path -> all output paths (one per page when paginated)
    standalone_outputs: HashMap<PathBuf, Vec<PathBuf>>,
    /// Output path -> template path (for standalone, reverse lookup)
    output_to_template: HashMap<PathBuf, PathBuf>,
    /// Output path -> 1-based page number (only set for paginated outputs)
    output_page: HashMap<PathBuf, usize>,
    /// Output path -> (markdown_path, collection_name) for collection outputs
    output_to_markdown: HashMap<PathBuf, (PathBuf, String)>,
    /// All templates (for transitive closure)
    all_templates: HashSet<PathBuf>,
}

impl DependencyGraph {
    pub fn new(template_folder: PathBuf, output_folder: PathBuf) -> Self {
        Self {
            template_folder,
            output_folder,
            reverse_template_deps: HashMap::new(),
            collection_partials: HashMap::new(),
            partial_to_collection: HashMap::new(),
            markdown_outputs: HashMap::new(),
            standalone_outputs: HashMap::new(),
            output_to_template: HashMap::new(),
            output_to_markdown: HashMap::new(),
            all_templates: HashSet::new(),
            output_page: HashMap::new(),
        }
    }

    /// Extract render() calls from template content. Matches render('path') and render("path").
    fn parse_render_calls(content: &str) -> Vec<String> {
        RENDER_RE
            .captures_iter(content)
            .map(|cap| cap[1].to_string())
            .collect()
    }

    /// Resolve a path from a render() call to an absolute path.
    fn resolve_render_path(
        render_path: &str,
        template_file: &Path,
        template_folder: &Path,
        base_dir: &Path,
    ) -> Option<PathBuf> {
        let path = Path::new(render_path);
        // Try as absolute first
        if path.is_absolute() {
            return Some(path.to_path_buf());
        }
        // Try relative to template folder
        let candidate = template_folder.join(path);
        if candidate.exists() {
            return Some(candidate.canonicalize().unwrap_or(candidate));
        }
        // Try relative to base dir (cwd)
        let candidate = base_dir.join(path);
        if candidate.exists() {
            return Some(candidate.canonicalize().unwrap_or(candidate));
        }
        // Try relative to template file's parent
        if let Some(parent) = template_file.parent() {
            let candidate = parent.join(path);
            if candidate.exists() {
                return Some(candidate.canonicalize().unwrap_or(candidate));
            }
        }
        None
    }

    /// Register a template and its dependencies from parsing its content.
    pub fn register_template(&mut self, template_path: PathBuf, content: &str, base_dir: &Path) {
        let template_path = template_path.canonicalize().unwrap_or(template_path);
        self.all_templates.insert(template_path.clone());

        for render_path in Self::parse_render_calls(content) {
            if let Some(dep_path) = Self::resolve_render_path(
                &render_path,
                &template_path,
                &self.template_folder,
                base_dir,
            ) {
                let dep_path = dep_path.canonicalize().unwrap_or(dep_path);
                self.reverse_template_deps
                    .entry(dep_path)
                    .or_default()
                    .insert(template_path.clone());
            }
        }
    }

    /// Register a standalone template output. Call once per output file (multiple times for paginated templates).
    pub fn register_standalone(&mut self, template_path: PathBuf, output_name: &str) {
        let template_path = template_path.canonicalize().unwrap_or(template_path);
        self.all_templates.insert(template_path.clone());
        let output_path = self.output_folder.join(output_name);
        self.standalone_outputs
            .entry(template_path.clone())
            .or_default()
            .push(output_path.clone());
        self.output_to_template.insert(output_path, template_path);
    }

    /// Mark an output as belonging to a specific pagination page (1-based).
    pub fn register_page_number(&mut self, output_path: PathBuf, page: usize) {
        self.output_page.insert(output_path, page);
    }

    /// Return the 1-based page number for a paginated output, or None if not paginated.
    pub fn page_for_output(&self, path: &Path) -> Option<usize> {
        self.output_page.get(path).copied()
    }

    /// Register a collection partial (produces one output per markdown in collection).
    pub fn register_collection_partial(&mut self, collection_name: &str, template_path: PathBuf) {
        let template_path = template_path.canonicalize().unwrap_or(template_path);
        self.all_templates.insert(template_path.clone());
        self.collection_partials
            .insert(collection_name.to_string(), template_path.clone());
        self.partial_to_collection
            .insert(template_path, collection_name.to_string());
    }

    /// Register a markdown file's output.
    pub fn register_markdown_output(
        &mut self,
        markdown_path: PathBuf,
        collection_name: &str,
        output_path: PathBuf,
    ) {
        let markdown_path = markdown_path.canonicalize().unwrap_or(markdown_path);
        self.markdown_outputs.insert(
            markdown_path.clone(),
            (collection_name.to_string(), output_path.clone()),
        );
        self.output_to_markdown
            .insert(output_path, (markdown_path, collection_name.to_string()));
    }

    /// Find all output paths that need to be rebuilt when the given file changes.
    pub fn affected_outputs(&self, change: &FileChangeEvent) -> HashSet<PathBuf> {
        let mut outputs = HashSet::new();

        for path in &change.paths {
            let path = path.canonicalize().unwrap_or(path.clone());

            match &change.change_type {
                FileChangeType::Config => {
                    // Config change: full rebuild - return empty to signal full rebuild
                    return HashSet::new();
                }
                FileChangeType::Static => {
                    // Static: caller copies files. No template outputs to rebuild.
                }
                FileChangeType::Markdown => {
                    if let Some((_, output_path)) = self.markdown_outputs.get(&path) {
                        outputs.insert(output_path.clone());
                    }
                }
                FileChangeType::Template => {
                    // 1. Outputs from this template directly (standalone or partial)
                    if let Some(outs) = self.standalone_outputs.get(&path) {
                        outputs.extend(outs.iter().cloned());
                    }
                    if let Some(coll_name) = self.partial_to_collection.get(&path) {
                        for (_, (_, out)) in self
                            .markdown_outputs
                            .iter()
                            .filter(|(_, (c, _))| c == coll_name)
                        {
                            outputs.insert(out.clone());
                        }
                    }
                    // 2. Templates that include this one (transitive)
                    let mut to_check = vec![path.clone()];
                    let mut checked = HashSet::new();
                    while let Some(check_path) = to_check.pop() {
                        if !checked.insert(check_path.clone()) {
                            continue;
                        }
                        if let Some(dependents) = self.reverse_template_deps.get(&check_path) {
                            for dep in dependents {
                                to_check.push(dep.clone());
                                // Add outputs for these dependent templates
                                if let Some(outs) = self.standalone_outputs.get(dep) {
                                    outputs.extend(outs.iter().cloned());
                                }
                                if let Some(coll_name) = self.partial_to_collection.get(dep) {
                                    for (_, (_, out)) in self
                                        .markdown_outputs
                                        .iter()
                                        .filter(|(_, (c, _))| c == coll_name)
                                    {
                                        outputs.insert(out.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        outputs
    }

    /// Returns true if the change requires a full rebuild (e.g. config change).
    pub fn requires_full_rebuild(&self, change: &FileChangeEvent) -> bool {
        change.change_type == FileChangeType::Config
    }

    /// Returns true if this is a static file change (handled by copy, not template build).
    pub fn is_static_change(&self, change: &FileChangeEvent) -> bool {
        change.change_type == FileChangeType::Static
    }

    /// Get the template path that produces this output (for standalone templates).
    pub fn template_for_output(&self, output_path: &Path) -> Option<PathBuf> {
        self.output_to_template.get(output_path).cloned()
    }

    /// Get the (markdown path, collection name) for a collection output.
    pub fn markdown_for_output(&self, output_path: &Path) -> Option<(PathBuf, String)> {
        self.output_to_markdown.get(output_path).cloned()
    }

    /// Get the partial template path for a collection.
    pub fn partial_for_collection(&self, collection_name: &str) -> Option<PathBuf> {
        self.collection_partials.get(collection_name).cloned()
    }

    /// Returns true if the given markdown path is tracked in the dependency graph.
    pub fn knows_markdown(&self, path: &Path) -> bool {
        self.markdown_outputs.contains_key(path)
    }

    /// Iterate all standalone template → output path pairs (one per output file).
    pub fn standalones(&self) -> impl Iterator<Item = (&PathBuf, &PathBuf)> {
        self.standalone_outputs
            .iter()
            .flat_map(|(template, outputs)| outputs.iter().map(move |out| (template, out)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_graph() -> DependencyGraph {
        DependencyGraph::new(PathBuf::from("/templates"), PathBuf::from("/output"))
    }

    #[test]
    fn test_parse_render_calls_single_quotes() {
        let calls = DependencyGraph::parse_render_calls("{{ render('partial.html') }}");
        assert_eq!(calls, vec!["partial.html"]);
    }

    #[test]
    fn test_parse_render_calls_double_quotes() {
        let calls = DependencyGraph::parse_render_calls(r#"{{ render("header.html") }}"#);
        assert_eq!(calls, vec!["header.html"]);
    }

    #[test]
    fn test_parse_render_calls_multiple() {
        let calls = DependencyGraph::parse_render_calls(
            "{{ render('header.html') }} body {{ render('footer.html') }}",
        );
        assert_eq!(calls.len(), 2);
        assert!(calls.contains(&"header.html".to_string()));
        assert!(calls.contains(&"footer.html".to_string()));
    }

    #[test]
    fn test_parse_render_calls_none() {
        assert!(DependencyGraph::parse_render_calls("<h1>plain content</h1>").is_empty());
    }

    #[test]
    fn test_requires_full_rebuild_config() {
        let graph = make_graph();
        let change = FileChangeEvent {
            change_type: FileChangeType::Config,
            paths: vec![PathBuf::from("/config.toml")],
        };
        assert!(graph.requires_full_rebuild(&change));
    }

    #[test]
    fn test_requires_full_rebuild_template_is_false() {
        let graph = make_graph();
        let change = FileChangeEvent {
            change_type: FileChangeType::Template,
            paths: vec![PathBuf::from("/templates/index.template")],
        };
        assert!(!graph.requires_full_rebuild(&change));
    }

    #[test]
    fn test_is_static_change() {
        let graph = make_graph();
        let static_change = FileChangeEvent {
            change_type: FileChangeType::Static,
            paths: vec![PathBuf::from("/static/style.css")],
        };
        assert!(graph.is_static_change(&static_change));
        let template_change = FileChangeEvent {
            change_type: FileChangeType::Template,
            paths: vec![PathBuf::from("/templates/index.template")],
        };
        assert!(!graph.is_static_change(&template_change));
    }

    #[test]
    fn test_affected_outputs_markdown_change() {
        let mut graph = make_graph();
        let md = PathBuf::from("/markdown/posts/post1.md");
        let out = PathBuf::from("/output/posts/post1.html");
        graph.register_markdown_output(md.clone(), "posts", out.clone());

        let change = FileChangeEvent {
            change_type: FileChangeType::Markdown,
            paths: vec![md],
        };
        let affected = graph.affected_outputs(&change);
        assert_eq!(affected.len(), 1);
        assert!(affected.contains(&out));
    }

    #[test]
    fn test_affected_outputs_unregistered_markdown_is_empty() {
        let graph = make_graph();
        let change = FileChangeEvent {
            change_type: FileChangeType::Markdown,
            paths: vec![PathBuf::from("/markdown/posts/unknown.md")],
        };
        assert!(graph.affected_outputs(&change).is_empty());
    }

    #[test]
    fn test_affected_outputs_static_is_empty() {
        let graph = make_graph();
        let change = FileChangeEvent {
            change_type: FileChangeType::Static,
            paths: vec![PathBuf::from("/static/style.css")],
        };
        assert!(graph.affected_outputs(&change).is_empty());
    }

    #[test]
    fn test_affected_outputs_standalone_template() {
        let mut graph = make_graph();
        let template = PathBuf::from("/templates/index.template");
        graph.register_standalone(template.clone(), "index.html");

        let change = FileChangeEvent {
            change_type: FileChangeType::Template,
            paths: vec![template],
        };
        let affected = graph.affected_outputs(&change);
        assert!(affected.contains(&PathBuf::from("/output/index.html")));
    }

    #[test]
    fn test_affected_outputs_partial_change_rebuilds_collection() {
        let mut graph = make_graph();
        let partial = PathBuf::from("/templates/_posts.template");
        let md = PathBuf::from("/markdown/posts/post1.md");
        let out = PathBuf::from("/output/posts/post1.html");
        graph.register_collection_partial("posts", partial.clone());
        graph.register_markdown_output(md, "posts", out.clone());

        let change = FileChangeEvent {
            change_type: FileChangeType::Template,
            paths: vec![partial],
        };
        assert!(graph.affected_outputs(&change).contains(&out));
    }

    #[test]
    fn test_template_for_output() {
        let mut graph = make_graph();
        let template = PathBuf::from("/templates/index.template");
        graph.register_standalone(template.clone(), "index.html");

        assert_eq!(
            graph.template_for_output(&PathBuf::from("/output/index.html")),
            Some(template)
        );
        assert!(graph
            .template_for_output(&PathBuf::from("/output/other.html"))
            .is_none());
    }

    #[test]
    fn test_markdown_for_output() {
        let mut graph = make_graph();
        let md = PathBuf::from("/markdown/posts/post1.md");
        let out = PathBuf::from("/output/posts/post1.html");
        graph.register_markdown_output(md.clone(), "posts", out.clone());

        let (md_back, coll) = graph.markdown_for_output(&out).unwrap();
        assert_eq!(md_back, md);
        assert_eq!(coll, "posts");
        assert!(graph
            .markdown_for_output(&PathBuf::from("/output/posts/other.html"))
            .is_none());
    }

    #[test]
    fn test_partial_for_collection() {
        let mut graph = make_graph();
        let partial = PathBuf::from("/templates/_posts.template");
        graph.register_collection_partial("posts", partial.clone());

        assert_eq!(graph.partial_for_collection("posts"), Some(partial));
        assert!(graph.partial_for_collection("nonexistent").is_none());
    }

    #[test]
    fn test_knows_markdown() {
        let mut graph = make_graph();
        let md = PathBuf::from("/markdown/posts/post1.md");
        graph.register_markdown_output(
            md.clone(),
            "posts",
            PathBuf::from("/output/posts/post1.html"),
        );
        assert!(graph.knows_markdown(&md));
        assert!(!graph.knows_markdown(&PathBuf::from("/markdown/posts/other.md")));
    }

    #[test]
    fn test_page_number_registration() {
        let mut graph = make_graph();
        let out = PathBuf::from("/output/index-page-2.html");
        graph.register_page_number(out.clone(), 2);

        assert_eq!(graph.page_for_output(&out), Some(2));
        assert!(graph
            .page_for_output(&PathBuf::from("/output/index.html"))
            .is_none());
    }

    #[test]
    fn test_standalones_count() {
        let mut graph = make_graph();
        graph.register_standalone(PathBuf::from("/templates/index.template"), "index.html");
        graph.register_standalone(PathBuf::from("/templates/about.template"), "about.html");
        assert_eq!(graph.standalones().count(), 2);
    }
}
