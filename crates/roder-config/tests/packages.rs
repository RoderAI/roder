//! End-to-end tests for the package fetch/store/settings layer. Everything
//! runs against temp directories and local fixtures: no real npm, no network
//! git. The single live test is `#[ignore]`d behind `RODER_PACKAGES_LIVE=1`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use roder_api::packages::{
    PackageResourceFilters, PackageResourceKind, PackageScope, PackageSource,
};
use roder_config::packages::{
    InstallOptions, ManifestSourceKind, PackagePaths, SyncStatus, UpdateStatus, approve_extensions,
    enabled_package_resources, install_package, list_packages, load_package_manifest,
    load_settings, package_command_dirs, package_process_extensions, package_skill_roots,
    package_theme_dirs, remove_package, set_filters, set_package_enabled, set_resource_enabled,
    sync_project_packages, update_packages,
};

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn tempdir(name: &str) -> PathBuf {
    let unique = TEMP_COUNTER.fetch_add(1, Ordering::SeqCst);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "roder-pkg-tests-{name}-{}-{nanos}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn test_paths(name: &str) -> (PackagePaths, PathBuf) {
    let base = tempdir(name);
    let user_dir = base.join("user-roder");
    let workspace = base.join("workspace");
    fs::create_dir_all(&user_dir).unwrap();
    fs::create_dir_all(&workspace).unwrap();
    (
        PackagePaths {
            user_dir,
            workspace: Some(workspace),
            ephemeral_roots: Vec::new(),
            ephemeral_extensions_approved: false,
        },
        base,
    )
}

fn fixture_root() -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/packages/demo-pkg");
    fs::canonicalize(&path).unwrap()
}

