//! `roder install` / `roder remove` / `roder update` / `roder packages` CLI
//! (roadmap phase 93), plus the ephemeral `-e <spec>` TUI launch support and
//! the package -> process-extension merge used by the runtime builder.
//!
//! These subcommands operate directly on the `roder_config::packages` ops
//! layer (no app-server round trip): package state is plain files under the
//! user config dir and `<workspace>/.roder`.

mod init;

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::Context;
use roder_api::packages::{
    PackageRecord, PackageResource, PackageResourceKind, PackageScope, PackageSource,
    parse_package_resource_id, parse_package_spec,
};
use roder_api::process_extension::ProcessExtensionConfig;
use roder_config::packages::{
    InstallOptions, PackagePaths, RODER_EPHEMERAL_APPROVE_ENV, RODER_EPHEMERAL_PACKAGES_ENV,
    SyncStatus, UpdateStatus, approve_extensions, enumerate_resources, install_package,
    list_packages, load_package_manifest, package_process_extensions, remove_package,
    set_filters, set_package_enabled, set_resource_enabled, sync_project_packages,
    update_packages,
};

pub(crate) const PACKAGES_HELP: &str = "\
Roder packages: installable bundles of process extensions, skills, slash
commands, and themes fetched from npm, git, or a local path.

usage:
  roder install <spec> [-l|--local] [--allow-scripts]
      Install a package for your user, or into this workspace with -l.
      Specs: npm:<name>[@version], git:<url>[@ref], a protocol URL, or a
      local path. --allow-scripts permits npm lifecycle scripts.
  roder remove <spec-or-id> [-l|--local]
      Remove an installed package (user scope, or project scope with -l).
  roder update [<spec-or-id>] [--packages-only]
      Update installed packages in both scopes. Bulk update skips pinned
      npm versions. --packages-only is accepted for forward compatibility
      and currently behaves identically: binary self-update is not part of
      `roder update` yet.
  roder packages [list]
      List installed packages from both scopes.
  roder packages resources <package-id>
      List a package's resources with their enabled state and resource ids.
  roder packages enable|disable <package-id-or-resource-id>
      Toggle a whole package, or one resource by id
      (<package-id>:<kind>/<name>).
  roder packages approve|revoke <package-id>
      Allow (or forbid) a package's process extensions to launch.
  roder packages filter <package-id> <kind> [--clear|--none|<pattern>...]
      Narrow which resources of one kind load: include globs, !excludes,
      +path/-path exact forces. --clear loads all, --none loads nothing.
  roder packages sync
      Materialize missing project-scope package stores (e.g. after cloning
      a repo with a committed .roder/packages.json).
  roder packages init <dir>
      Scaffold a new package: roder.toml, a skill, a command, a theme, and
      a Python process extension.
  roder packages help
      Show this help.

ephemeral packages (TUI launch):
  roder -e <spec> [--approve-ephemeral-extensions]
      Load packages for this run only without installing them. Repeatable.
      Process extensions from ephemeral packages launch only when
      --approve-ephemeral-extensions is also passed.";

pub(crate) fn run_install_cli(args: &[String]) -> anyhow::Result<()> {
    let mut spec = None;
    let mut local = false;
    let mut allow_scripts = false;
    for arg in args {
        match arg.as_str() {
            "-l" | "--local" => local = true,
            "--allow-scripts" => allow_scripts = true,
            other if other.starts_with("--") => {
                anyhow::bail!("unknown install flag {other}\n{INSTALL_USAGE}")
            }
            other => {
                if spec.replace(other.to_string()).is_some() {
                    anyhow::bail!("{INSTALL_USAGE}");
                }
            }
        }
    }
    let Some(spec) = spec else {
        anyhow::bail!("{INSTALL_USAGE}");
    };
    let paths = standard_paths();
    let scope = scope_for_local(&paths, local)?;
    let installed = install_package(
        &paths,
        scope,
        &spec,
        InstallOptions {
            allow_scripts,
            ..InstallOptions::default()
        },
    )
    .with_context(|| format!("install package {spec}"))?;

    let record = &installed.record;
    println!(
        "installed {} ({}) in {} scope",
        record.package_id, record.identity, record.scope
    );
    if let Some(resolved) = &record.resolved {
        println!("  resolved: {resolved}");
    }
    println!("  resources: {}", resource_summary(&installed.resources));
    for diagnostic in &installed.diagnostics {
        println!("  warning: {diagnostic}");
    }
    let extensions: Vec<&PackageResource> = installed
        .resources
        .iter()
        .filter(|resource| resource.kind == PackageResourceKind::Extension)
        .collect();
    if !extensions.is_empty() {
        if record.extensions_approved {
            println!("  process extensions are approved and will launch");
        } else {
            println!(
                "  process extensions are pending approval; run `roder packages approve {}` to allow them to launch",
                record.package_id
            );
        }
    }
    Ok(())
}

