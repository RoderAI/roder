//! "Packages" command palette source (roadmap phase 93).
//!
//! Lists installed Roder packages from `packages/list` and seeds `/packages`
//! slash commands into the composer, mirroring the marketplaces source: a
//! fixed block of management actions first, then one row per installed
//! package with enable/disable and approval follow-up rows.

use roder_api::packages::PackageResourceKind;
use roder_protocol::PackageDescriptor;

use super::{PaletteAction, PaletteItem, StaticPaletteSource};

/// Source specs (npm names, git URLs, paths) can get long; keep the row
/// subtitle scannable in narrow terminals.
const MAX_SPEC_CHARS: usize = 40;
const MAX_DIAGNOSTIC_CHARS: usize = 72;

pub fn packages_source(
    packages: &[PackageDescriptor],
    diagnostics: &[String],
) -> StaticPaletteSource {
    let mut entries = vec![
        (
            PaletteItem {
                id: "packages-install".to_string(),
                title: "Install a package".to_string(),
                subtitle: Some(
                    "npm:<name>[@version], git:<url>[@ref], or a local path".to_string(),
                ),
                keywords: vec![
                    "package".to_string(),
                    "packages".to_string(),
                    "install".to_string(),
                    "npm".to_string(),
                    "git".to_string(),
                ],
                icon: Some('+'),
            },
            PaletteAction::InsertComposerText("/packages install ".to_string()),
        ),
        (
            PaletteItem {
                id: "packages-update-all".to_string(),
                title: "Update all packages".to_string(),
                subtitle: Some(
                    "Update installed packages in both scopes (skips pinned versions)".to_string(),
                ),
                keywords: vec![
                    "package".to_string(),
                    "packages".to_string(),
                    "update".to_string(),
                    "upgrade".to_string(),
                ],
                icon: Some('U'),
            },
            PaletteAction::InsertComposerText("/packages update".to_string()),
        ),
        (
            PaletteItem {
                id: "packages-sync".to_string(),
                title: "Sync project packages".to_string(),
                subtitle: Some("Materialize missing project-scope package stores".to_string()),
                keywords: vec![
                    "package".to_string(),
                    "packages".to_string(),
                    "sync".to_string(),
                    "project".to_string(),
                ],
                icon: Some('S'),
            },
            PaletteAction::InsertComposerText("/packages sync".to_string()),
        ),
        (
            PaletteItem {
                id: "packages-remove".to_string(),
                title: "Remove a package".to_string(),
                subtitle: Some("Remove an installed package by spec or id".to_string()),
                keywords: vec![
                    "package".to_string(),
                    "packages".to_string(),
                    "remove".to_string(),
                    "uninstall".to_string(),
                ],
                icon: Some('-'),
            },
            PaletteAction::InsertComposerText("/packages remove ".to_string()),
        ),
    ];

    for package in packages {
        entries.extend(package_rows(package));
    }

    if let Some(first) = diagnostics.first() {
        entries.push(diagnostics_row(diagnostics.len(), first));
    }

    StaticPaletteSource::new("packages", "Packages", entries)
}