fn write(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap().flatten() {
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            fs::copy(entry.path(), &dest).unwrap();
        }
    }
}

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .env("GIT_TERMINAL_PROMPT", "0")
        .current_dir(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Creates a local origin repo seeded with the demo-pkg fixture and returns
/// `(repo path, file:// url)`.
fn init_origin_repo(base: &Path) -> (PathBuf, String) {
    let origin = base.join("origin");
    copy_dir(&fixture_root(), &origin);
    git(&origin, &["init", "--initial-branch=main"]);
    git(&origin, &["config", "user.email", "roder@example.test"]);
    git(&origin, &["config", "user.name", "Roder Test"]);
    git(&origin, &["add", "."]);
    git(&origin, &["commit", "-m", "initial package"]);
    let url = format!("file://{}", origin.display());
    (origin, url)
}

fn resource_ids(installed: &roder_config::packages::InstalledPackage) -> Vec<String> {
    installed
        .resources
        .iter()
        .map(|resource| resource.id())
        .collect()
}

#[test]
fn local_path_install_records_and_enumerates_all_resource_kinds() {
    let (paths, _base) = test_paths("local-install");
    let fixture = fixture_root();

    let installed = install_package(
        &paths,
        PackageScope::User,
        &fixture.display().to_string(),
        InstallOptions::default(),
    )
    .unwrap();

    let record = &installed.record;
    assert_eq!(record.package_id, "demo-pkg");
    assert_eq!(record.scope, PackageScope::User);
    assert_eq!(record.install_path, None, "local paths load in place");
    assert!(record.enabled);
    assert!(!record.allow_scripts);
    assert!(!record.extensions_approved);
    assert!(record.content_hash.is_some());
    assert_eq!(
        record.source,
        PackageSource::LocalPath {
            path: fixture.display().to_string()
        }
    );

    let ids = resource_ids(&installed);
    assert!(
        ids.contains(&"demo-pkg:skill/changelog".to_string()),
        "{ids:?}"
    );
    assert!(
        ids.contains(&"demo-pkg:command/greet".to_string()),
        "{ids:?}"
    );
    assert!(
        ids.contains(&"demo-pkg:theme/demo-dark".to_string()),
        "{ids:?}"
    );
    assert!(
        ids.contains(&"demo-pkg:extension/hello-tools".to_string()),
        "{ids:?}"
    );
    assert_eq!(installed.resources.len(), 4);
    let extension = installed
        .resources
        .iter()
        .find(|resource| resource.kind == PackageResourceKind::Extension)
        .unwrap();
    assert!(extension.requires_approval);
    assert_eq!(extension.path, "extensions/hello/roder-extension.toml");

    // Settings file landed in the user scope dir.
    let settings = load_settings(&paths.settings_path(PackageScope::User).unwrap()).unwrap();
    assert_eq!(settings.packages.len(), 1);

    // Relative specs resolve against `resolve_base`.
    let installed_relative = install_package(
        &paths,
        PackageScope::User,
        "./demo-pkg",
        InstallOptions {
            resolve_base: Some(fixture.parent().unwrap().to_path_buf()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        installed_relative.record.identity,
        installed.record.identity
    );
    let settings = load_settings(&paths.settings_path(PackageScope::User).unwrap()).unwrap();
    assert_eq!(settings.packages.len(), 1, "re-install must not duplicate");
}

#[test]
fn empty_root_installs_with_zero_resources_and_diagnostic() {
    let (paths, base) = test_paths("empty-root");
    let empty = base.join("empty-pkg");
    fs::create_dir_all(&empty).unwrap();

    let installed = install_package(
        &paths,
        PackageScope::User,
        &empty.display().to_string(),
        InstallOptions::default(),
    )
    .unwrap();

    assert_eq!(installed.record.package_id, "empty-pkg");
    assert!(installed.resources.is_empty());
    assert!(
        installed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("declares no resources")),
        "{:?}",
        installed.diagnostics
    );
}

#[test]
fn conventional_dirs_only_package_derives_id_and_enumerates() {
    let (paths, base) = test_paths("conventional");
    let root = base.join("conv-pkg");
    write(
        &root.join("skills/release-notes/SKILL.md"),
        "---\nname: release-notes\ndescription: Notes.\n---\nBody.\n",
    );
    write(
        &root.join("commands/ship.md"),
        "---\ndescription: Ship\n---\nShip it.\n",
    );
    write(
        &root.join("themes/conv-light.css"),
        ":root { --bg: #fff; }\n",
    );
    write(
        &root.join("extensions/probe/roder-extension.toml"),
        "id = \"probe\"\nname = \"Probe\"\nversion = \"0.1.0\"\napi_version = \"^0.1\"\nprovides = [{ type = \"event_sink\", id = \"probe-sink\" }]\n\n[launch]\ncommand = \"python3\"\nargs = [\"probe.py\"]\n",
    );

    let source = PackageSource::LocalPath {
        path: root.display().to_string(),
    };
    let (manifest, _diagnostics) = load_package_manifest(&root, &source).unwrap();
    assert_eq!(manifest.source, ManifestSourceKind::Conventional);

    let installed = install_package(
        &paths,
        PackageScope::User,
        &root.display().to_string(),
        InstallOptions::default(),
    )
    .unwrap();
    assert_eq!(installed.record.package_id, "conv-pkg");
    let ids = resource_ids(&installed);
    assert!(
        ids.contains(&"conv-pkg:skill/release-notes".to_string()),
        "{ids:?}"
    );
    assert!(
        ids.contains(&"conv-pkg:command/ship".to_string()),
        "{ids:?}"
    );
    assert!(
        ids.contains(&"conv-pkg:theme/conv-light".to_string()),
        "{ids:?}"
    );
    assert!(
        ids.contains(&"conv-pkg:extension/probe".to_string()),
        "{ids:?}"
    );
}

#[test]
fn package_json_roder_key_declares_resources_and_falls_back_to_derived_id() {
    let (paths, base) = test_paths("package-json");
    let root = base.join("json-demo");
    write(
        &root.join("package.json"),
        r#"{
  "name": "@roder-test/json-demo",
  "version": "2.0.0",
  "keywords": ["roder-package"],
  "roder": {
    "id": "json-pkg",
    "skills": ["skills"],
    "commands": ["commands"]
  }
}
"#,
    );
    write(
        &root.join("skills/triage/SKILL.md"),
        "---\nname: triage\ndescription: Triage.\n---\nBody.\n",
    );
    write(
        &root.join("commands/triage.md"),
        "---\ndescription: Triage\n---\nGo.\n",
    );
    // Present on disk but NOT declared in the roder key: must not enumerate.
    write(&root.join("themes/sneaky.css"), ":root {}\n");

    let source = PackageSource::LocalPath {
        path: root.display().to_string(),
    };
    let (manifest, _diagnostics) = load_package_manifest(&root, &source).unwrap();
    assert_eq!(manifest.source, ManifestSourceKind::PackageJson);
    assert_eq!(manifest.spec.id, "json-pkg");
    assert_eq!(manifest.spec.version.as_deref(), Some("2.0.0"));

    let installed = install_package(
        &paths,
        PackageScope::User,
        &root.display().to_string(),
        InstallOptions::default(),
    )
    .unwrap();
    let ids = resource_ids(&installed);
    assert!(
        ids.contains(&"json-pkg:skill/triage".to_string()),
        "{ids:?}"
    );
    assert!(
        ids.contains(&"json-pkg:command/triage".to_string()),
        "{ids:?}"
    );
    assert!(
        !ids.iter().any(|id| id.contains(":theme/")),
        "undeclared themes must not enumerate: {ids:?}"
    );

    // No explicit id: falls back to derive_package_id (directory name).
    let fallback_root = base.join("fallback-demo");
    write(
        &fallback_root.join("package.json"),
        r#"{ "name": "fallback", "roder": { "skills": ["skills"] } }"#,
    );
    write(
        &fallback_root.join("skills/helper/SKILL.md"),
        "---\nname: helper\ndescription: Helper.\n---\nBody.\n",
    );
    let installed = install_package(
        &paths,
        PackageScope::User,
        &fallback_root.display().to_string(),
        InstallOptions::default(),
    )
    .unwrap();
    assert_eq!(installed.record.package_id, "fallback-demo");
}

#[test]
fn duplicate_package_id_in_scope_is_rejected_with_actionable_error() {
    let (paths, base) = test_paths("duplicate-id");
    for name in ["first", "second"] {
        let root = base.join(name);
        write(
            &root.join("roder.toml"),
            "[package]\nid = \"same-id\"\n\n[resources]\nskills = []\n",
        );
    }
    install_package(
        &paths,
        PackageScope::User,
        &base.join("first").display().to_string(),
        InstallOptions::default(),
    )
    .unwrap();
    let err = install_package(
        &paths,
        PackageScope::User,
        &base.join("second").display().to_string(),
        InstallOptions::default(),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("same-id"), "{err}");
    assert!(err.contains("already used"), "{err}");
}

#[test]
fn git_install_from_local_repo_and_pinned_tag() {
    let (paths, base) = test_paths("git-install");
    let (origin, url) = init_origin_repo(&base);
    let head = git(&origin, &["rev-parse", "HEAD"]);

    let installed = install_package(
        &paths,
        PackageScope::User,
        &format!("git:{url}"),
        InstallOptions::default(),
    )
    .unwrap();
    assert_eq!(installed.record.package_id, "demo-pkg");
    assert_eq!(installed.record.resolved.as_deref(), Some(head.as_str()));
    let store = PathBuf::from(installed.record.install_path.clone().unwrap());
    assert!(store.starts_with(paths.store_root(PackageScope::User).unwrap()));
    assert!(store.join(".git").exists(), "clone keeps .git for updates");
    assert!(store.join("skills/changelog/SKILL.md").exists());
    assert_eq!(resource_ids(&installed).len(), 4);

    // Pin to a tag: same identity, record updates in place.
    git(&origin, &["tag", "v1"]);
    write(&origin.join("extra.txt"), "after v1\n");
    git(&origin, &["add", "."]);
    git(&origin, &["commit", "-m", "after v1"]);

    let pinned = install_package(
        &paths,
        PackageScope::User,
        &format!("git:{url}@v1"),
        InstallOptions::default(),
    )
    .unwrap();
    assert_eq!(pinned.record.identity, installed.record.identity);
    assert_eq!(pinned.record.resolved.as_deref(), Some(head.as_str()));
    assert!(pinned.record.source.pinned());
    assert!(
        !store.join("extra.txt").exists(),
        "pinned clone stays at v1"
    );

    let settings = load_settings(&paths.settings_path(PackageScope::User).unwrap()).unwrap();
    assert_eq!(
        settings.packages.len(),
        1,
        "pin reinstall must not duplicate"
    );
}

#[test]
fn git_update_reconciles_unpinned_and_pinned_clones() {
    let (paths, base) = test_paths("git-update");
    let (origin, url) = init_origin_repo(&base);
    let first = git(&origin, &["rev-parse", "HEAD"]);

    let installed = install_package(
        &paths,
        PackageScope::User,
        &format!("git:{url}"),
        InstallOptions::default(),
    )
    .unwrap();
    assert_eq!(installed.record.resolved.as_deref(), Some(first.as_str()));
    let store = PathBuf::from(installed.record.install_path.clone().unwrap());

    // Advance origin; bulk update reconciles the unpinned clone to the tip.
    write(&origin.join("extra.txt"), "second commit\n");
    git(&origin, &["add", "."]);
    git(&origin, &["commit", "-m", "second"]);
    let second = git(&origin, &["rev-parse", "HEAD"]);

    let outcomes = update_packages(&paths, None, None).unwrap();
    assert_eq!(outcomes.len(), 1);
    assert_eq!(
        outcomes[0].status,
        UpdateStatus::Updated {
            resolved: Some(second.clone())
        }
    );
    assert!(store.join("extra.txt").exists());
    let settings = load_settings(&paths.settings_path(PackageScope::User).unwrap()).unwrap();
    assert_eq!(
        settings.packages[0].resolved.as_deref(),
        Some(second.as_str())
    );

    // Pin to v1 (the first commit), advance origin again, and bulk update:
    // pinned git clones still reconcile to their ref.
    git(&origin, &["tag", "v1", &first]);
    install_package(
        &paths,
        PackageScope::User,
        &format!("git:{url}@v1"),
        InstallOptions::default(),
    )
    .unwrap();
    write(&origin.join("extra2.txt"), "third commit\n");
    git(&origin, &["add", "."]);
    git(&origin, &["commit", "-m", "third"]);

    let outcomes = update_packages(&paths, None, None).unwrap();
    assert_eq!(outcomes.len(), 1);
    assert_eq!(
        outcomes[0].status,
        UpdateStatus::Updated {
            resolved: Some(first.clone())
        }
    );
    assert!(
        !store.join("extra2.txt").exists(),
        "pinned clone stays at v1"
    );

    // Single-target update works by package id too.
    let outcomes = update_packages(&paths, None, Some("demo-pkg")).unwrap();
    assert_eq!(outcomes.len(), 1);
    assert_eq!(
        outcomes[0].status,
        UpdateStatus::Updated {
            resolved: Some(first.clone())
        }
    );
}

#[cfg(unix)]
fn write_stub_npm(dir: &Path, log: &Path, name: &str, version: &str, package_id: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script_path = dir.join("npm-stub");
    let script = format!(
        r#"#!/bin/sh
set -e
printf '%s\n' "$@" > "{log}"
prefix=""
prev=""
for arg in "$@"; do
  if [ "$prev" = "--prefix" ]; then prefix="$arg"; fi
  prev="$arg"
done
if [ -z "$prefix" ]; then echo "stub npm: missing --prefix" >&2; exit 1; fi
pkg="$prefix/node_modules/{name}"
mkdir -p "$pkg/skills/stub-skill"
cat > "$pkg/package.json" <<'EOF'
{{ "name": "{name}", "version": "{version}" }}
EOF
cat > "$pkg/roder.toml" <<'EOF'
[package]
id = "{package_id}"

[resources]
skills = ["skills"]
EOF
cat > "$pkg/skills/stub-skill/SKILL.md" <<'EOF'
---
name: stub-skill
description: Stub skill from the fake npm registry.
---
Stub body.
EOF
mkdir -p "$prefix/node_modules/dep-a"
printf '{{ "name": "dep-a", "version": "1.0.0" }}' > "$prefix/node_modules/dep-a/package.json"
"#,
        log = log.display(),
        name = name,
        version = version,
        package_id = package_id,
    );
    fs::write(&script_path, script).unwrap();
    fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).unwrap();
    script_path
}

