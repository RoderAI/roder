//! Behavioral tests for the canonical package contracts (phase 93).

use roder_api::packages::{
    PackageExtensionLaunch, PackageResourceFilters, PackageResourceKind, PackageSource,
    derive_package_id, glob_match, package_resource_id, parse_package_resource_id,
    parse_package_spec, validate_package_id,
};

fn parse(spec: &str) -> PackageSource {
    parse_package_spec(spec).unwrap_or_else(|err| panic!("spec {spec:?} should parse: {err}"))
}

#[test]
fn npm_specs_parse_and_round_trip() {
    let cases = [
        ("npm:left-pad", "left-pad", None),
        ("npm:left-pad@1.3.0", "left-pad", Some("1.3.0")),
        ("npm:@scope/pkg", "@scope/pkg", None),
        ("npm:@scope/pkg@2.0.0-rc.1", "@scope/pkg", Some("2.0.0-rc.1")),
    ];
    for (spec, name, version) in cases {
        let source = parse(spec);
        assert_eq!(
            source,
            PackageSource::Npm {
                name: name.to_string(),
                version: version.map(str::to_string),
            },
            "{spec}"
        );
        assert_eq!(source.spec(), spec, "round trip for {spec}");
        assert_eq!(source.pinned(), version.is_some());
    }
}

#[test]
fn git_specs_parse_and_round_trip() {
    // (input, expected url, expected ref, canonical spec)
    let cases = [
        (
            "git:github.com/user/repo",
            "https://github.com/user/repo",
            None,
            "git:https://github.com/user/repo",
        ),
        (
            "git:github.com/user/repo@v1",
            "https://github.com/user/repo",
            Some("v1"),
            "git:https://github.com/user/repo@v1",
        ),
        (
            "git:git@github.com:user/repo",
            "git@github.com:user/repo",
            None,
            "git:git@github.com:user/repo",
        ),
        (
            "git:git@github.com:user/repo@v1.0.0",
            "git@github.com:user/repo",
            Some("v1.0.0"),
            "git:git@github.com:user/repo@v1.0.0",
        ),
        (
            "https://github.com/user/repo",
            "https://github.com/user/repo",
            None,
            "git:https://github.com/user/repo",
        ),
        (
            "https://github.com/user/repo@v2",
            "https://github.com/user/repo",
            Some("v2"),
            "git:https://github.com/user/repo@v2",
        ),
        (
            "ssh://git@github.com/user/repo",
            "ssh://git@github.com/user/repo",
            None,
            "git:ssh://git@github.com/user/repo",
        ),
        (
            "ssh://git@github.com/user/repo@v1",
            "ssh://git@github.com/user/repo",
            Some("v1"),
            "git:ssh://git@github.com/user/repo@v1",
        ),
        (
            "git://host/user/repo",
            "git://host/user/repo",
            None,
            "git:git://host/user/repo",
        ),
        (
            "git:file:///tmp/fixture-repo",
            "file:///tmp/fixture-repo",
            None,
            "git:file:///tmp/fixture-repo",
        ),
        (
            "git:file:///tmp/fixture-repo@main",
            "file:///tmp/fixture-repo",
            Some("main"),
            "git:file:///tmp/fixture-repo@main",
        ),
    ];
    for (input, url, ref_name, canonical) in cases {
        let source = parse(input);
        assert_eq!(
            source,
            PackageSource::Git {
                url: url.to_string(),
                ref_name: ref_name.map(str::to_string),
            },
            "{input}"
        );
        assert_eq!(source.spec(), canonical, "canonical spec for {input}");
        assert_eq!(source.pinned(), ref_name.is_some(), "{input}");
        // The canonical rendering must parse back to the same source.
        assert_eq!(parse(&source.spec()), source, "re-parse {canonical}");
    }
}

#[test]
fn local_path_specs_parse() {
    for spec in ["/abs/path/pkg", "./relative/pkg", "../up/pkg", "~/home/pkg"] {
        let source = parse(spec);
        assert_eq!(
            source,
            PackageSource::LocalPath {
                path: spec.to_string(),
            }
        );
        assert_eq!(source.spec(), spec);
        assert!(!source.pinned());
    }
}

#[test]
fn invalid_specs_fail_with_actionable_errors() {
    let cases = [
        ("", "empty"),
        ("npm:", "missing a package name"),
        ("npm:@scope", "scoped npm names"),
        ("npm:pkg@", "version after @ is empty"),
        ("git:", "missing a repository"),
        ("git:just-a-word", "shorthand"),
        ("not-a-spec", "expected npm:"),
        ("github.com/user/repo", "expected npm:"),
    ];
    for (spec, fragment) in cases {
        let err = parse_package_spec(spec).expect_err(spec).to_string();
        assert!(
            err.contains(fragment),
            "error for {spec:?} should mention {fragment:?}, got: {err}"
        );
    }
}

#[test]
fn identity_is_stable_across_refs_and_versions() {
    assert_eq!(
        parse("npm:@scope/pkg@1.0.0").identity(),
        parse("npm:@scope/pkg").identity()
    );
    assert_eq!(
        parse("git:github.com/User/Repo@v1").identity(),
        parse("https://github.com/user/repo").identity()
    );
    assert_eq!(
        parse("https://github.com/user/repo.git").identity(),
        parse("https://github.com/user/repo").identity()
    );
    assert_ne!(
        parse("npm:@scope/pkg").identity(),
        parse("npm:pkg").identity()
    );
}