/// Rows for one installed package: the package row itself (Enter seeds
/// `/packages resources <id>`), an enable/disable toggle, and — only while
/// process extensions are pending — an approval row.
fn package_rows(package: &PackageDescriptor) -> Vec<(PaletteItem, PaletteAction)> {
    let record = &package.record;
    let id = &record.package_id;
    let spec = record.source.spec();
    let enabled_label = if record.enabled {
        "enabled"
    } else {
        "disabled"
    };

    let mut rows = vec![(
        PaletteItem {
            id: format!("package-{id}"),
            title: id.clone(),
            subtitle: Some(package_subtitle(package, &spec, enabled_label)),
            keywords: vec![
                "package".to_string(),
                "packages".to_string(),
                id.clone(),
                spec,
                record.scope.to_string(),
                enabled_label.to_string(),
            ],
            icon: Some('◆'),
        },
        PaletteAction::InsertComposerText(format!("/packages resources {id} ")),
    )];

    rows.push((
        PaletteItem {
            id: format!("package-toggle-{id}"),
            title: format!(
                "{} package: {id}",
                if record.enabled { "Disable" } else { "Enable" }
            ),
            subtitle: Some("Toggle the whole package without removing it".to_string()),
            keywords: vec![
                "package".to_string(),
                "packages".to_string(),
                id.clone(),
                (if record.enabled { "disable" } else { "enable" }).to_string(),
            ],
            icon: Some(if record.enabled { '-' } else { '+' }),
        },
        PaletteAction::InsertComposerText(format!(
            "/packages {} {id}",
            if record.enabled { "disable" } else { "enable" }
        )),
    ));

    if extensions_state(package) == "pending" {
        rows.push((
            PaletteItem {
                id: format!("package-approve-{id}"),
                title: format!("Approve extensions: {id}"),
                subtitle: Some("Allow this package's process extensions to launch".to_string()),
                keywords: vec![
                    "package".to_string(),
                    "packages".to_string(),
                    id.clone(),
                    "approve".to_string(),
                    "extensions".to_string(),
                ],
                icon: Some('A'),
            },
            PaletteAction::InsertComposerText(format!("/packages approve {id}")),
        ));
    }

    rows
}

fn package_subtitle(package: &PackageDescriptor, spec: &str, enabled_label: &str) -> String {
    let mut parts = vec![
        truncate(spec, MAX_SPEC_CHARS),
        package.record.scope.to_string(),
        enabled_label.to_string(),
        format!("extensions {}", extensions_state(package)),
    ];
    if package.shadowed_by_project {
        parts.push("shadowed".to_string());
    }
    parts.join(" · ")
}

/// `none` (no process extensions), `approved`, or `pending`.
fn extensions_state(package: &PackageDescriptor) -> &'static str {
    let has_extensions = package
        .resources
        .iter()
        .any(|resource| resource.resource.kind == PackageResourceKind::Extension);
    if !has_extensions {
        "none"
    } else if package.record.extensions_approved {
        "approved"
    } else {
        "pending"
    }
}

fn diagnostics_row(count: usize, first: &str) -> (PaletteItem, PaletteAction) {
    let noun = if count == 1 {
        "diagnostic"
    } else {
        "diagnostics"
    };
    (
        PaletteItem {
            id: "packages-diagnostics".to_string(),
            title: format!("{count} package {noun}"),
            subtitle: Some(truncate(first, MAX_DIAGNOSTIC_CHARS)),
            keywords: vec![
                "package".to_string(),
                "packages".to_string(),
                "diagnostics".to_string(),
                "warning".to_string(),
            ],
            icon: Some('!'),
        },
        PaletteAction::InsertComposerText("/packages list".to_string()),
    )
}

fn truncate(value: &str, max_chars: usize) -> String {
    let mut out = value.chars().take(max_chars).collect::<String>();
    if out.len() < value.len() {
        out.push_str("...");
    }
    out
}

#[cfg(test)]
mod tests {
    use roder_api::packages::{
        PackageRecord, PackageResource, PackageResourceFilters, PackageResourceKind, PackageScope,
        parse_package_spec,
    };
    use roder_protocol::PackageResourceDescriptor;
    use time::OffsetDateTime;

    use super::*;

    fn record(spec: &str, package_id: &str) -> PackageRecord {
        let source = parse_package_spec(spec).expect("valid spec");
        PackageRecord {
            package_id: package_id.to_string(),
            identity: source.identity(),
            source,
            scope: PackageScope::User,
            install_path: None,
            resolved: None,
            enabled: true,
            allow_scripts: false,
            extensions_approved: false,
            installed_at: OffsetDateTime::UNIX_EPOCH,
            content_hash: None,
            filters: PackageResourceFilters::default(),
            disabled_resources: Vec::new(),
        }
    }

