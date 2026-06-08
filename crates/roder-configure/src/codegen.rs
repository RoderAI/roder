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
    let required_env_comments = required_env_comments(&extensions);
    let chosen_extensions = chosen_extensions(&extensions);
    let provider_config = provider_config(manifest);
    let default_provider = manifest
        .default_provider
        .as_deref()
        .map(provider_runtime_id)
        .unwrap_or("mock");
    let default_model = if default_provider == "mock" {
        "mock"
    } else {
        "gpt-5.5"
    };
    let session_store_config = session_store_config(manifest);
    let config_overrides_toml = config_overrides_toml(manifest);
    let config_overrides_json = config_overrides_json(manifest);
    let main_template = if is_remote_app_server_only(manifest) {
        include_str!("../templates/remote-app-server-main.rs.hbs")
    } else {
        include_str!("../templates/main.rs.hbs")
    };

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
                main_template,
                &[
                    ("install_extensions", install_extensions.as_str()),
                    ("default_provider", default_provider),
                    ("default_model", default_model),
                ],
            ),
        },
        GeneratedFile {
            path: PathBuf::from("config.toml"),
            contents: render_template(
                include_str!("../templates/config.toml.hbs"),
                &[
                    ("provider_config", provider_config.as_str()),
                    ("session_store_config", session_store_config.as_str()),
                    ("config_overrides_toml", config_overrides_toml.as_str()),
                    ("config_overrides_json", config_overrides_json.as_str()),
                    ("required_env_comments", required_env_comments.as_str()),
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
    let mut lines = vec!["anyhow = \"1\"".to_string()];
    if manifest.include_cli {
        lines.push(format!(
            "roder-cli = {{ path = {:?} }}",
            workspace.join("crates/roder-cli").display().to_string()
        ));
    }
    if is_remote_app_server_only(manifest) {
        lines.extend([
            format!(
                "roder-api = {{ path = {:?} }}",
                workspace.join("crates/roder-api").display().to_string()
            ),
            format!(
                "roder-config = {{ path = {:?} }}",
                workspace.join("crates/roder-config").display().to_string()
            ),
            format!(
                "roder-core = {{ path = {:?} }}",
                workspace.join("crates/roder-core").display().to_string()
            ),
            format!(
                "roder-extension-host = {{ path = {:?} }}",
                workspace
                    .join("crates/roder-extension-host")
                    .display()
                    .to_string()
            ),
            "tokio = { version = \"1\", features = [\"macros\", \"rt-multi-thread\"] }".to_string(),
        ]);
    }
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

fn is_remote_app_server_only(manifest: &DistributionManifest) -> bool {
    manifest.include_app_server && !manifest.include_tui && !manifest.include_cli
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

fn required_env_comments(extensions: &[&crate::catalog::CatalogEntry]) -> String {
    required_env(extensions)
        .lines()
        .map(|line| format!("# {line}"))
        .collect::<Vec<_>>()
        .join("\n")
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

fn provider_config(manifest: &DistributionManifest) -> String {
    manifest
        .default_provider
        .as_deref()
        .map(provider_runtime_id)
        .map(|provider| format!("provider = {provider:?}\n"))
        .unwrap_or_default()
}

fn provider_runtime_id(default_provider: &str) -> &str {
    match default_provider {
        "openai-responses" => "openai",
        "anthropic" => "anthropic",
        "gemini" => "gemini",
        "xai" => "xai",
        "opencode" => "opencode",
        "poolside" => "poolside",
        "cursor" => "cursor",
        "xiaomi-mimo" => "xiaomi-mimo",
        _ => default_provider,
    }
}

fn session_store_config(manifest: &DistributionManifest) -> String {
    manifest
        .default_thread_store
        .as_deref()
        .map(thread_store_runtime_id)
        .map(|store| format!("[sessions]\nstore = {store:?}\n"))
        .unwrap_or_default()
}

fn thread_store_runtime_id(default_thread_store: &str) -> &str {
    match default_thread_store {
        "jsonl-thread-store" => "jsonl",
        "postgres-session" => "postgres",
        _ => default_thread_store,
    }
}

fn config_overrides_toml(manifest: &DistributionManifest) -> String {
    if manifest.config_overrides.is_null() {
        return String::new();
    }
    toml::to_string_pretty(&manifest.config_overrides).unwrap_or_default()
}

fn config_overrides_json(manifest: &DistributionManifest) -> String {
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