const INSTALL_USAGE: &str = "usage: roder install <spec> [-l|--local] [--allow-scripts]";

pub(crate) fn run_remove_cli(args: &[String]) -> anyhow::Result<()> {
    let mut spec_or_id = None;
    let mut local = false;
    for arg in args {
        match arg.as_str() {
            "-l" | "--local" => local = true,
            other if other.starts_with("--") => {
                anyhow::bail!(
                    "unknown remove flag {other}\nusage: roder remove <spec-or-id> [-l|--local]"
                )
            }
            other => {
                if spec_or_id.replace(other.to_string()).is_some() {
                    anyhow::bail!("usage: roder remove <spec-or-id> [-l|--local]");
                }
            }
        }
    }
    let Some(spec_or_id) = spec_or_id else {
        anyhow::bail!("usage: roder remove <spec-or-id> [-l|--local]");
    };
    let paths = standard_paths();
    let scope = scope_for_local(&paths, local)?;
    let removed = remove_package(&paths, scope, &spec_or_id)?;
    println!(
        "removed {} ({}) from {} scope",
        removed.package_id, removed.identity, removed.scope
    );
    Ok(())
}

pub(crate) fn run_update_cli(args: &[String]) -> anyhow::Result<()> {
    let mut target = None;
    for arg in args {
        match arg.as_str() {
            // Accepted for forward compatibility: `roder update` only
            // updates packages today (binary self-update is out of scope).
            "--packages-only" => {}
            other if other.starts_with("--") => anyhow::bail!(
                "unknown update flag {other}\nusage: roder update [<spec-or-id>] [--packages-only]"
            ),
            other => {
                if target.replace(other.to_string()).is_some() {
                    anyhow::bail!("usage: roder update [<spec-or-id>] [--packages-only]");
                }
            }
        }
    }
    let paths = standard_paths();
    let outcomes = update_packages(&paths, None, target.as_deref())?;
    if outcomes.is_empty() {
        println!("no packages installed");
        return Ok(());
    }
    for outcome in outcomes {
        match outcome.status {
            UpdateStatus::Updated { resolved } => println!(
                "updated\t{}\t{}\t{}",
                outcome.package_id,
                outcome.scope,
                resolved.unwrap_or_else(|| "-".to_string())
            ),
            UpdateStatus::SkippedPinned => println!(
                "skipped\t{}\t{}\tpinned (update it explicitly with `roder update {}`)",
                outcome.package_id, outcome.scope, outcome.package_id
            ),
            UpdateStatus::Failed { message } => println!(
                "failed\t{}\t{}\t{}",
                outcome.package_id, outcome.scope, message
            ),
        }
    }
    Ok(())
}

pub(crate) fn run_packages_cli(args: &[String]) -> anyhow::Result<()> {
    match args.first().map(String::as_str) {
        None | Some("list") => print_packages_list(),
        Some("resources") => {
            let Some(id) = args.get(1) else {
                anyhow::bail!("usage: roder packages resources <package-id>");
            };
            print_package_resources(id)
        }
        Some("enable") => set_enabled_cli(args.get(1), true),
        Some("disable") => set_enabled_cli(args.get(1), false),
        Some("approve") => approve_cli(args.get(1), true),
        Some("revoke") => approve_cli(args.get(1), false),
        Some("filter") => filter_cli(&args[1..]),
        Some("sync") => sync_cli(),
        Some("init") => {
            let Some(dir) = args.get(1) else {
                anyhow::bail!("usage: roder packages init <dir>");
            };
            init::run_packages_init(dir)
        }
        Some("help") | Some("--help") | Some("-h") => {
            println!("{PACKAGES_HELP}");
            Ok(())
        }
        _ => anyhow::bail!(
            "usage: roder packages \
             <list|resources|enable|disable|approve|revoke|filter|sync|init|help>"
        ),
    }
}