#[test]
fn glob_match_supports_segments_and_depth() {
    assert!(glob_match("skills", "skills/changelog/SKILL.md"));
    assert!(glob_match("extensions/*.toml", "extensions/hello.toml"));
    assert!(!glob_match("extensions/*.toml", "extensions/nested/hello.toml"));
    assert!(glob_match("extensions/**/*.toml", "extensions/nested/hello.toml"));
    assert!(glob_match("**/SKILL.md", "skills/a/b/SKILL.md"));
    assert!(glob_match("**", "anything/at/all"));
    assert!(glob_match("themes/*.css", "themes/midnight.css"));
    assert!(!glob_match("themes/*.css", "commands/midnight.css"));
    assert!(glob_match("commands/re*ew.md", "commands/review.md"));
    assert!(!glob_match("commands/re*ew.md", "commands/reviews.md"));
}

#[test]
fn filters_follow_documented_semantics() {
    let mut filters = PackageResourceFilters::default();
    // None loads everything.
    assert!(filters.allows(PackageResourceKind::Skill, "skills/x/SKILL.md"));

    // Some([]) loads nothing.
    filters.skills = Some(Vec::new());
    assert!(!filters.allows(PackageResourceKind::Skill, "skills/x/SKILL.md"));
    // ...except +path force-includes.
    filters.skills = Some(vec!["+skills/x/SKILL.md".to_string()]);
    assert!(filters.allows(PackageResourceKind::Skill, "skills/x/SKILL.md"));
    assert!(!filters.allows(PackageResourceKind::Skill, "skills/y/SKILL.md"));

    // Include globs with ! exclusions; filters narrow, never widen.
    filters.commands = Some(vec![
        "commands/*.md".to_string(),
        "!commands/legacy.md".to_string(),
    ]);
    assert!(filters.allows(PackageResourceKind::Command, "commands/review.md"));
    assert!(!filters.allows(PackageResourceKind::Command, "commands/legacy.md"));
    assert!(!filters.allows(PackageResourceKind::Command, "other/review.md"));

    // -path force-excludes even when an include glob matches.
    filters.themes = Some(vec![
        "themes/*.css".to_string(),
        "-themes/banned.css".to_string(),
    ]);
    assert!(filters.allows(PackageResourceKind::Theme, "themes/ok.css"));
    assert!(!filters.allows(PackageResourceKind::Theme, "themes/banned.css"));

    // Other kinds stay unfiltered.
    assert!(filters.allows(PackageResourceKind::Extension, "extensions/x.toml"));
}

#[test]
fn resource_ids_round_trip() {
    let id = package_resource_id("pr-helper", PackageResourceKind::Skill, "changelog");
    assert_eq!(id, "pr-helper:skill/changelog");
    let (package, kind, name) = parse_package_resource_id(&id).unwrap();
    assert_eq!(package, "pr-helper");
    assert_eq!(kind, PackageResourceKind::Skill);
    assert_eq!(name, "changelog");

    assert!(parse_package_resource_id("missing-separator").is_err());
    assert!(parse_package_resource_id("pkg:badkind/x").is_err());
    assert!(parse_package_resource_id("pkg:skill/").is_err());
}

#[test]
fn package_id_validation_and_derivation() {
    validate_package_id("pr-helper").unwrap();
    validate_package_id("a1.b_c-d").unwrap();
    for bad in ["", "Pr-Helper", "has space", "-leading", "über"] {
        assert!(validate_package_id(bad).is_err(), "{bad:?}");
    }

    assert_eq!(derive_package_id(&parse("npm:@scope/My-Pkg")), "my-pkg");
    assert_eq!(
        derive_package_id(&parse("git:github.com/user/Repo.Name")),
        "repo.name"
    );
    assert_eq!(derive_package_id(&parse("/tmp/some_dir/")), "some_dir");
}

#[test]
fn records_and_launch_serde_round_trip() {
    let record = roder_api::packages::PackageRecord {
        package_id: "pr-helper".to_string(),
        identity: parse("npm:@scope/pkg").identity(),
        source: parse("npm:@scope/pkg@1.0.0"),
        scope: roder_api::packages::PackageScope::User,
        install_path: Some("/home/u/.roder/packages/npm/@scope/pkg".to_string()),
        resolved: Some("1.0.0".to_string()),
        enabled: true,
        allow_scripts: false,
        extensions_approved: false,
        installed_at: time::OffsetDateTime::UNIX_EPOCH,
        content_hash: Some("abc".to_string()),
        filters: PackageResourceFilters::default(),
        disabled_resources: vec![],
    };
    let json = serde_json::to_string(&record).unwrap();
    assert!(json.contains("\"packageId\":\"pr-helper\""), "{json}");
    let back: roder_api::packages::PackageRecord = serde_json::from_str(&json).unwrap();
    assert_eq!(back, record);

    let launch: PackageExtensionLaunch = toml::from_str(
        r#"
        command = "python3"
        args = ["main.py"]
        startup_timeout_ms = 5000
        [env]
        PYTHONUNBUFFERED = "1"
        "#,
    )
    .unwrap();
    assert_eq!(launch.command, "python3");
    assert_eq!(launch.args, ["main.py"]);
    assert_eq!(launch.startup_timeout_ms, Some(5000));
    assert_eq!(launch.env.get("PYTHONUNBUFFERED").unwrap(), "1");
}

#[test]
fn manifest_spec_parses_from_toml_shape() {
    // The shape `roder.toml` uses at a package root (under [package] /
    // [resources] tables it is flattened by the config layer; this checks the
    // canonical struct itself).
    let spec: roder_api::packages::PackageManifestSpec = serde_json::from_value(serde_json::json!({
        "id": "pr-helper",
        "name": "PR Helper",
        "version": "0.1.0",
        "extensions": ["extensions/hello/roder-extension.toml"],
        "skills": ["skills"],
        "commands": ["commands"],
        "themes": ["themes"],
    }))
    .unwrap();
    assert_eq!(spec.id, "pr-helper");
    assert_eq!(spec.extensions.len(), 1);
}