#[cfg(unix)]
#[test]
fn npm_stub_install_disables_scripts_by_default_and_records_resolved_version() {
    let (paths, base) = test_paths("npm-stub");
    let log = base.join("npm-argv.log");
    let stub = write_stub_npm(&base, &log, "@roder-test/demo", "1.2.3", "stub-npm-pkg");
    let options = || InstallOptions {
        npm_command: Some(vec![stub.display().to_string()]),
        ..Default::default()
    };

    let installed = install_package(
        &paths,
        PackageScope::User,
        "npm:@roder-test/demo@1.2.3",
        options(),
    )
    .unwrap();

    let argv = fs::read_to_string(&log).unwrap();
    let args: Vec<&str> = argv.lines().collect();
    assert!(args.contains(&"install"), "{args:?}");
    assert!(args.contains(&"@roder-test/demo@1.2.3"), "{args:?}");
    assert!(
        args.contains(&"--ignore-scripts"),
        "scripts must be off by default: {args:?}"
    );
    assert!(args.contains(&"--omit=dev"), "{args:?}");
    assert!(args.contains(&"--no-audit"), "{args:?}");
    assert!(args.contains(&"--no-fund"), "{args:?}");

    assert_eq!(installed.record.package_id, "stub-npm-pkg");
    assert_eq!(installed.record.resolved.as_deref(), Some("1.2.3"));
    assert!(installed.record.source.pinned());
    let store = PathBuf::from(installed.record.install_path.clone().unwrap());
    assert!(
        store.ends_with("packages/npm/@roder-test__demo"),
        "{}",
        store.display()
    );
    assert!(store.join("package.json").exists());
    assert!(
        store.join("node_modules/dep-a/package.json").exists(),
        "hoisted dependencies must move next to the package"
    );
    assert_eq!(
        resource_ids(&installed),
        vec!["stub-npm-pkg:skill/stub-skill"]
    );

    // Bulk update skips the pinned npm package without invoking npm.
    fs::remove_file(&log).unwrap();
    let outcomes = update_packages(&paths, Some(PackageScope::User), None).unwrap();
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].status, UpdateStatus::SkippedPinned);
    assert!(!log.exists(), "skipped pinned npm package must not run npm");

    // `--allow-scripts` drops `--ignore-scripts` and is recorded.
    let installed = install_package(
        &paths,
        PackageScope::User,
        "npm:@roder-test/demo@1.2.3",
        InstallOptions {
            allow_scripts: true,
            ..options()
        },
    )
    .unwrap();
    assert!(installed.record.allow_scripts);
    let argv = fs::read_to_string(&log).unwrap();
    let args: Vec<&str> = argv.lines().collect();
    assert!(!args.contains(&"--ignore-scripts"), "{args:?}");
    let settings = load_settings(&paths.settings_path(PackageScope::User).unwrap()).unwrap();
    assert_eq!(settings.packages.len(), 1, "re-install must not duplicate");
}

