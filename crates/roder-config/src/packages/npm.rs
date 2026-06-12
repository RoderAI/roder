//! npm fetcher: installs one package into a staging prefix with lifecycle
//! scripts disabled by default, then swaps the package tree into the store.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context;

use super::fsutil::{copy_dir_recursive, staging_sibling, swap_dir_into_place, unique_suffix};

pub(crate) const DEFAULT_NPM_COMMAND: &str = "npm";

#[derive(Debug, Default)]
pub(crate) struct NpmFetchOutcome {
    /// Version reported by the installed `package.json`.
    pub resolved_version: Option<String>,
    pub warnings: Vec<String>,
}

/// Runs `<npm_command> install <name>@<version-or-latest> --ignore-scripts
/// --omit=dev --no-audit --no-fund --prefix <staging>` (dropping
/// `--ignore-scripts` only when `allow_scripts`), then materializes the
/// package at `store_path`.
///
/// Store layout: `store_path` is the package's own tree (the staged
/// `node_modules/<name>` moved into place). Any sibling dependencies npm
/// hoisted into the staging `node_modules` are copied into
/// `store_path/node_modules/` so `require()` resolution keeps working from
/// inside the package without keeping the whole staging prefix around.
pub(crate) fn npm_fetch_into_store(
    npm_command: &[String],
    name: &str,
    version: Option<&str>,
    allow_scripts: bool,
    store_path: &Path,
) -> anyhow::Result<NpmFetchOutcome> {
    anyhow::ensure!(
        !npm_command.is_empty(),
        "npm command is empty; set `[packages] npm_command` in config.toml or leave it unset"
    );
    let staging = std::env::temp_dir().join(format!("roder-npm-{}", unique_suffix()));
    fs::create_dir_all(&staging)
        .with_context(|| format!("create npm staging dir {}", staging.display()))?;

    let result = npm_fetch_with_staging(
        npm_command,
        name,
        version,
        allow_scripts,
        store_path,
        &staging,
    );
    let _ = fs::remove_dir_all(&staging);
    result
}

fn npm_fetch_with_staging(
    npm_command: &[String],
    name: &str,
    version: Option<&str>,
    allow_scripts: bool,
    store_path: &Path,
    staging: &Path,
) -> anyhow::Result<NpmFetchOutcome> {
    let spec = format!("{name}@{}", version.unwrap_or("latest"));
    let mut args: Vec<String> = npm_command[1..].to_vec();
    args.push("install".to_string());
    args.push(spec.clone());
    if !allow_scripts {
        args.push("--ignore-scripts".to_string());
    }
    args.extend(
        ["--omit=dev", "--no-audit", "--no-fund", "--prefix"]
            .into_iter()
            .map(str::to_string),
    );
    args.push(staging.display().to_string());

    run_npm(&npm_command[0], &args, None)
        .with_context(|| format!("npm install of {spec} failed"))?;

    let package_root = staged_package_root(staging, name);
    anyhow::ensure!(
        package_root.is_dir(),
        "npm install of {spec} did not produce {}; the package may not exist",
        package_root.display()
    );
    let resolved_version = read_package_json_version(&package_root.join("package.json"));

    // Build the final tree next to the store path, then swap it in atomically.
    let staged_final = staging_sibling(store_path, "npm-stage");
    if let Some(parent) = staged_final.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create store directory {}", parent.display()))?;
    }
    copy_dir_recursive(&package_root, &staged_final)?;
    merge_hoisted_dependencies(
        &staging.join("node_modules"),
        name,
        &staged_final.join("node_modules"),
    )?;
    swap_dir_into_place(&staged_final, store_path)?;

    Ok(NpmFetchOutcome {
        resolved_version,
        warnings: Vec::new(),
    })
}

/// `node_modules/<name>`; scoped names (`@scope/pkg`) nest one level deeper.
fn staged_package_root(staging: &Path, name: &str) -> PathBuf {
    let mut root = staging.join("node_modules");
    for segment in name.split('/') {
        root = root.join(segment);
    }
    root
}

