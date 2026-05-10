# Squid

Squid is a static website generator written in Rust that uses the template language "TinyLang".

The project is still under development.

## TinyLang

TinyLang is a lightweight template language that is easy to learn and use. It was designed specifically for Squid,
 but it can also be used independently in other projects. TinyLang has a simple syntax that is similar to other popular template languages like Handlebars and Mustache.

If you're interested in learning more about TinyLang, you can check out the [GitHub repository](https://github.com/era/tinylang) for the project.

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

## Contributing

Squid is still under development, and contributions are always welcome. If you find a bug or have an idea for a new feature, please open an issue on the GitHub repository.

If you want to contribute to the project, you can fork the repository and make your changes on a new branch. Once you're done, you can submit a pull request to have your changes reviewed and merged into the main branch.

## License

Squid is licensed under the MIT license. See the `LICENSE` file for more information.