    fn descriptor(
        record: PackageRecord,
        resources: Vec<PackageResourceDescriptor>,
    ) -> PackageDescriptor {
        PackageDescriptor {
            record,
            shadowed_by_project: false,
            resources,
        }
    }

    fn extension_resource(package_id: &str) -> PackageResourceDescriptor {
        PackageResource {
            package_id: package_id.to_string(),
            kind: PackageResourceKind::Extension,
            path: "extensions/main/roder-extension.toml".to_string(),
            name: "main".to_string(),
            enabled: true,
            requires_approval: true,
        }
        .into()
    }

    #[test]
    fn packages_source_exposes_static_management_rows() {
        let source = packages_source(&[], &[]);
        let entries = source.entries();
        let titles = entries
            .iter()
            .map(|entry| entry.item.title.as_str())
            .collect::<Vec<_>>();

        assert!(titles.contains(&"Install a package"));
        assert!(titles.contains(&"Update all packages"));
        assert!(titles.contains(&"Sync project packages"));
        assert!(titles.contains(&"Remove a package"));
        assert_eq!(
            entries[0].action,
            PaletteAction::InsertComposerText("/packages install ".to_string())
        );
        assert!(entries.iter().any(|entry| entry.action
            == PaletteAction::InsertComposerText("/packages update".to_string())));
        assert!(
            entries.iter().any(|entry| entry.action
                == PaletteAction::InsertComposerText("/packages sync".to_string()))
        );
        assert!(entries.iter().any(|entry| entry.action
            == PaletteAction::InsertComposerText("/packages remove ".to_string())));
        // No packages, no diagnostics: nothing beyond the static block.
        assert_eq!(entries.len(), 4);
    }

    #[test]
    fn package_row_seeds_resources_command_and_summarizes_state() {
        let package = descriptor(record("npm:@scope/pkg@1.0.0", "demo-pkg"), Vec::new());
        let source = packages_source(&[package], &[]);
        let entries = source.entries();

        let row = entries
            .iter()
            .find(|entry| entry.item.id == "package-demo-pkg")
            .expect("package row");
        assert_eq!(row.item.title, "demo-pkg");
        assert_eq!(
            row.item.subtitle.as_deref(),
            Some("npm:@scope/pkg@1.0.0 · user · enabled · extensions none")
        );
        assert_eq!(
            row.action,
            PaletteAction::InsertComposerText("/packages resources demo-pkg ".to_string())
        );
    }

    #[test]
    fn enabled_package_gets_disable_toggle_and_disabled_gets_enable() {
        let enabled = descriptor(record("npm:@scope/pkg@1.0.0", "on-pkg"), Vec::new());
        let mut disabled_record = record("npm:other", "off-pkg");
        disabled_record.enabled = false;
        let disabled = descriptor(disabled_record, Vec::new());

        let source = packages_source(&[enabled, disabled], &[]);
        let entries = source.entries();

        let disable = entries
            .iter()
            .find(|entry| entry.item.id == "package-toggle-on-pkg")
            .expect("toggle row for enabled package");
        assert_eq!(disable.item.title, "Disable package: on-pkg");
        assert_eq!(
            disable.action,
            PaletteAction::InsertComposerText("/packages disable on-pkg".to_string())
        );

        let enable = entries
            .iter()
            .find(|entry| entry.item.id == "package-toggle-off-pkg")
            .expect("toggle row for disabled package");
        assert_eq!(enable.item.title, "Enable package: off-pkg");
        assert_eq!(
            enable.action,
            PaletteAction::InsertComposerText("/packages enable off-pkg".to_string())
        );
        assert!(
            enable
                .item
                .subtitle
                .as_deref()
                .is_some_and(|s| s.contains("without removing"))
        );
    }