/// `roder packages filter <package-id> <kind> [--clear|--none|<pattern>...]`.
fn filter_cli(args: &[String]) -> anyhow::Result<()> {
    let (Some(id), Some(kind_arg)) = (args.first(), args.get(1)) else {
        anyhow::bail!(
            "usage: roder packages filter <package-id> \
             <extensions|skills|commands|themes> [--clear|--none|<pattern>...]"
        );
    };
    let kind: PackageResourceKind = kind_arg.parse()?;
    let rest = &args[2..];
    let patterns = match rest.first().map(String::as_str) {
        None | Some("--clear") => None,
        Some("--none") => Some(Vec::new()),
        _ => Some(rest.to_vec()),
    };
    let paths = standard_paths();
    let record = find_record(&paths, id)?;
    let mut filters = record.filters.clone();
    filters.set_for_kind(kind, patterns.clone());
    let record = set_filters(&paths, id, filters)?;
    match patterns {
        None => println!("cleared {kind} filter for {} (loads all)", record.package_id),
        Some(patterns) if patterns.is_empty() => println!(
            "set {kind} filter for {} to load nothing (use +<path> entries to force-include)",
            record.package_id
        ),
        Some(patterns) => println!(
            "set {kind} filter for {}: {}",
            record.package_id,
            patterns.join(" ")
        ),
    }
    print_package_resources(&record.package_id)
}

fn print_packages_list() -> anyhow::Result<()> {
    let paths = standard_paths();
    let listed = list_packages(&paths)?;
    if listed.is_empty() {
        println!("no packages installed; run `roder install <spec>` (see `roder packages help`)");
        return Ok(());
    }
    println!("ID\tSOURCE\tSCOPE\tENABLED\tPINNED\tEXTENSIONS\tPATH");
    let mut diagnostics = Vec::new();
    for entry in &listed {
        let record = &entry.record;
        let (resources, resource_diagnostics) = record_resources(record);
        diagnostics.extend(
            resource_diagnostics
                .into_iter()
                .map(|diagnostic| format!("{}: {diagnostic}", record.package_id)),
        );
        let has_extensions = resources
            .iter()
            .any(|resource| resource.kind == PackageResourceKind::Extension);
        let extensions = if !has_extensions {
            "-"
        } else if record.extensions_approved {
            "approved"
        } else {
            "pending"
        };
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}{}",
            record.package_id,
            record.source.spec(),
            record.scope,
            if record.enabled {
                "enabled"
            } else {
                "disabled"
            },
            if record.source.pinned() {
                "pinned"
            } else {
                "-"
            },
            extensions,
            package_root_display(record),
            if entry.shadowed_by_project {
                "\t[shadowed by project]"
            } else {
                ""
            },
        );
    }
    for diagnostic in diagnostics {
        println!("warning: {diagnostic}");
    }
    Ok(())
}

fn print_package_resources(id: &str) -> anyhow::Result<()> {
    let paths = standard_paths();
    let record = find_record(&paths, id)?;
    let (resources, diagnostics) = record_resources(&record);
    if resources.is_empty() {
        println!("{} declares no resources", record.package_id);
    }
    for resource in resources {
        println!(
            "{}\t{}\t{}\t{}",
            resource.id(),
            if resource.enabled {
                "enabled"
            } else {
                "disabled"
            },
            resource.kind,
            resource.path
        );
    }
    for diagnostic in diagnostics {
        println!("warning: {diagnostic}");
    }
    Ok(())
}

fn set_enabled_cli(id: Option<&String>, enabled: bool) -> anyhow::Result<()> {
    let action = if enabled { "enable" } else { "disable" };
    let Some(id) = id else {
        anyhow::bail!("usage: roder packages {action} <package-id-or-resource-id>");
    };
    let paths = standard_paths();
    let record = if parse_package_resource_id(id).is_ok() {
        set_resource_enabled(&paths, id, enabled)?
    } else {
        set_package_enabled(&paths, id, enabled)?
    };
    println!("{action}d {id} ({} scope)", record.scope);
    Ok(())
}

fn approve_cli(id: Option<&String>, approved: bool) -> anyhow::Result<()> {
    let action = if approved { "approve" } else { "revoke" };
    let Some(id) = id else {
        anyhow::bail!("usage: roder packages {action} <package-id>");
    };
    let paths = standard_paths();
    let record = approve_extensions(&paths, id, approved)?;
    if !approved {
        println!(
            "revoked process extension approval for {}; its extensions will no longer launch",
            record.package_id
        );
        return Ok(());
    }
    println!("approved process extensions for {}", record.package_id);
    let launching: Vec<ProcessExtensionConfig> = package_process_extensions(&paths)
        .into_iter()
        .filter(|extension| extension_belongs_to_record(extension, &record))
        .collect();
    if launching.is_empty() {
        println!("  no enabled process extensions found in this package");
    }
    for extension in launching {
        println!(
            "  will launch: {} ({} {})",
            extension.id,
            extension.command,
            extension.args.join(" ")
        );
    }
    Ok(())
}

