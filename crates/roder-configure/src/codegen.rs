use std::fs;
use std::path::{Path, PathBuf};

use roder_api::distribution::DistributionManifest;

use crate::catalog::Catalog;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedFile {
    pub path: PathBuf,
    pub contents: String,
}

pub fn emit(
    manifest: &DistributionManifest,
    catalog: &Catalog,
    out_dir: impl AsRef<Path>,
) -> anyhow::Result<Vec<GeneratedFile>> {
    let files = render(manifest, catalog)?;
    let out_dir = out_dir.as_ref();
    fs::create_dir_all(out_dir.join("src"))?;
    for file in &files {
        let path = out_dir.join(&file.path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, &file.contents)?;
    }
    Ok(files)
}

pub fn render(
    manifest: &DistributionManifest,
    catalog: &Catalog,
) -> anyhow::Result<Vec<GeneratedFile>> {
    let extensions = selected_extensions(manifest, catalog)?;
    let package_name = sanitize_package_name(&manifest.name);
    let cargo_dependencies = cargo_dependencies(manifest, &extensions);
    let install_extensions = install_extensions(&extensions);
    let required_env = required_env(&extensions);
    let chosen_extensions = chosen_extensions(&extensions);

    Ok(vec![
        GeneratedFile {
            path: PathBuf::from("Cargo.toml"),
            contents: render_template(
                include_str!("../templates/Cargo.toml.hbs"),
                &[
                    ("package_name", package_name.as_str()),
                    ("version", manifest.version.as_str()),
                    ("cargo_dependencies", cargo_dependencies.as_str()),
                ],
            ),
        },
        GeneratedFile {
            path: PathBuf::from("src/main.rs"),
            contents: render_template(
                include_str!("../templates/main.rs.hbs"),
                &[("install_extensions", install_extensions.as_str())],
            ),
        },
        GeneratedFile {
            path: PathBuf::from("config.toml"),
            contents: render_template(
                include_str!("../templates/config.toml.hbs"),
                &[
                    (
                        "default_provider",
                        manifest.default_provider.as_deref().unwrap_or(""),
                    ),
                    (
                        "default_thread_store",
                        manifest.default_thread_store.as_deref().unwrap_or(""),
                    ),
                    ("config_overrides", &config_overrides(manifest)),
                    ("required_env", required_env.as_str()),
                ],
            ),
        },
        GeneratedFile {
            path: PathBuf::from("README.md"),
            contents: render_template(
                include_str!("../templates/README.md.hbs"),
                &[
                    ("name", manifest.name.as_str()),
                    ("chosen_extensions", chosen_extensions.as_str()),
                    ("required_env", required_env.as_str()),
                ],
            ),
        },
    ])
}

fn selected_extensions<'a>(
    manifest: &DistributionManifest,
    catalog: &'a Catalog,
) -> anyhow::Result<Vec<&'a crate::catalog::CatalogEntry>> {
    let mut ids = manifest.extensions.clone();
    ids.sort();
    ids.dedup();
    ids.into_iter()
        .map(|id| {
            catalog
                .get(&id)
                .ok_or_else(|| anyhow::anyhow!("unknown extension `{id}`"))
        })
        .collect()
}

fn cargo_dependencies(
    manifest: &DistributionManifest,
    extensions: &[&crate::catalog::CatalogEntry],
) -> String {
    let workspace = workspace_root();
    let mut lines = vec![
        "anyhow = \"1\"".to_string(),
        format!(
            "roder-cli = {{ path = {:?} }}",
            workspace.join("crates/roder-cli").display().to_string()
        ),
    ];
    if manifest.include_tui {
        lines.push(format!(
            "roder-tui = {{ path = {:?} }}",
            workspace.join("crates/roder-tui").display().to_string()
        ));
    }
    if manifest.include_app_server {
        lines.push(format!(
            "roder-app-server = {{ path = {:?} }}",
            workspace
                .join("crates/roder-app-server")
                .display()
                .to_string()
        ));
    }
    for extension in extensions {
        let path = extension
            .manifest_path
            .parent()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| format!("../crates/{}", extension.entry.crate_name));
        lines.push(format!(
            "{} = {{ path = {:?} }}",
            extension.entry.crate_name, path
        ));
    }
    lines.sort();
    lines.join("\n")
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn install_extensions(extensions: &[&crate::catalog::CatalogEntry]) -> String {
    extensions
        .iter()
        .map(|extension| {
            format!(
                "    // install {} via {}\n",
                extension.entry.id, extension.entry.extension_path
            )
        })
        .collect::<String>()
}

fn required_env(extensions: &[&crate::catalog::CatalogEntry]) -> String {
    let mut env = extensions
        .iter()
        .flat_map(|extension| extension.entry.required_env.iter())
        .cloned()
        .collect::<Vec<_>>();
    env.sort();
    env.dedup();
    if env.is_empty() {
        "none".to_string()
    } else {
        env.into_iter()
            .map(|name| format!("- {name}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn chosen_extensions(extensions: &[&crate::catalog::CatalogEntry]) -> String {
    extensions
        .iter()
        .map(|extension| {
            format!(
                "- {} ({})",
                extension.entry.id, extension.entry.display_name
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn config_overrides(manifest: &DistributionManifest) -> String {
    if manifest.config_overrides.is_null() {
        "{}".to_string()
    } else {
        serde_json::to_string_pretty(&manifest.config_overrides)
            .unwrap_or_else(|_| "{}".to_string())
    }
}

fn sanitize_package_name(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

fn render_template(template: &str, values: &[(&str, &str)]) -> String {
    let mut rendered = template.to_string();
    for (key, value) in values {
        rendered = rendered.replace(&format!("{{{{{key}}}}}"), value);
    }
    rendered
}