/// Copies hoisted sibling dependencies from the staging `node_modules` into
/// the package's own `node_modules`, skipping the package itself and
/// anything the package already ships nested.
fn merge_hoisted_dependencies(
    staged_modules: &Path,
    name: &str,
    dest_modules: &Path,
) -> anyhow::Result<()> {
    let Ok(entries) = fs::read_dir(staged_modules) else {
        return Ok(());
    };
    let (own_scope, own_tail) = match name.split_once('/') {
        Some((scope, tail)) => (Some(scope), tail),
        None => (None, name),
    };
    for entry in entries.flatten() {
        let entry_name = entry.file_name();
        let entry_name_str = entry_name.to_string_lossy().to_string();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if entry_name_str.starts_with('@') && is_dir {
            // Scope directory: copy inner packages individually so the
            // package's own scoped tree is skipped but scope siblings keep
            // resolving.
            for inner in fs::read_dir(entry.path()).into_iter().flatten().flatten() {
                let inner_name = inner.file_name().to_string_lossy().to_string();
                if own_scope == Some(entry_name_str.as_str()) && inner_name == own_tail {
                    continue;
                }
                let dest = dest_modules.join(&entry_name_str).join(&inner_name);
                if dest.exists() {
                    continue;
                }
                copy_dir_recursive(&inner.path(), &dest)?;
            }
            continue;
        }
        if own_scope.is_none() && entry_name_str == own_tail {
            continue;
        }
        let dest = dest_modules.join(&entry_name_str);
        if dest.exists() {
            continue;
        }
        if is_dir {
            copy_dir_recursive(&entry.path(), &dest)?;
        } else {
            fs::create_dir_all(dest_modules)
                .with_context(|| format!("create {}", dest_modules.display()))?;
            fs::copy(entry.path(), &dest)
                .with_context(|| format!("copy {}", entry.path().display()))?;
        }
    }
    Ok(())
}

pub(crate) fn read_package_json_version(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value
        .get("version")
        .and_then(|version| version.as_str())
        .map(str::to_string)
}

fn run_npm(program: &str, args: &[String], cwd: Option<&Path>) -> anyhow::Result<()> {
    let mut command = Command::new(program);
    command.args(args);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let output = command.output().map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            anyhow::anyhow!(
                "npm command {program:?} not found; install npm or set `[packages] npm_command` \
                 in config.toml"
            )
        } else {
            anyhow::Error::from(err).context(format!("run npm command {program:?}"))
        }
    })?;
    if !output.status.success() {
        anyhow::bail!(
            "{program} {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// Installs a git package's node dependencies in place (`npm install` inside
/// the clone). A missing npm binary degrades to a warning because git
/// packages may not need node dependencies at all.
pub(crate) fn install_node_dependencies(
    root: &Path,
    npm_command: &[String],
    allow_scripts: bool,
    warnings: &mut Vec<String>,
) -> anyhow::Result<()> {
    if !root.join("package.json").is_file() {
        return Ok(());
    }
    if npm_command.is_empty() {
        warnings.push("skipped node dependency install: npm command is empty".to_string());
        return Ok(());
    }
    let mut args: Vec<String> = npm_command[1..].to_vec();
    args.push("install".to_string());
    if !allow_scripts {
        args.push("--ignore-scripts".to_string());
    }
    args.extend(
        ["--omit=dev", "--no-audit", "--no-fund"]
            .into_iter()
            .map(str::to_string),
    );
    let mut command = Command::new(&npm_command[0]);
    command.args(&args).current_dir(root);
    let output = match command.output() {
        Ok(output) => output,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            warnings.push(format!(
                "skipped node dependency install: npm command {:?} not found (set `[packages] \
                 npm_command` in config.toml if this package needs node dependencies)",
                npm_command[0]
            ));
            return Ok(());
        }
        Err(err) => {
            return Err(err).with_context(|| {
                format!("run npm command {:?} in {}", npm_command[0], root.display())
            });
        }
    };
    if !output.status.success() {
        anyhow::bail!(
            "node dependency install in {} failed: {}",
            root.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}