/// Matches a launchable extension config back to a record by manifest path:
/// package extension manifests live under the package root.
fn extension_belongs_to_record(extension: &ProcessExtensionConfig, record: &PackageRecord) -> bool {
    let Some(root) = package_root(record) else {
        return false;
    };
    PathBuf::from(&extension.manifest).starts_with(&root)
}

fn sync_cli() -> anyhow::Result<()> {
    let paths = standard_paths();
    let outcomes = sync_project_packages(&paths)?;
    if outcomes.is_empty() {
        println!("no project packages to sync");
        return Ok(());
    }
    for outcome in outcomes {
        match outcome.status {
            SyncStatus::Materialized { resolved } => println!(
                "materialized\t{}\t{}",
                outcome.package_id,
                resolved.unwrap_or_else(|| "-".to_string())
            ),
            SyncStatus::AlreadyPresent => println!("present\t{}", outcome.package_id),
            SyncStatus::Failed { message } => {
                println!("failed\t{}\t{}", outcome.package_id, message)
            }
        }
    }
    Ok(())
}

/// `-e <spec>` / `--extension <spec>` (repeatable) on the default TUI launch
/// path: resolve local paths in place, install other sources into a fresh
/// temp store, and export the ephemeral env vars before the registry/config
/// is built. Returns the args with the ephemeral flags stripped.
pub(crate) fn apply_ephemeral_package_args(args: &[String]) -> anyhow::Result<Vec<String>> {
    let mut stripped = Vec::with_capacity(args.len());
    let mut specs = Vec::new();
    let mut approve = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-e" | "--extension" => {
                let Some(spec) = args.get(i + 1) else {
                    anyhow::bail!("{} requires a package spec", args[i]);
                };
                specs.push(spec.clone());
                i += 1;
            }
            arg if arg.starts_with("--extension=") => {
                specs.push(arg["--extension=".len()..].to_string());
            }
            "--approve-ephemeral-extensions" => approve = true,
            _ => stripped.push(args[i].clone()),
        }
        i += 1;
    }
    if specs.is_empty() {
        if approve {
            eprintln!(
                "warning: --approve-ephemeral-extensions has no effect without -e/--extension"
            );
        }
        return Ok(stripped);
    }
    let mut roots = Vec::new();
    for spec in &specs {
        let root = resolve_ephemeral_root(spec)?;
        println!("ephemeral package: {spec} -> {}", root.display());
        roots.push(root);
    }
    let joined = std::env::join_paths(&roots).context("join ephemeral package roots")?;
    // SAFETY: applied at CLI startup before the registry/config is built or
    // read, mirroring the --config-dir override in main.
    unsafe {
        std::env::set_var(RODER_EPHEMERAL_PACKAGES_ENV, &joined);
        if approve {
            std::env::set_var(RODER_EPHEMERAL_APPROVE_ENV, "1");
        }
    }
    Ok(stripped)
}

