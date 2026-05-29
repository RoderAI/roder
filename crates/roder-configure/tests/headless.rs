use std::fs;

use roder_configure::headless;

#[test]
fn headless_profile_list_and_show_work() {
    let workspace = workspace();

    let list = headless::run(&args(["profile", "list"]), workspace);
    assert_eq!(list.status, 0);
    assert!(list.stdout.contains("openai-only"));
    assert!(list.stdout.contains("zero-coder-edits"));

    let show = headless::run(&args(["profile", "show", "openai-only"]), workspace);
    assert_eq!(show.status, 0);
    assert!(show.stdout.contains("openai-responses"));

    let show_zero = headless::run(&args(["profile", "show", "zero-coder-edits"]), workspace);
    assert_eq!(show_zero.status, 0);
    assert!(show_zero.stdout.contains("zerolang"));
}

#[test]
fn headless_catalog_list_and_show_work() {
    let workspace = workspace();

    let list = headless::run(&args(["catalog", "list"]), workspace);
    assert_eq!(list.status, 0);
    assert!(list.stdout.contains("openai-responses"));

    let show = headless::run(&args(["catalog", "show", "openai-responses"]), workspace);
    assert_eq!(show.status, 0);
    assert!(show.stdout.contains("OpenAI Responses"));
}

#[test]
fn headless_validate_supports_json_errors() {
    let workspace = workspace();
    let path = temp_profile(
        "bad",
        r#"
id = "bad"
description = "bad profile"

[distribution]
name = "bad-roder"
version = "0.1.0"
include_tui = true
include_app_server = false
include_cli = true
extensions = ["does-not-exist"]
"#,
    );

    let result = headless::run(
        &args(["--format", "json", "validate", path.to_str().unwrap()]),
        workspace,
    );

    assert_eq!(result.status, 1);
    assert!(result.stderr.contains("does-not-exist"));
    let _ = fs::remove_file(path);
}

#[test]
fn headless_generate_writes_distribution() {
    let workspace = workspace();
    let profile = temp_profile(
        "minimal",
        r#"
id = "minimal-test"
description = "minimal profile"

[distribution]
name = "minimal-test-roder"
version = "0.1.0"
include_tui = true
include_app_server = false
include_cli = true
extensions = ["jsonl-thread-store", "plan-mode", "notify-terminal"]
default_thread_store = "jsonl-thread-store"
"#,
    );
    let out = std::env::temp_dir().join(format!("roder-headless-{}", std::process::id()));
    let _ = fs::remove_dir_all(&out);

    let result = headless::run(
        &args([
            "generate",
            "--profile",
            profile.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ]),
        workspace,
    );

    assert_eq!(result.status, 0);
    assert!(out.join("Cargo.toml").exists());
    assert!(out.join("src/main.rs").exists());
    let _ = fs::remove_file(profile);
    let _ = fs::remove_dir_all(out);
}

fn workspace() -> &'static str {
    env!("CARGO_MANIFEST_DIR")
}

fn args<const N: usize>(items: [&str; N]) -> Vec<String> {
    items.iter().map(|item| (*item).to_string()).collect()
}

fn temp_profile(name: &str, contents: &str) -> std::path::PathBuf {
    let path =
        std::env::temp_dir().join(format!("roder-headless-{name}-{}.toml", std::process::id()));
    fs::write(&path, contents).unwrap();
    path
}
