# Squid

Squid is a static website generator written in Rust that uses the template language "TinyLang".

## Features

- **GitHub Flavored Markdown** — tables, strikethrough, footnotes, task lists, and raw HTML pass-through
- **Syntax highlighting** for fenced code blocks (configurable theme, no extra CSS needed)
- **Collections** — every folder of markdown files becomes a collection rendered by its `_<name>.template`
- **Tags** — declare `tags: rust, web` in frontmatter; a `_tag.template` generates `/tags/<slug>.html` pages, and every template can read the global `tags` list
- **Pagination** — set `posts_per_page` and use the `pagination` object in templates
- **Drafts & scheduled posts** — `draft: true` or a future `date` keeps a post out of the build (preview them with `--drafts`)
- **Date-sorted collections** with `next`/`previous` post navigation
- **RSS feed and sitemap.xml** generated automatically
- **Pretty URLs** — opt into `/posts/my-post/` style urls with `pretty_urls = true`
- **Dev server with live reload** — `--serve 8080` rebuilds on change and refreshes the browser
- **Incremental rebuilds** — a dependency graph rebuilds only the affected pages while watching

## TinyLang

TinyLang is a lightweight template language that is easy to learn and use. It was designed specifically for Squid,
 but it can also be used independently in other projects. TinyLang has a simple syntax that is similar to other popular template languages like Handlebars and Mustache.

If you're interested in learning more about TinyLang, you can check out the [GitHub repository](https://github.com/era/tinylang) for the project.

### Built-in template functions

| Function | Example | Description |
| -------- | ------- | ----------- |
| `render` | `{{ render('templates/_header.template') }}` | include another template |
| `sort_by_key` | `{{ sort_by_key(posts.items, 'title') }}` | sort objects by a key (add `'reversed'` to invert) |
| `reverse` | `{{ reverse(posts.items) }}` | reverse an array |
| `paginate` | `{{ paginate(posts.items, 2, 10) }}` | slice a page out of an array |
| `where` | `{{ where(posts.items, 'author', 'era') }}` | filter objects by a key's value |
| `limit` | `{{ limit(posts.items, 5) }}` | first n items |
| `group_by` | `{{ group_by(posts.items, 'author') }}` | group objects: `[{key, items}, ...]` |
| `date_format` | `{{ date_format(content.date, '%B %d, %Y') }}` | format a frontmatter date |
| `slugify` | `{{ slugify(content.title) }}` | url-friendly slug |
| `truncate` | `{{ truncate(content.title, 50) }}` | shorten a string with an ellipsis |

### Template state

Every template sees the config values (`website_name`, `uri`, everything in `[custom_keys]`),
one object per collection (e.g. `posts.items`, `posts.size`), and the global `tags` list.
Collection partials (`_posts.template`) additionally get `content` (the current document:
frontmatter keys, `content`, `partial_uri`, `next`, `previous`). The reserved `_tag.template`
gets `tag` (`name`, `slug`, `uri`, `size`, `items`). Paginated templates get `pagination`
(`current_page`, `total_pages`, `has_next`, `has_prev`, `next_file`, `prev_file`).

## Getting Started

To get started with Squid, you'll need to have Rust installed on your computer. Once you have Rust installed, you can clone the Squid repository and build the project using Cargo:

```sh
git clone https://github.com/example/squid.git
cd squid
cargo build --release
```

Once the project is built, you can run it using the following command:

```sh
./target/release/squid --template-folder templates --markdown-folder my_posts --template-variables config.toml --output-folder content
```

This will generate a new website in the `output` directory using the templates and content from the `templates` and `content` directories.

If you want to see an usage example, check the `tests/integration.rs`. The templates are at `tests/templates` and the expected output is in `tests/output`.

## Commands

To set up a new website in the current directory:

```sh
squid init
```

This creates the `markdown/`, `templates/`, `static/`, and `output/` folders along with a `config.toml` and some sample templates.

To create a new post:

```sh
squid new posts my-post-title
```

This creates `my-post-title.md` inside the `posts` subfolder of your markdown directory.

To preview your site locally with live reload (drafts included):

```sh
squid --template-folder templates --markdown-folder markdown --static-resources static \
      --output-folder output --template-variables config.toml --watch --serve 8080 --drafts
```

## Configuration

```toml
website_name = "My Website"
uri = "https://example.com"

# optional
pretty_urls = true            # /posts/my-post/ instead of /posts/my-post.html
posts_per_page = 10           # paginate listings
code_theme = "InspiredGitHub" # syntax highlighting theme, "none" disables

[custom_keys]                 # exposed to every template
description = "A website built with Squid"
language = "en-us"
```

Available `code_theme` values: `InspiredGitHub`, `Solarized (dark)`, `Solarized (light)`,
`base16-eighties.dark`, `base16-mocha.dark`, `base16-ocean.dark`, `base16-ocean.light`, `none`.

## Frontmatter

```markdown
---
title: My Post
date: 2024-01-10            # also accepts RFC 3339 / RFC 2822
tags: rust, web             # or a YAML list
draft: true                 # excluded from builds unless --drafts
excerpt: Short description  # used in the RSS feed
---
```

## Contributing

Squid is still under development, and contributions are always welcome. If you find a bug or have an idea for a new feature, please open an issue on the GitHub repository.

If you want to contribute to the project, you can fork the repository and make your changes on a new branch. Once you're done, you can submit a pull request to have your changes reviewed and merged into the main branch.

## License

Squid is licensed under the MIT license. See the `LICENSE` file for more information.