/// Local paths load in place; npm/git specs install into a throwaway store
/// for this run and contribute their materialized root.
fn resolve_ephemeral_root(spec: &str) -> anyhow::Result<PathBuf> {
    let source = parse_package_spec(spec)
        .with_context(|| format!("parse ephemeral package spec {spec:?}"))?;
    if let PackageSource::LocalPath { path } = &source {
        let root = resolve_local_path(path)?;
        anyhow::ensure!(
            root.is_dir(),
            "ephemeral package path {} does not exist or is not a directory",
            root.display()
        );
        return Ok(root);
    }
    let temp = std::env::temp_dir().join(format!(
        "roder-ephemeral-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::create_dir_all(&temp)
        .with_context(|| format!("create ephemeral package store {}", temp.display()))?;
    let paths = PackagePaths {
        user_dir: temp,
        workspace: None,
        ephemeral_roots: Vec::new(),
        ephemeral_extensions_approved: false,
    };
    let installed = install_package(&paths, PackageScope::User, spec, InstallOptions::default())
        .with_context(|| format!("fetch ephemeral package {spec}"))?;
    installed
        .record
        .install_path
        .map(PathBuf::from)
        .with_context(|| format!("ephemeral package {spec} produced no materialized root"))
}

fn resolve_local_path(path: &str) -> anyhow::Result<PathBuf> {
    let home = || {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .context("cannot resolve `~`: no home directory")
    };
    let expanded = if path == "~" {
        home()?
    } else if let Some(rest) = path.strip_prefix("~/") {
        home()?.join(rest)
    } else {
        PathBuf::from(path)
    };
    let absolute = if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir()
            .context("resolve relative package path")?
            .join(expanded)
    };
    Ok(std::fs::canonicalize(&absolute).unwrap_or(absolute))
}

/// Config-declared `[[process_extensions]]` entries merged with
/// approval-gated package process extensions. A config entry explicitly
/// configured by the user beats a package-provided one with the same id.
pub(crate) fn merged_process_extensions(
    configured: Vec<ProcessExtensionConfig>,
    workspace: Option<&std::path::Path>,
) -> Vec<ProcessExtensionConfig> {
    let mut merged = configured;
    let paths = PackagePaths::standard(workspace);
    for extension in package_process_extensions(&paths) {
        if let Some(existing) = merged.iter().find(|existing| existing.id == extension.id) {
            eprintln!(
                "warning: package process extension {:?} is shadowed by an existing entry ({})",
                extension.id, existing.manifest
            );
            continue;
        }
        merged.push(extension);
    }
    merged
}

fn standard_paths() -> PackagePaths {
    let workspace = std::env::current_dir().ok();
    PackagePaths::standard(workspace.as_deref())
}

fn scope_for_local(paths: &PackagePaths, local: bool) -> anyhow::Result<PackageScope> {
    if !local {
        return Ok(PackageScope::User);
    }
    anyhow::ensure!(
        paths.workspace.is_some(),
        "-l/--local needs a workspace; run from the workspace directory"
    );
    Ok(PackageScope::Project)
}

/// First record matching a package id or spec; project records come first in
/// `list_packages`, so the shadowing winner wins here too.
fn find_record(paths: &PackagePaths, query: &str) -> anyhow::Result<PackageRecord> {
    let parsed_identity = parse_package_spec(query)
        .ok()
        .map(|source| source.identity());
    list_packages(paths)?
        .into_iter()
        .map(|entry| entry.record)
        .find(|record| {
            record.package_id == query
                || record.source.spec() == query
                || parsed_identity
                    .as_ref()
                    .is_some_and(|identity| &record.identity == identity)
        })
        .with_context(|| format!("package {query:?} is not installed"))
}

/// Enumerates a record's resources from its root (works for disabled
/// packages too; their resources report enabled = false).
fn record_resources(record: &PackageRecord) -> (Vec<PackageResource>, Vec<String>) {
    let Some(root) = package_root(record) else {
        return (
            Vec::new(),
            vec![
                "package has no materialized root; run `roder packages sync` or reinstall"
                    .to_string(),
            ],
        );
    };
    if !root.is_dir() {
        return (
            Vec::new(),
            vec![format!(
                "package root {} is missing; run `roder packages sync` or reinstall",
                root.display()
            )],
        );
    }
    let (manifest, mut diagnostics) = match load_package_manifest(&root, &record.source) {
        Ok(loaded) => loaded,
        Err(err) => return (Vec::new(), vec![format!("{err:#}")]),
    };
    let (resources, resource_diagnostics) = enumerate_resources(&root, &manifest.spec, record);
    diagnostics.extend(resource_diagnostics);
    (resources, diagnostics)
}

fn package_root(record: &PackageRecord) -> Option<PathBuf> {
    if let Some(install_path) = &record.install_path {
        return Some(PathBuf::from(install_path));
    }
    match &record.source {
        PackageSource::LocalPath { path } => Some(PathBuf::from(path)),
        _ => None,
    }
}

fn package_root_display(record: &PackageRecord) -> String {
    package_root(record)
        .map(|root| root.display().to_string())
        .unwrap_or_else(|| "-".to_string())
}

/// Counts by kind plus names, e.g.
/// `1 command (greet); 1 extension (hello-tools); 2 skills (a, b)`.
fn resource_summary(resources: &[PackageResource]) -> String {
    if resources.is_empty() {
        return "none".to_string();
    }
    let mut by_kind: BTreeMap<PackageResourceKind, Vec<&str>> = BTreeMap::new();
    for resource in resources {
        by_kind
            .entry(resource.kind)
            .or_default()
            .push(resource.name.as_str());
    }
    by_kind
        .iter()
        .map(|(kind, names)| {
            format!(
                "{} {}{} ({})",
                names.len(),
                kind,
                if names.len() == 1 { "" } else { "s" },
                names.join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}