#[test]
fn reinstall_preserves_user_decisions_and_applies_new_args() {
    let (paths, _base) = test_paths("reinstall");
    let fixture = fixture_root();
    let spec = fixture.display().to_string();

    install_package(&paths, PackageScope::User, &spec, InstallOptions::default()).unwrap();
    approve_extensions(&paths, "demo-pkg", true).unwrap();
    set_resource_enabled(&paths, "demo-pkg:command/greet", false).unwrap();

    let reinstalled = install_package(
        &paths,
        PackageScope::User,
        &spec,
        InstallOptions {
            filters: PackageResourceFilters {
                themes: Some(vec![]),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();

    let record = &reinstalled.record;
    assert!(
        record.extensions_approved,
        "approval preserved across re-install"
    );
    assert_eq!(record.disabled_resources, vec!["demo-pkg:command/greet"]);
    assert_eq!(record.filters.themes, Some(vec![]), "new filters applied");
    let greet = reinstalled
        .resources
        .iter()
        .find(|resource| resource.id() == "demo-pkg:command/greet")
        .unwrap();
    assert!(!greet.enabled);
    assert!(
        !reinstalled
            .resources
            .iter()
            .any(|resource| resource.kind == PackageResourceKind::Theme),
        "filtered-out themes must not enumerate"
    );
    let settings = load_settings(&paths.settings_path(PackageScope::User).unwrap()).unwrap();
    assert_eq!(settings.packages.len(), 1);
}

#[test]
fn project_records_shadow_user_records_in_list_and_snapshot() {
    let (paths, _base) = test_paths("shadowing");
    let spec = fixture_root().display().to_string();

    install_package(&paths, PackageScope::User, &spec, InstallOptions::default()).unwrap();
    install_package(
        &paths,
        PackageScope::Project,
        &spec,
        InstallOptions::default(),
    )
    .unwrap();

    let listed = list_packages(&paths).unwrap();
    assert_eq!(listed.len(), 2);
    let project = listed
        .iter()
        .find(|entry| entry.record.scope == PackageScope::Project)
        .unwrap();
    let user = listed
        .iter()
        .find(|entry| entry.record.scope == PackageScope::User)
        .unwrap();
    assert!(!project.shadowed_by_project);
    assert!(user.shadowed_by_project, "project entry wins on identity");

    let (snapshots, diagnostics) = enabled_package_resources(&paths);
    assert_eq!(snapshots.len(), 1, "{diagnostics:?}");
    assert_eq!(snapshots[0].record.scope, PackageScope::Project);

    // Removing the project record unshadows the user record.
    remove_package(&paths, PackageScope::Project, "demo-pkg").unwrap();
    let (snapshots, _diagnostics) = enabled_package_resources(&paths);
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].record.scope, PackageScope::User);
}

#[test]
fn filters_narrow_enumeration_and_force_include_exact_paths() {
    let (paths, _base) = test_paths("filters");
    let spec = fixture_root().display().to_string();

    let installed = install_package(
        &paths,
        PackageScope::User,
        &spec,
        InstallOptions {
            filters: PackageResourceFilters {
                skills: Some(vec![]),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        !installed
            .resources
            .iter()
            .any(|resource| resource.kind == PackageResourceKind::Skill),
        "`skills = []` loads no skills"
    );
    assert!(
        installed
            .resources
            .iter()
            .any(|resource| resource.kind == PackageResourceKind::Command),
        "other kinds stay unfiltered"
    );

    // `+path` force-includes an exact path even when the list is otherwise
    // empty.
    set_filters(
        &paths,
        "demo-pkg",
        PackageResourceFilters {
            skills: Some(vec!["+skills/changelog".to_string()]),
            ..Default::default()
        },
    )
    .unwrap();
    let (snapshots, _diagnostics) = enabled_package_resources(&paths);
    assert!(
        snapshots[0]
            .resources
            .iter()
            .any(|resource| resource.id() == "demo-pkg:skill/changelog")
    );
}

#[test]
fn resource_and_package_toggles_apply_to_snapshot_views() {
    let (paths, _base) = test_paths("toggles");
    let spec = fixture_root().display().to_string();
    install_package(&paths, PackageScope::User, &spec, InstallOptions::default()).unwrap();

    assert_eq!(package_command_dirs(&paths).len(), 1);
    assert_eq!(package_theme_dirs(&paths).len(), 1);
    let skill_roots = package_skill_roots(&paths);
    assert_eq!(skill_roots.len(), 1);
    let (package_id, skill_dir, canonical_prefix) = &skill_roots[0];
    assert_eq!(package_id, "demo-pkg");
    assert_eq!(skill_dir, &fixture_root().join("skills"));
    assert_eq!(canonical_prefix, "package://demo-pkg/skills");

    let record = set_resource_enabled(&paths, "demo-pkg:command/greet", false).unwrap();
    assert_eq!(record.disabled_resources, vec!["demo-pkg:command/greet"]);
    assert!(
        package_command_dirs(&paths).is_empty(),
        "directory with no enabled commands drops out"
    );
    set_resource_enabled(&paths, "demo-pkg:command/greet", true).unwrap();
    assert_eq!(package_command_dirs(&paths).len(), 1);

    // Disabling the whole package empties every view.
    set_package_enabled(&paths, "demo-pkg", false).unwrap();
    let (snapshots, _diagnostics) = enabled_package_resources(&paths);
    assert!(snapshots.is_empty());
    assert!(package_skill_roots(&paths).is_empty());
    set_package_enabled(&paths, "demo-pkg", true).unwrap();
    assert_eq!(package_skill_roots(&paths).len(), 1);
}

#[test]
fn approval_gates_process_extension_configs() {
    let (paths, _base) = test_paths("approval");
    let spec = fixture_root().display().to_string();
    install_package(&paths, PackageScope::User, &spec, InstallOptions::default()).unwrap();

    assert!(
        package_process_extensions(&paths).is_empty(),
        "unapproved packages must not produce launchable configs"
    );

    approve_extensions(&paths, "demo-pkg", true).unwrap();
    let configs = package_process_extensions(&paths);
    assert_eq!(configs.len(), 1);
    let config = &configs[0];
    assert_eq!(config.id, "hello-tools");
    assert!(config.enabled);
    assert_eq!(config.command, "python3");
    assert_eq!(config.args, vec!["main.py"]);
    let manifest_dir = fixture_root().join("extensions/hello");
    assert_eq!(
        config.cwd.as_deref(),
        Some(manifest_dir.display().to_string().as_str())
    );
    assert_eq!(
        config.manifest,
        manifest_dir
            .join("roder-extension.toml")
            .display()
            .to_string()
    );
    assert_eq!(config.startup_timeout_ms, 10_000);

    // Disabling the extension resource removes it even while approved.
    set_resource_enabled(&paths, "demo-pkg:extension/hello-tools", false).unwrap();
    assert!(package_process_extensions(&paths).is_empty());

    approve_extensions(&paths, "demo-pkg", false).unwrap();
    set_resource_enabled(&paths, "demo-pkg:extension/hello-tools", true).unwrap();
    assert!(package_process_extensions(&paths).is_empty());
}

#[test]
fn remove_deletes_store_dirs_but_never_local_roots() {
    let (paths, base) = test_paths("remove");
    let (_origin, url) = init_origin_repo(&base);

    let installed = install_package(
        &paths,
        PackageScope::User,
        &format!("git:{url}"),
        InstallOptions::default(),
    )
    .unwrap();
    let store = PathBuf::from(installed.record.install_path.clone().unwrap());
    assert!(store.exists());
    let removed = remove_package(&paths, PackageScope::User, "demo-pkg").unwrap();
    assert_eq!(removed.package_id, "demo-pkg");
    assert!(!store.exists(), "store dir is deleted on remove");
    let settings = load_settings(&paths.settings_path(PackageScope::User).unwrap()).unwrap();
    assert!(settings.packages.is_empty());

    // Local-path packages: the record goes away, the directory stays.
    let local = base.join("local-pkg");
    write(
        &local.join("roder.toml"),
        "[package]\nid = \"local-pkg\"\n\n[resources]\ncommands = [\"commands\"]\n",
    );
    write(
        &local.join("commands/hi.md"),
        "---\ndescription: Hi\n---\nHi.\n",
    );
    install_package(
        &paths,
        PackageScope::User,
        &local.display().to_string(),
        InstallOptions::default(),
    )
    .unwrap();
    remove_package(&paths, PackageScope::User, "local-pkg").unwrap();
    assert!(
        local.join("commands/hi.md").exists(),
        "local roots are never deleted"
    );

    let err = remove_package(&paths, PackageScope::User, "local-pkg")
        .unwrap_err()
        .to_string();
    assert!(err.contains("not installed"), "{err}");
}

#[test]
fn sync_materializes_missing_project_stores() {
    let (paths, base) = test_paths("sync");
    let (_origin, url) = init_origin_repo(&base);
    let installed = install_package(
        &paths,
        PackageScope::Project,
        &format!("git:{url}"),
        InstallOptions::default(),
    )
    .unwrap();
    let store = PathBuf::from(installed.record.install_path.clone().unwrap());

    // Simulate a fresh checkout: settings committed, store missing.
    fs::remove_dir_all(&store).unwrap();
    let (snapshots, diagnostics) = enabled_package_resources(&paths);
    assert!(snapshots.is_empty());
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("missing")),
        "{diagnostics:?}"
    );

    let outcomes = sync_project_packages(&paths).unwrap();
    assert_eq!(outcomes.len(), 1);
    assert!(
        matches!(outcomes[0].status, SyncStatus::Materialized { .. }),
        "{:?}",
        outcomes[0].status
    );
    assert!(store.join("skills/changelog/SKILL.md").exists());

    let outcomes = sync_project_packages(&paths).unwrap();
    assert_eq!(outcomes[0].status, SyncStatus::AlreadyPresent);
}

#[test]
fn ephemeral_roots_appear_in_snapshot_without_settings_records() {
    let base = tempdir("ephemeral");
    let user_dir = base.join("user-roder");
    fs::create_dir_all(&user_dir).unwrap();
    let mut paths = PackagePaths {
        user_dir,
        workspace: None,
        ephemeral_roots: vec![fixture_root()],
        ephemeral_extensions_approved: false,
    };

    let (snapshots, diagnostics) = enabled_package_resources(&paths);
    assert_eq!(snapshots.len(), 1, "{diagnostics:?}");
    let snapshot = &snapshots[0];
    assert_eq!(snapshot.record.package_id, "demo-pkg");
    assert_eq!(snapshot.record.scope, PackageScope::Project);
    assert!(!snapshot.record.extensions_approved);
    assert_eq!(snapshot.resources.len(), 4);
    assert!(
        package_process_extensions(&paths).is_empty(),
        "ephemeral extensions stay gated until approved"
    );
    assert_eq!(package_skill_roots(&paths).len(), 1);

    // No settings file was written anywhere.
    assert!(!paths.settings_path(PackageScope::User).unwrap().exists());

    paths.ephemeral_extensions_approved = true;
    let configs = package_process_extensions(&paths);
    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].id, "hello-tools");
}

