//! `roder packages init <dir>`: scaffolds a new package with all four
//! resource kinds (a skill, a slash command, a theme, and a stdlib-only
//! Python process extension adapted from
//! `examples/non-rust-extensions/python-tools`).

use std::fs;
use std::path::Path;

use anyhow::Context;
use roder_api::packages::{PACKAGE_MANIFEST_FILE, PackageSource, derive_package_id};

/// Verbatim copy of the example tool-provider extension entrypoint; it reads
/// the manifest next to it, so it works unchanged under any package id.
const EXAMPLE_EXTENSION_MAIN: &str = include_str!("templates/python_tools_main.py");

pub(super) fn run_packages_init(dir: &str) -> anyhow::Result<()> {
    let root = Path::new(dir);
    let manifest_path = root.join(PACKAGE_MANIFEST_FILE);
    anyhow::ensure!(
        !manifest_path.exists(),
        "{} already exists; refusing to overwrite an existing package",
        manifest_path.display()
    );
    let id = derive_package_id(&PackageSource::LocalPath {
        path: dir.to_string(),
    });

    write_file(&manifest_path, &roder_toml(&id))?;
    write_file(
        &root.join("skills").join("getting-started").join("SKILL.md"),
        &getting_started_skill(&id),
    )?;
    write_file(&root.join("commands").join("hello.md"), HELLO_COMMAND)?;
    write_file(
        &root.join("themes").join(format!("{id}-theme.css")),
        ":root { --accent: #7aa2f7; }\n",
    )?;
    write_file(
        &root
            .join("extensions")
            .join("example")
            .join("roder-extension.toml"),
        &extension_manifest(&id),
    )?;
    write_file(
        &root.join("extensions").join("example").join("main.py"),
        EXAMPLE_EXTENSION_MAIN,
    )?;

    println!("initialized package {id} in {}", root.display());
    for (path, what) in [
        ("roder.toml".to_string(), "package manifest"),
        ("skills/getting-started/".to_string(), "skill"),
        ("commands/hello.md".to_string(), "/hello command"),
        (format!("themes/{id}-theme.css"), "theme"),
        (
            "extensions/example/".to_string(),
            "python process extension (word_count tool)",
        ),
    ] {
        println!("  {path:<32}{what}");
    }
    println!("next steps:");
    println!("  roder install ./{dir}", dir = trimmed_dir(dir));
    println!("  roder packages approve {id}     # allow the process extension to launch");
    Ok(())
}

fn trimmed_dir(dir: &str) -> &str {
    dir.trim_start_matches("./")
}

fn write_file(path: &Path, contents: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create directory {}", parent.display()))?;
    }
    fs::write(path, contents).with_context(|| format!("write {}", path.display()))
}

fn roder_toml(id: &str) -> String {
    format!(
        r#"[package]
id = "{id}"
name = "{id}"
version = "0.1.0"
description = "A Roder package."

[resources]
extensions = ["extensions/example/roder-extension.toml"]
skills = ["skills"]
commands = ["commands"]
themes = ["themes"]
"#
    )
}

fn getting_started_skill(id: &str) -> String {
    format!(
        r#"---
name: getting-started
description: Explain what the {id} package provides and how to use it.
---

Describe the workflows this package supports. Replace this skill with real
guidance for your package, and add more skills as sibling directories
containing a SKILL.md.
"#
    )
}

const HELLO_COMMAND: &str = r#"---
description: Say hello.
argument-hint: <name>
---
Say hello to {{arguments}}.
"#;

fn extension_manifest(id: &str) -> String {
    format!(
        r#"id = "{id}-example"
name = "{id} example tools"
version = "0.1.0"
api_version = "^0.2"
description = "Process-hosted stdlib-only Python tool provider."

[launch]
command = "python3"
args = ["main.py"]

[[provides]]
type = "tool_provider"
id = "{id}-example"
tools = [
  {{ name = "word_count", description = "Count whitespace-separated words in the given text.", parameters = {{ type = "object", properties = {{ text = {{ type = "string", description = "Text to count words in." }} }}, required = ["text"] }} }},
]
"#
    )
}
