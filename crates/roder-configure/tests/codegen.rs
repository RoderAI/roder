use std::fs;

use roder_configure::catalog::Catalog;
use roder_configure::codegen::{emit, render};
use roder_configure::profile::{ProfileExt, built_in_profile};

#[test]
fn codegen_renders_openai_only_distribution_deterministically() {
    let catalog = Catalog::from_workspace(env!("CARGO_MANIFEST_DIR")).unwrap();
    let profile = built_in_profile("openai-only").unwrap().unwrap();

    let first = render(&profile.manifest, &catalog).unwrap();
    let second = render(&profile.manifest, &catalog).unwrap();

    assert_eq!(first, second);
    let cargo_toml = file(&first, "Cargo.toml");
    assert!(cargo_toml.contains("[workspace]"));
    assert!(cargo_toml.contains("roder-ext-openai-responses"));
    assert!(cargo_toml.contains("roder"));
    let readme = file(&first, "README.md");
    assert!(readme.contains("openai-responses"));
    assert!(readme.contains("OPENAI_API_KEY"));
    let config = file(&first, "config.toml");
    toml::from_str::<toml::Value>(&config).unwrap();
    assert!(config.contains("provider = \"openai\""));
    assert!(config.contains("[sessions]\nstore = \"jsonl\""));
    assert!(config.contains("# - OPENAI_API_KEY"));
    assert!(config.contains("[dynamic_workflows]"));
    assert!(config.contains("workspace_workflows_dir = \".agents/workflows\""));
}

#[test]
fn codegen_respects_headless_profile_dependencies() {
    let catalog = Catalog::from_workspace(env!("CARGO_MANIFEST_DIR")).unwrap();
    let profile = built_in_profile("research-headless").unwrap().unwrap();

    let files = render(&profile.manifest, &catalog).unwrap();
    let cargo_toml = file(&files, "Cargo.toml");

    assert!(cargo_toml.contains("roder-app-server"));
    assert!(!cargo_toml.contains("roder-tui"));
}

#[test]
fn codegen_renders_remote_app_server_without_cli_or_tui() {
    let catalog = Catalog::from_workspace(env!("CARGO_MANIFEST_DIR")).unwrap();
    let profile = built_in_profile("remote-app-server").unwrap().unwrap();

    let files = render(&profile.manifest, &catalog).unwrap();
    let cargo_toml = file(&files, "Cargo.toml");
    assert!(cargo_toml.contains("roder-app-server"));
    assert!(cargo_toml.contains("roder-core"));
    assert!(cargo_toml.contains("roder-extension-host"));
    assert!(!cargo_toml.contains("\nroder ="));
    assert!(!cargo_toml.contains("roder-tui"));

    let main_rs = file(&files, "src/main.rs");
    assert!(main_rs.contains("listen_remote_websocket"));
    assert!(!main_rs.contains("roder_cli::run"));
}

#[test]
fn codegen_renders_tavily_enabled_distribution() {
    let catalog = Catalog::from_workspace(env!("CARGO_MANIFEST_DIR")).unwrap();
    let profile = built_in_profile("tavily").unwrap().unwrap();

    let files = render(&profile.manifest, &catalog).unwrap();
    let cargo_toml = file(&files, "Cargo.toml");
    assert!(cargo_toml.contains("roder-ext-tavily-search"));
    assert!(cargo_toml.contains("roder-ext-web-search"));

    let config = file(&files, "config.toml");
    toml::from_str::<toml::Value>(&config).unwrap();
    assert!(config.contains(r#""provider": "tavily""#));
    assert!(config.contains(r#""api_key_env": "TAVILY_API_KEY""#));
    assert!(config.contains("[web_search]"));
    assert!(config.contains("provider = \"tavily\""));

    let readme = file(&files, "README.md");
    assert!(readme.contains("tavily-search"));
    assert!(readme.contains("TAVILY_API_KEY"));
}

#[test]
fn codegen_renders_zero_coder_edits_distribution() {
    let catalog = Catalog::from_workspace(env!("CARGO_MANIFEST_DIR")).unwrap();
    let profile = built_in_profile("zero-coder-edits").unwrap().unwrap();
    profile.validate(&catalog).unwrap();

    let files = render(&profile.manifest, &catalog).unwrap();
    let cargo_toml = file(&files, "Cargo.toml");
    assert!(cargo_toml.contains("roder-ext-zerolang"));
    assert!(cargo_toml.contains("roder-ext-task-process"));
    assert!(cargo_toml.contains("roder-app-server"));
    assert!(!cargo_toml.contains("roder-tui"));

    let config = file(&files, "config.toml");
    toml::from_str::<toml::Value>(&config).unwrap();
    assert!(config.contains("provider = \"openai\""));
    assert!(config.contains("[zerolang]"));
    assert!(config.contains("artifact_dir = \".zero/roder\""));
    assert!(config.contains("[speed_policy]"));
    assert!(config.contains("# - OPENAI_API_KEY"));

    let readme = file(&files, "README.md");
    assert!(readme.contains("zerolang"));
    assert!(readme.contains("OPENAI_API_KEY"));
}

#[test]
fn codegen_emit_writes_expected_files() {
    let catalog = Catalog::from_workspace(env!("CARGO_MANIFEST_DIR")).unwrap();
    let profile = built_in_profile("minimal").unwrap().unwrap();
    profile.validate(&catalog).unwrap();
    let out = std::env::temp_dir().join(format!("roder-codegen-{}", std::process::id()));
    let _ = fs::remove_dir_all(&out);

    let files = emit(&profile.manifest, &catalog, &out).unwrap();

    assert_eq!(files.len(), 4);
    assert!(out.join("Cargo.toml").exists());
    assert!(out.join("src/main.rs").exists());
    assert!(out.join("config.toml").exists());
    assert!(out.join("README.md").exists());
    let _ = fs::remove_dir_all(out);
}

fn file(files: &[roder_configure::codegen::GeneratedFile], path: &str) -> String {
    files
        .iter()
        .find(|file| file.path == std::path::Path::new(path))
        .unwrap()
        .contents
        .clone()
}