#[test]
fn build_skills_registry_includes_project_package_skill_roots() {
    let (paths, _base) = test_paths("skills-registry");
    let workspace = paths.workspace.clone().unwrap();
    install_package(
        &paths,
        PackageScope::Project,
        &fixture_root().display().to_string(),
        InstallOptions::default(),
    )
    .unwrap();

    let registry = roder_config::build_skills_registry(&workspace, None);
    let changelog = registry
        .skills()
        .iter()
        .find(|skill| skill.descriptor.name == "changelog")
        .expect("package skill must surface through the registry");
    assert_eq!(
        changelog.descriptor.canonical_path,
        "package://demo-pkg/skills/changelog/SKILL.md"
    );
    assert_eq!(
        changelog.descriptor.source,
        roder_api::skills::SkillSource::Plugin {
            plugin_id: "pkg-demo-pkg".to_string()
        }
    );
}

/// Live end-to-end npm install. Opt in with:
/// `RODER_PACKAGES_LIVE=1 cargo test -p roder-config --test packages -- --ignored`
#[test]
#[ignore]
fn live_npm_install_end_to_end() {
    if std::env::var("RODER_PACKAGES_LIVE").ok().as_deref() != Some("1") {
        eprintln!("skipping live npm test: set RODER_PACKAGES_LIVE=1 to run");
        return;
    }
    if Command::new("npm").arg("--version").output().is_err() {
        eprintln!("skipping live npm test: npm not found on PATH");
        return;
    }
    let (paths, _base) = test_paths("live-npm");
    let installed = install_package(
        &paths,
        PackageScope::User,
        "npm:left-pad",
        InstallOptions::default(),
    )
    .unwrap();
    assert_eq!(installed.record.package_id, "left-pad");
    assert!(installed.record.resolved.is_some());
    let store = PathBuf::from(installed.record.install_path.clone().unwrap());
    assert!(store.join("package.json").exists());
    assert!(installed.record.content_hash.is_some());
}
