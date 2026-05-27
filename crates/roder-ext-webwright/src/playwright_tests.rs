use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::{
    DependencyCheckMode, WebwrightSetupOptions, preflight_local_dependencies_in_roder_home,
    setup_webwright_runtime_in_roder_home,
};

#[test]
fn setup_dry_run_plans_selected_browser_without_writing_runtime() {
    let roder_home = tempdir("setup-dry-run");

    let report = setup_webwright_runtime_in_roder_home(
        &roder_home,
        WebwrightSetupOptions {
            browser: Some("chromium".to_string()),
            python: Some("/usr/bin/python3".to_string()),
            dry_run: true,
        },
    )
    .unwrap();

    assert_eq!(report.browser, "chromium");
    assert_eq!(report.roder_home, roder_home.display().to_string());
    assert_eq!(
        report.python,
        roder_home
            .join("python/webwright/venv/bin/python")
            .display()
            .to_string()
    );
    assert!(report.dry_run);
    assert!(!report.installed);
    assert!(report.steps.iter().any(|step| step.command
        == vec![
            roder_home
                .join("python/webwright/venv/bin/python")
                .display()
                .to_string(),
            "-m".to_string(),
            "playwright".to_string(),
            "install".to_string(),
            "chromium".to_string(),
        ]));
    assert!(!roder_home.join("python/webwright/setup.json").exists());
}

#[test]
fn setup_installs_with_fake_python_and_preflight_uses_managed_runtime() {
    let roder_home = tempdir("setup-fake-python");
    let log = roder_home.join("fake-python.log");
    let python = roder_home.join("fake-python");
    write_fake_python(&python, &log);

    let report = setup_webwright_runtime_in_roder_home(
        &roder_home,
        WebwrightSetupOptions {
            browser: Some("webkit".to_string()),
            python: Some(python.display().to_string()),
            dry_run: false,
        },
    )
    .unwrap();

    assert_eq!(report.browser, "webkit");
    assert!(report.installed);
    assert!(roder_home.join("python/webwright/setup.json").is_file());
    let log_text = fs::read_to_string(&log).unwrap();
    assert!(log_text.contains("-m venv"));
    assert!(log_text.contains("-m playwright install webkit"));

    let dependency_report = preflight_local_dependencies_in_roder_home(
        DependencyCheckMode::Required,
        &roder_home,
        Some("webkit"),
    )
    .unwrap();
    assert_eq!(dependency_report.python_command, report.python);
    assert!(dependency_report.playwright_available);
}

#[test]
fn setup_rejects_unknown_browser_names() {
    let roder_home = tempdir("setup-unknown-browser");
    let err = setup_webwright_runtime_in_roder_home(
        &roder_home,
        WebwrightSetupOptions {
            browser: Some("lynx".to_string()),
            python: Some("/usr/bin/python3".to_string()),
            dry_run: true,
        },
    )
    .unwrap_err();

    assert!(err.to_string().contains("unsupported Webwright browser"));
}

fn tempdir(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "roder-webwright-{name}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn write_fake_python(path: &Path, log: &Path) {
    let log = log.display().to_string();
    let script = format!(
        r#"#!/bin/sh
echo "$0 $@" >> "{log}"
if [ "$1" = "--version" ]; then
  echo "Python 3.11.9"
  exit 0
fi
if [ "$1" = "-m" ] && [ "$2" = "venv" ]; then
  mkdir -p "$3/bin"
  cat > "$3/bin/python" <<'PY'
#!/bin/sh
echo "$0 $@" >> "{log}"
if [ "$1" = "-c" ]; then
  exit 0
fi
if [ "$1" = "-m" ] && [ "$2" = "pip" ]; then
  exit 0
fi
if [ "$1" = "-m" ] && [ "$2" = "playwright" ] && [ "$3" = "install" ]; then
  exit 0
fi
exit 1
PY
  chmod +x "$3/bin/python"
  exit 0
fi
exit 1
"#
    );
    fs::write(path, script).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}