    #[test]
    fn pending_extensions_add_approve_row_and_pending_label() {
        let package = descriptor(
            record("npm:@scope/pkg@1.0.0", "ext-pkg"),
            vec![extension_resource("ext-pkg")],
        );
        let source = packages_source(&[package], &[]);
        let entries = source.entries();

        let row = entries
            .iter()
            .find(|entry| entry.item.id == "package-ext-pkg")
            .expect("package row");
        assert!(
            row.item
                .subtitle
                .as_deref()
                .is_some_and(|s| s.contains("extensions pending"))
        );

        let approve = entries
            .iter()
            .find(|entry| entry.item.id == "package-approve-ext-pkg")
            .expect("approve row");
        assert_eq!(approve.item.title, "Approve extensions: ext-pkg");
        assert_eq!(
            approve.action,
            PaletteAction::InsertComposerText("/packages approve ext-pkg".to_string())
        );
    }

    #[test]
    fn approved_extensions_omit_approve_row() {
        let mut approved_record = record("npm:@scope/pkg@1.0.0", "ok-pkg");
        approved_record.extensions_approved = true;
        let package = descriptor(approved_record, vec![extension_resource("ok-pkg")]);
        let source = packages_source(&[package], &[]);
        let entries = source.entries();

        let row = entries
            .iter()
            .find(|entry| entry.item.id == "package-ok-pkg")
            .expect("package row");
        assert!(
            row.item
                .subtitle
                .as_deref()
                .is_some_and(|s| s.contains("extensions approved"))
        );
        assert!(
            !entries
                .iter()
                .any(|entry| entry.item.id == "package-approve-ok-pkg"),
            "approved package must not offer an approve row"
        );
    }

    #[test]
    fn long_specs_truncate_in_subtitle_but_stay_searchable() {
        let spec = "git:https://example.com/some/very/long/organization/repository-name";
        let package = descriptor(record(spec, "long-pkg"), Vec::new());
        let source = packages_source(&[package], &[]);
        let entries = source.entries();

        let row = entries
            .iter()
            .find(|entry| entry.item.id == "package-long-pkg")
            .expect("package row");
        let subtitle = row.item.subtitle.as_deref().unwrap();
        assert!(subtitle.contains("..."), "subtitle={subtitle}");
        assert!(!subtitle.contains("repository-name"), "subtitle={subtitle}");
        // The full canonical spec stays in the keywords so the complete
        // value remains searchable even though the subtitle is truncated.
        assert!(
            row.item
                .keywords
                .iter()
                .any(|keyword| keyword.contains("repository-name"))
        );
    }

    #[test]
    fn shadowed_package_is_labeled() {
        let mut package = descriptor(record("npm:@scope/pkg", "dup-pkg"), Vec::new());
        package.shadowed_by_project = true;
        let source = packages_source(&[package], &[]);
        let row_subtitle = source
            .entries()
            .iter()
            .find(|entry| entry.item.id == "package-dup-pkg")
            .and_then(|entry| entry.item.subtitle.clone())
            .expect("package row subtitle");
        assert!(row_subtitle.ends_with("· shadowed"), "{row_subtitle}");
    }

    #[test]
    fn diagnostics_row_counts_and_shows_first_truncated() {
        let long = "x".repeat(100);
        let source = packages_source(&[], &["first problem".to_string(), long]);
        let entries = source.entries();
        let row = entries
            .iter()
            .find(|entry| entry.item.id == "packages-diagnostics")
            .expect("diagnostics row");
        assert_eq!(row.item.title, "2 package diagnostics");
        assert_eq!(row.item.subtitle.as_deref(), Some("first problem"));
        assert_eq!(
            row.action,
            PaletteAction::InsertComposerText("/packages list".to_string())
        );

        let single = packages_source(&[], &["only one".to_string()]);
        let single_row = single.entries();
        let single_row = single_row
            .iter()
            .find(|entry| entry.item.id == "packages-diagnostics")
            .expect("diagnostics row");
        assert_eq!(single_row.item.title, "1 package diagnostic");
    }
}
