use std::process::{Command, Stdio};

#[test]
fn version_probe_succeeds_without_a_terminal_or_valid_config() {
    let root = std::env::temp_dir().join(format!(
        "roder-version-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let invalid_config = root.join("invalid");
    let missing_config = root.join("missing");
    std::fs::create_dir_all(&invalid_config).unwrap();
    std::fs::write(invalid_config.join("config.toml"), "invalid = [").unwrap();

    for config_dir in [&invalid_config, &missing_config] {
        for flag in ["--version", "-V"] {
            for explicit_config in [false, true] {
                let mut command = Command::new(env!("CARGO_BIN_EXE_roder"));
                if explicit_config {
                    command.arg("--config-dir").arg(config_dir);
                }
                let output = command
                    .arg(flag)
                    .env_clear()
                    .env("HOME", &root)
                    .env("RODER_CONFIG_DIR", config_dir)
                    .env("RODER_DATA_DIR", root.join("data"))
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .output()
                    .expect("run roder version probe");

                assert!(output.status.success(), "{flag}: {output:?}");
                assert_eq!(
                    String::from_utf8_lossy(&output.stdout),
                    format!("roder {}\n", env!("CARGO_PKG_VERSION"))
                );
                assert!(output.stderr.is_empty(), "{flag}: {output:?}");
            }
        }
    }
    assert!(!missing_config.exists());
    assert!(!root.join("data").exists());
    std::fs::remove_dir_all(root).unwrap();
}
